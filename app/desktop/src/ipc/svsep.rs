//! 音轨分离的命令 —— 从 `server/svsep.rs` 搬过来的。
//!
//! 真正的活在 Python 那边的分离后端里（见 `crate::svsep`）。这一层的价值是
//! 前端只认一个后端、一套错误形状，而且「服务没起来」这类话说得比 Python 的
//! 英文堆栈清楚。
//!
//! 用户不管服务的启停：提交时自动起，任务结束、队列空了就自动关
//! （`auto_stop_when_idle`）—— 那服务占着约 5 GB 内存。
//!
//! ## 与旧 HTTP 层的三处差别
//!
//! | 旧路由 | 现在 |
//! |---|---|
//! | `POST /api/svsep/separate`（multipart 上传字节） | `svsep_separate(path, engine)` —— 直接给本机路径 |
//! | `GET /api/svsep/task/{id}/file/{name}` | **asset 协议**（`convertFileSrc(输出目录 + 文件名)`） |
//! | `GET /api/svsep/backend/system-stats` | 并进 `svsep_status`（前端本来就每 2 秒轮一次它） |
//!
//! 上传那条改法的理由值得留一句：Python 服务要的本来就是**一个文件**，传字节只是
//! 因为以前的页面拿不到路径。现在拖放与对话框都给真路径，那一趟白搬的字节没了。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::Cmd;

/* ══════════════════════════════ 全局状态（下载 / 删除） ══════════════════════════════ */

/// 大包下载进度。前端每 2 秒轮询一次 `svsep_status` 就能看到它动。
///
/// 放全局静态是因为「同一时刻只可能有一个下载」—— 用户能同时点两次，
/// 但第二次会被 `DL_ACTIVE` 挡掉。运行时（几 GB）与模型（730 MB）共用这一份
/// 状态，`DL_KIND` 说明现在下的是哪一个，界面按它显示对应的按钮。
static DL_BYTES: AtomicU64 = AtomicU64::new(0);
static DL_TOTAL: AtomicU64 = AtomicU64::new(0);
static DL_ACTIVE: AtomicU64 = AtomicU64::new(0);
static DL_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static DL_KIND: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);
/// 用户按了「暂停」：下载循环下一块就收手，**`.part` 留着**（下次带 Range 接着下）
static DL_PAUSE: AtomicBool = AtomicBool::new(false);
/// 用户按了「停止」：收手并且**删掉 `.part`**（下次从头下）
static DL_STOP: AtomicBool = AtomicBool::new(false);
/// 现在跑到哪一段了：0 = 在下字节，1 = 下完了、正在解压（`crate::svsep::Stage`）。
///
/// 为什么要让界面知道：解压的分母（所有条目压缩后大小之和）跟 zip 的字节数几乎
/// 一样大，界面上不分段看着就像「下到 100% 又归零、在同一个『正在下载…』标签下
/// 重下一遍」—— 用户 2026-10-03 报的就是这一幕（实际在解压）。
static DL_STAGE: AtomicU64 = AtomicU64::new(0);

/// 跑完的任务留一份快照（最近 `KEEP_TASKS` 个，新的在前）。
///
/// **为什么需要**：任务一结束我们就自动把分离服务关了（`auto_stop_when_idle`），
/// 而界面是靠「再轮询一次拿到 `status=done`」才知道该显示那几轨的 —— 轮询间隔
/// 2 秒、关服务在 1.5 秒后，正好错开的话那道 `done` 就永远问不到了：服务已经没了，
/// 界面卡在 90% 还会每 2 秒弹一次「分离服务还没启动」。
/// 终态一到就抄一份在这儿，服务停了也照样答得上（文件本身走 asset 协议，读盘）。
static DONE_TASKS: std::sync::Mutex<Vec<(String, Value)>> = std::sync::Mutex::new(Vec::new());
/// 留着几条 —— 够用户回看最近几次，也不会把内存当缓存使。
const KEEP_TASKS: usize = 8;

/// 「一键删除依赖」在跑吗
static DEL_ACTIVE: AtomicBool = AtomicBool::new(false);
static DEL_BYTES: AtomicU64 = AtomicU64::new(0);
static DEL_FILES: AtomicU64 = AtomicU64::new(0);

/* ══════════════════════════════ 进度与状态 ══════════════════════════════ */

/// 下载循环每收一块就调它。
fn note_progress(got: u64, total: Option<u64>, stage: crate::svsep::Stage) {
    DL_BYTES.store(got, Ordering::Relaxed);
    if let Some(t) = total {
        DL_TOTAL.store(t, Ordering::Relaxed);
    }
    DL_STAGE.store(
        if stage == crate::svsep::Stage::Extract { 1 } else { 0 },
        Ordering::Relaxed,
    );
}

/// 新建下载任务时要挂上去的续传链接（`DownloadCtl::resume_url`）。
///
/// 续传必须拿**暂停时那一条链接**发 Range，不能现查配置：用户在暂停期间改了
/// `MODEL_URL` 的话，接着下的会是另一个包的文件，拼出来的 zip 要到解压时才炸。
///
/// ⚠️ **这个记忆只在内存里，重启就没了** —— 而盘上那半个包还在。所以「能不能
/// 接着下」的判据不看它，看盘（`svsep.rs::resume_point`，旁边那个 `.part.url`
/// 记号才是持久的出处）；这里只是把盘上的结论翻成 `DownloadCtl` 要的形状。
fn resume_for(
    kind: &str,
    root: &std::path::Path,
    writable: &std::path::Path,
    url: &str,
) -> Option<String> {
    crate::svsep::resume_point(root, writable, kind, url).map(|_| url.to_string())
}

fn download_state(root: &std::path::Path, writable: &std::path::Path) -> Value {
    let active = DL_ACTIVE.load(Ordering::Relaxed) == 1;
    let err = DL_ERROR.lock().ok().and_then(|e| e.clone());
    let kind = DL_KIND.lock().ok().and_then(|k| *k);
    /* 有 `.part` 就说明「下过一半、可以接着下」。界面靠它把按钮文案从
       「下载模型」改成「继续下载模型」。
       ⚠️ **判据是盘上的半个包，不是内存里的记号**。第一版拿 `DL_KIND` 去对：
       那是「此刻在下的包」，下载任务一收场就被清成 `None`，而「能不能续传」问的
       恰恰是**收场之后**的事 —— 于是暂停后 `resumable` 恒为 false，界面永远不
       显示「继续下载」。第二版改成内存里的记号，暂停当下对了，但**工作站一重启
       记号就没了**，盘上 4.7 GB 的半个包界面看不见，用户一点就从头下。
       现在按盘上查，两边都对。 */
    let paused = ["runtime", "models"].iter().find_map(|k| {
        let url = if *k == "runtime" {
            crate::svsep::runtime_url()
        } else {
            crate::svsep::model_url()
        };
        crate::svsep::resume_point(root, writable, k, &url).map(|n| (*k, n))
    });
    let (paused_kind, paused_bytes) = match paused {
        Some((k, n)) => (Some(k), n),
        None => (None, 0),
    };
    let del_active = DEL_ACTIVE.load(Ordering::Relaxed);
    json!({
        "active": active,
        // "runtime" / "models"；没有下载时是 null
        "kind": kind,
        // "download" / "extract"：界面据此把标签换成「正在解压…」，
        // 并在解压期间藏起暂停/停止（那两个按钮对解压无效）
        "stage": if DL_STAGE.load(Ordering::Relaxed) == 1 { "extract" } else { "download" },
        "done": DL_BYTES.load(Ordering::Relaxed) as f64,
        "total": DL_TOTAL.load(Ordering::Relaxed) as f64,
        "error": err,
        // 上次暂停留下的进度：`resumable` 为真时 `done` 就是已下字节数
        "resumable": paused_kind.is_some() && !active,
        // 暂停的是哪个包 + 已经下到哪（界面拿它决定哪一行按钮写「继续下载」）
        "pausedKind": paused_kind,
        "pausedBytes": paused_bytes as f64,
        "delete": {
            "active": del_active,
            "files": DEL_FILES.load(Ordering::Relaxed) as f64,
            "bytes": DEL_BYTES.load(Ordering::Relaxed) as f64,
        },
    })
}

/// 起一个下载任务，立刻返回。两个下载命令共用。
///
/// 下载要跑几分钟到几小时（运行时几 GB），**不能占着调用** —— 回一句
/// 「开始了」，进度由前端轮询 `svsep_status` 的 `download` 拿。
fn spawn_download<F>(kind: &'static str, job: F) -> Result<Value, String>
where
    F: std::future::Future<Output = Result<crate::svsep::FetchOutcome, String>> + Send + 'static,
{
    if DL_ACTIVE.load(Ordering::Relaxed) == 1 {
        let now = DL_KIND.lock().ok().and_then(|k| *k).unwrap_or("包");
        let now = if now == "runtime" { "运行时" } else { "模型" };
        return Err(format!("{now}正在下载中。想换一个就先暂停或停止它。"));
    }
    if DEL_ACTIVE.load(Ordering::Relaxed) {
        return Err("正在删除依赖文件，等它删完再下（删到一半开始下会互相拆台）。".into());
    }
    if let Ok(mut e) = DL_ERROR.lock() {
        *e = None;
    }
    if let Ok(mut k) = DL_KIND.lock() {
        *k = Some(kind);
    }
    DL_PAUSE.store(false, Ordering::Relaxed);
    DL_STOP.store(false, Ordering::Relaxed);
    DL_STAGE.store(0, Ordering::Relaxed);
    DL_ACTIVE.store(1, Ordering::Relaxed);

    tokio::spawn(async move {
        let res = job.await;
        DL_ACTIVE.store(0, Ordering::Relaxed);
        if let Ok(mut k) = DL_KIND.lock() {
            *k = None;
        }
        if let Err(e) = res {
            /* 出错 = 那个 `.part` 不可信，别留着让下次去续。
               ⚠️ 这里只清内存里的记号是**不够**的（判据已经改成看盘了），
               真正删 `.part` 的是 `fetch_bundle`：它把「打不开 / 不是 206」
               这些情况都归到「从 0 开始」，那条分支会把 `.part` 和
               `.part.url` 一起删掉。 */
            if let Ok(mut slot) = DL_ERROR.lock() {
                *slot = Some(e);
            }
        }
    });

    Ok(json!({ "started": true }))
}

/* ══════════════════════════════ 状态 / 起停 ══════════════════════════════ */

/// 分离服务 / 运行时 / 模型 / 下载的整体状态（前端每 2 秒轮它）。
#[tauri::command]
pub async fn svsep_status(st: super::St<'_>) -> Cmd {
    let s = &st.inner().svsep;
    let running = s.probe().await;
    Ok(json!({
        "runtimeReady": s.runtime_ready(),
        "dir": s.dir().to_string_lossy(),
        "modelsDir": s.models().to_string_lossy(),
        "dataDir": s.data().to_string_lossy(),
        "outputsDir": s.outputs().to_string_lossy(),
        "runtime": crate::svsep::runtime_status(&st.inner().root),
        "models": crate::svsep::models_status(s.writable()),
        "download": download_state(&st.inner().root, s.writable()),
        "running": running,
        "port": s.port_hint(),
        "baseUrl": s.base_url(),
        "lastError": s.last_error(),
    }))
}

/// 起分离服务。已经起着就原样回（`started: false`）。
#[tauri::command]
pub async fn svsep_start(st: super::St<'_>) -> Cmd {
    let (port, started) = st.inner().svsep.start().await.map_err(|e| e.to_string())?;
    let status = st.inner().svsep.get("/api/status").await.unwrap_or(json!({}));
    Ok(json!({
        "running": true,
        "started": started,
        "port": port,
        "baseUrl": st.inner().svsep.base_url(),
        "backend": status,
    }))
}

/// 停分离服务。幂等 —— 重复调用无害。
#[tauri::command]
pub async fn svsep_stop(st: super::St<'_>) -> Cmd {
    st.inner().svsep.stop();
    Ok(json!({ "running": false }))
}

/* ══════════════════════════════ 下载 ══════════════════════════════ */

/// 下模型（压缩包 462 MB，解压后 730 MB）。
///
/// 上次是**暂停**在这里的（同一个包、同一条链接）就带着 `Range` 接着下；
/// 换了包、或者上次是出错/停止结束的，就从头下（`fetch_bundle` 会把无效的
/// `.part` 删掉）。
#[tauri::command]
pub async fn svsep_models_download(st: super::St<'_>) -> Cmd {
    let writable = st.inner().svsep.writable().to_path_buf();
    let root = st.inner().root.clone();
    let url = crate::svsep::model_url();
    let resume = resume_for("models", &root, &writable, &url);
    spawn_download("models", async move {
        let ctl = crate::svsep::DownloadCtl::new(&DL_PAUSE, &DL_STOP, resume);
        let out = crate::svsep::download_models(&writable, &url, &ctl, note_progress).await;
        // 暂停了就留着记号（下次接着下要用）；下完 / 停止 / 出错都不用留。
        // ⚠️ 「下完」到底是哪一种要现查 —— `ctl.paused()` 只有暂停为真，但它
        //    分不出 Done 与 Cancelled，所以这里再问一次盘上的 `.part` 还在不在。
        if crate::svsep::resume_point(&root, &writable, "models", &url).is_none() {
            crate::svsep::clear_resume_marker(&root, &writable, "models");
        }
        out
    })
}

/// 下运行时（几 GB，只该下一次）。
///
/// ⚠️ 它解到 `<root>/app/data/svsep/`（**程序目录**，不是 `%APPDATA%`）——
/// 因为 `python.exe` 与 `backend/` 必须待在一起，而上游后端就是按
/// 「runtime 与 backend 同级」找东西的。安装版下 `Program Files` 不可写，
/// 那时这个下载会以「建目录失败」失败，错误文案照实说。
#[tauri::command]
pub async fn svsep_runtime_download(st: super::St<'_>) -> Cmd {
    let root = st.inner().root.clone();
    let writable = st.inner().svsep.writable().to_path_buf();
    let url = crate::svsep::runtime_url();
    let resume = resume_for("runtime", &root, &writable, &url);
    let root2 = root.clone();
    let writable2 = writable.clone();
    spawn_download("runtime", async move {
        let ctl = crate::svsep::DownloadCtl::new(&DL_PAUSE, &DL_STOP, resume);
        let out = crate::svsep::download_runtime(&root2, &url, &ctl, note_progress).await;
        if crate::svsep::resume_point(&root2, &writable2, "runtime", &url).is_none() {
            crate::svsep::clear_resume_marker(&root2, &writable2, "runtime");
        }
        out
    })
}

/// 暂停下载：`.part` 留着，下次点「继续下载」带 Range 接着下。
#[tauri::command]
pub async fn svsep_download_pause() -> Cmd {
    if DL_ACTIVE.load(Ordering::Relaxed) != 1 {
        return Err("现在没有在下载".into());
    }
    DL_PAUSE.store(true, Ordering::Relaxed);
    Ok(json!({ "pausing": true }))
}

/// 停止下载：`.part` 也删掉，下次从头下。
#[tauri::command]
pub async fn svsep_download_stop() -> Cmd {
    if DL_ACTIVE.load(Ordering::Relaxed) != 1 {
        return Err("现在没有在下载".into());
    }
    DL_STOP.store(true, Ordering::Relaxed);
    Ok(json!({ "stopping": true }))
}

/// 一键删掉下下来的模型与运行时（**下完的、没下完的都删**）。
///
/// ⚠️ 删运行时等于「下次要重新下 4.7 GB」，所以前端必须让用户确认过。
/// ⚠️ 不删 `backend/`：那几个 .py 随程序打包，不属于「依赖」，删了就得重装。
#[tauri::command]
pub async fn svsep_deps_delete(st: super::St<'_>) -> Cmd {
    if DL_ACTIVE.load(Ordering::Relaxed) == 1 {
        return Err("正在下载，先暂停或停止再删（边下边删只会留下一堆半截文件）。".into());
    }
    if DEL_ACTIVE.swap(true, Ordering::Relaxed) {
        return Err("正在删除中，等它删完".into());
    }
    if st.inner().svsep.probe().await {
        // 引擎正跑着就删运行时 = 删正在运行的 python.exe（必然一批文件删不掉）。
        // 先停服务；真停不掉也不硬来，删不掉的会照实报给用户。
        st.inner().svsep.stop();
    }

    let root = st.inner().root.clone();
    let writable = st.inner().svsep.writable().to_path_buf();
    DEL_BYTES.store(0, Ordering::Relaxed);
    DEL_FILES.store(0, Ordering::Relaxed);

    tokio::spawn(async move {
        // 几万个文件，纯阻塞 IO，丢给阻塞线程池；`DL_STOP` 也当成「别删了」的开关
        // （用户这时能按的唯一一个停止按钮就是它）。
        let res = tokio::task::spawn_blocking(move || {
            crate::svsep::delete_dependencies(
                &root,
                &writable,
                || DL_STOP.load(Ordering::Relaxed),
                |files, bytes| {
                    DEL_FILES.store(files, Ordering::Relaxed);
                    DEL_BYTES.store(bytes, Ordering::Relaxed);
                },
            )
        })
        .await;
        DEL_ACTIVE.store(false, Ordering::Relaxed);
        match res {
            Ok(v) => {
                if let Ok(mut slot) = DL_ERROR.lock() {
                    *slot = None;
                }
                // 删除结果也放这儿让前端弹一句（真正的落盘状态下次轮询 status 就有了）
                if let Some(msg) = v.get("removedFiles") {
                    crate::log_line(&format!(
                        "音轨分离：已删除依赖文件 {} 个 / {} 字节",
                        msg,
                        v.get("removedBytes").and_then(|b| b.as_u64()).unwrap_or(0)
                    ));
                }
            }
            Err(e) => {
                if let Ok(mut slot) = DL_ERROR.lock() {
                    *slot = Some(format!("删除依赖失败：{e}"));
                }
            }
        }
    });

    Ok(json!({ "started": true }))
}

/* ══════════════════════════════ 分离任务 ══════════════════════════════ */

/// 提交一次分离 —— **收本机路径，不收字节**。
///
/// 这是这次减法里最典型的一处：旧实现是「前端把音频读成 multipart 传到后端，
/// 后端把字节原样转发给 Python 服务」。而 Python 服务要的本来就是**一个文件**，
/// 传字节只是因为我们以前只有字节。
///
/// `engine`：`"uvr"`（二轨：人声 / 伴奏）或 `"roformer"`（六轨）。默认 roformer。
///
/// 内部照旧：服务没起就先起（用户点「开始分离」时它通常还没起来）。
#[tauri::command]
pub async fn svsep_separate(st: super::St<'_>, path: String, engine: Option<String>) -> Cmd {
    let engine = engine.unwrap_or_else(|| "roformer".into());
    let p = std::path::Path::new(&path);
    if !p.is_file() {
        return Err(format!("音频文件不存在：{path}"));
    }
    let bytes = tokio::fs::read(p)
        .await
        .map_err(|e| format!("读音频失败：{e}"))?;
    if bytes.is_empty() {
        return Err("音频文件是空的".into());
    }
    // Content-Type 仍然要算出来给上游：Python 那边的 multipart 解析按它判类型。
    // 按扩展名映射就够（上游只认 audio 那几种）。
    let ct = match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .as_deref()
    {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("ogg") | Some("opus") => "audio/ogg",
        Some("wma") => "audio/x-ms-wma",
        _ => "application/octet-stream",
    };

    if !st.inner().svsep.probe().await {
        // 顺手把它起起来 —— 用户点「开始分离」时服务通常还没起
        st.inner().svsep.start().await.map_err(|e| e.to_string())?;
    }

    let mut out = st
        .inner()
        .svsep
        .submit(&engine, bytes, ct)
        .await
        .map_err(|e| e.to_string())?;
    // 任务对象捋平：前端拿 `res.task` 直接当任务记录用，而它读的是 `task.id` ——
    // 上游发的是 `task_id`、还包在 `task` 里。
    if let Some(o) = out.as_object_mut() {
        if o.contains_key("task") {
            let t = flat_task(o.get("task").cloned().unwrap_or(Value::Null));
            o.insert("task".to_string(), t);
        }
    }
    Ok(out)
}

/// 查一个分离任务。
#[tauri::command]
pub async fn svsep_task(st: super::St<'_>, id: String) -> Cmd {
    /* 服务已经自动关了（任务跑完就关，见 `auto_stop_when_idle`）：这时 `get()` 只会
       回「分离服务还没启动」，可界面要的恰恰是最后那道 `done` —— 先看快照。 */
    if !st.inner().svsep.probe().await {
        if let Some(t) = recall_task(&id) {
            return Ok(t);
        }
    }
    let v = st
        .inner()
        .svsep
        .get(&format!("/api/status/{id}"))
        .await
        .map_err(|e| e.to_string())?;
    let t = flat_task(v);
    /* 任务结束了就把服务关掉 —— 用户不用记着点「停止服务」，也不用白占 5 GB 内存。
       隔一会儿、并且确认队列空了才真关（见 `auto_stop_when_idle`）。 */
    if matches!(
        t.get("status").and_then(Value::as_str),
        Some("done") | Some("failed") | Some("cancelled")
    ) {
        remember_task(&id, &t);
        tokio::spawn(auto_stop_when_idle(st.inner().clone()));
    }
    Ok(t)
}

/// 取消一个分离任务。
#[tauri::command]
pub async fn svsep_cancel(st: super::St<'_>, id: String) -> Cmd {
    st.inner()
        .svsep
        .post_json(&format!("/api/cancel/{id}"), &json!({}))
        .await
        .map_err(|e| e.to_string())
}

/// 打开输出目录（或在资源管理器里定位某个产物）。
#[tauri::command]
pub async fn svsep_open_output(st: super::St<'_>, args: Option<Value>) -> Cmd {
    let body = args.unwrap_or_else(|| json!({}));
    st.inner()
        .svsep
        .post_json("/api/open-output", &body)
        .await
        .map_err(|e| e.to_string())
}

/// 分离服务的原始状态（`/api/status`）—— 页面上「设备 / 队列 / 输出目录」那一条用。
#[tauri::command]
pub async fn svsep_backend_status(st: super::St<'_>) -> Cmd {
    st.inner()
        .svsep
        .get("/api/status")
        .await
        .map_err(|e| e.to_string())
}

/* ══════════════════════════════ 推理方式 ══════════════════════════════ */

/// 推理方式：自动 / GPU / CPU。
///
/// 服务在跑就问它（它会顺手探一下硬件，给出 `badge` / `hardware`）；**服务没跑
/// 就读盘**上的 `<数据目录>/inference_settings.json` —— 任务一结束服务就自动关
/// 了（`auto_stop_when_idle`），可这个设置项在界面上得一直看得见、改得动。
#[tauri::command]
pub async fn svsep_inference_get(st: super::St<'_>) -> Cmd {
    if st.inner().svsep.probe().await {
        if let Ok(v) = st.inner().svsep.get("/api/inference-settings").await {
            return Ok(v);
        }
    }
    let mode = read_infer_mode(&st.inner().svsep);
    Ok(infer_reply(&mode, true))
}

/// 设置推理方式。`{ mode: "auto" | "cpu" | "gpu" }`
#[tauri::command]
pub async fn svsep_set_inference(st: super::St<'_>, mode: String) -> Cmd {
    if !["auto", "cpu", "gpu"].contains(&mode.trim().to_ascii_lowercase().as_str()) {
        return Err("无效模式，请选择 auto / cpu / gpu".into());
    }
    let mode = normalize_mode(&mode);
    write_infer_mode(&st.inner().svsep, &mode)?;
    /* 服务在跑就再告诉它一声：它把 mode 缓存在进程内存里（`inference_settings.py`
       的 `_cached_mode`），光改文件它不认。它没起来、或者答错了都不影响结果 ——
       下次启动读的就是这个文件。 */
    if st.inner().svsep.probe().await {
        if let Ok(v) = st
            .inner()
            .svsep
            .post_json("/api/inference-settings", &json!({ "mode": mode }))
            .await
        {
            return Ok(v);
        }
    }
    Ok(infer_reply(&mode, true))
}

/* ══════════════════════════════ 小工具 ══════════════════════════════ */

/// 服务已经不在了（任务跑完自动关的）：把终态快照掏出来答。
///
/// 放在 `svsep_task` 的最前面 —— 没服务的时候 `get()` 只会回「分离服务还没启动」，
/// 而这时候界面要的恰恰是最后那道 `done` 与那几轨的名字。
fn recall_task(id: &str) -> Option<Value> {
    DONE_TASKS
        .lock()
        .ok()
        .and_then(|all| all.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()))
}

fn remember_task(id: &str, t: &Value) {
    let Ok(mut all) = DONE_TASKS.lock() else {
        return;
    };
    all.retain(|(k, _)| k != id);
    all.insert(0, (id.to_string(), t.clone()));
    all.truncate(KEEP_TASKS);
}

/// 把上游的任务对象捋平。
///
/// 上游 `GET /api/status/<id>` 回的是 `{"ok":true,"task":{…}}`，而且里面的任务
/// id 叫 `task_id`；前端按**扁平 + `id`** 读。2026-10-03 之前这里是原样透传的 ——
/// 前端第一轮轮询拿到的是外壳对象，`task.id` 变 undefined，任务轮询自己停掉：
/// 界面永远停在提交那一刻（用户报的「分离完了进度还卡在 5%」就是这个）。
fn flat_task(v: Value) -> Value {
    let mut t = if v.get("task").map(Value::is_object).unwrap_or(false) {
        v.get("task").cloned().unwrap_or(Value::Null)
    } else {
        v
    };
    if let Some(o) = t.as_object_mut() {
        if let Some(id) = o.get("task_id").cloned() {
            o.entry("id").or_insert(id);
        }
    }
    t
}

/// 后端还有活没干完吗（排队中或正在算）。
///
/// 判据是上游 `/api/status` 里的 `queues.{uvr,roformer}.{waiting,processing}`。
/// **读不懂就当成「有活」** —— 宁可不关服务，也不能把用户刚排上的任务连锅端。
fn queues_busy(status: &Value) -> bool {
    let Some(q) = status.get("queues").and_then(Value::as_object) else {
        return true;
    };
    q.values().any(|e| {
        ["waiting", "processing"]
            .iter()
            .any(|k| e.get(*k).and_then(Value::as_u64).unwrap_or(0) > 0)
    })
}

/// 任务结束了 → 队列也空了 → 把分离服务关掉。
///
/// 隔 1.5 秒再看一眼：用户可能正好在这时又提交了一个文件，那个任务还在排队
/// （`queues_busy` 会拦住）。`stop()` 是幂等的，重复调用无害。
async fn auto_stop_when_idle(st: Arc<super::AppState>) {
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let Ok(v) = st.svsep.get("/api/status").await else {
        return;
    };
    if queues_busy(&v) {
        return;
    }
    crate::log_line("音轨分离：任务结束，自动关掉分离服务");
    st.svsep.stop();
}

/// 推理方式的设置文件 —— 上游存的就是 `<数据目录>/inference_settings.json`
/// （`inference_settings.py::_SETTINGS_PATH`）。
fn inference_file(svsep: &crate::svsep::Svsep) -> std::path::PathBuf {
    svsep.data().join("inference_settings.json")
}

/// 读盘上的推理方式（没有文件、文件坏了都算 `auto`，跟上游一致）。
fn read_infer_mode(svsep: &crate::svsep::Svsep) -> String {
    std::fs::read_to_string(inference_file(svsep))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("mode").and_then(Value::as_str).map(str::to_string))
        .map(|m| normalize_mode(&m))
        .unwrap_or_else(|| "auto".to_string())
}

/// 认不出的一律回 `auto`（上游的 `VALID_MODES` 也只有这三个）。
fn normalize_mode(m: &str) -> String {
    let m = m.trim().to_ascii_lowercase();
    if ["auto", "cpu", "gpu"].contains(&m.as_str()) {
        m
    } else {
        "auto".to_string()
    }
}

/// 写盘上的推理方式。格式跟上游 `set_mode()` 一样（`{"mode": …}`、两空格缩进），
/// 这样服务和界面谁先谁后写都不会打架。
fn write_infer_mode(svsep: &crate::svsep::Svsep, mode: &str) -> Result<(), String> {
    let path = inference_file(svsep);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{e}"))?;
    }
    let body = serde_json::to_string_pretty(&json!({ "mode": mode })).unwrap_or_default();
    std::fs::write(&path, body).map_err(|e| format!("写推理设置失败：{e}"))
}

/// 服务没跑时给界面的回包 —— 键跟上游 `public_settings()` 对得上，`offline`
/// 让界面知道这不是现探的硬件。
fn infer_reply(mode: &str, offline: bool) -> Value {
    json!({
        "mode": mode,
        "effective_mode": mode,
        "badge": infer_badge(mode),
        "detail": if offline { "分离服务没在跑，这是盘上的设置；开始分离时按它来。" } else { "" },
        "offline": offline,
    })
}

fn infer_badge(mode: &str) -> &'static str {
    match mode {
        "cpu" => "CPU（下次分离生效）",
        "gpu" => "GPU（下次分离生效）",
        _ => "自动（下次分离生效）",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_envelope_is_flattened_and_task_id_becomes_id() {
        let t = flat_task(json!({
            "ok": true,
            "task": { "task_id": "abc", "status": "processing", "progress": 42 },
        }));
        assert_eq!(t.get("id").and_then(Value::as_str), Some("abc"));
        assert_eq!(t.get("progress").and_then(Value::as_u64), Some(42));
        assert!(t.get("task").is_none() || t.get("task").map(Value::is_object) != Some(true));

        // 上游哪天改成扁平的了也不能坏
        let flat = flat_task(json!({ "task_id": "x", "status": "done" }));
        assert_eq!(flat.get("id").and_then(Value::as_str), Some("x"));
        assert_eq!(flat.get("status").and_then(Value::as_str), Some("done"));
    }

    #[test]
    fn an_empty_queue_is_idle_but_an_unreadable_one_is_busy() {
        let idle = json!({ "queues": {
            "uvr": { "waiting": 0, "processing": 0 },
            "roformer": { "waiting": 0, "processing": 0 },
        }});
        assert!(!queues_busy(&idle));

        let queued = json!({ "queues": { "roformer": { "waiting": 1, "processing": 0 } }});
        assert!(queues_busy(&queued));
        let running = json!({ "queues": { "uvr": { "waiting": 0, "processing": 1 } }});
        assert!(queues_busy(&running));

        // 读不懂就当有活 —— 不能因为看不懂就把服务关了
        assert!(queues_busy(&json!({})));
        assert!(queues_busy(&json!({ "queues": "?" })));
    }

    #[test]
    fn inference_mode_falls_back_to_auto() {
        assert_eq!(normalize_mode("GPU"), "gpu");
        assert_eq!(normalize_mode(" cpu "), "cpu");
        assert_eq!(normalize_mode("cuda"), "auto");
        assert_eq!(normalize_mode(""), "auto");
        assert!(infer_badge("gpu").starts_with("GPU"));
    }

    #[test]
    fn a_finished_task_stays_answerable_after_the_service_is_gone() {
        // 服务停掉以后界面还得靠这道 `done` 才显示那几轨，所以终态要能再掏出来
        let done = json!({ "id": "t1", "status": "done", "outputs": [{ "filename": "a.wav" }] });
        remember_task("t1", &done);
        assert_eq!(
            recall_task("t1").and_then(|v| v.get("status").cloned()),
            Some(json!("done"))
        );
        assert!(recall_task("nope").is_none());

        // 同 id 再记一次只留一份（不然轮询几次就攒一摞）
        remember_task("t1", &done);
        assert!(DONE_TASKS.lock().unwrap().iter().filter(|(k, _)| k == "t1").count() == 1);

        // 只留最近 KEEP_TASKS 个，新的在前
        for i in 0..KEEP_TASKS + 3 {
            remember_task(&format!("old{i}"), &json!({ "status": "done" }));
        }
        let all = DONE_TASKS.lock().unwrap();
        assert!(all.len() <= KEEP_TASKS);
        assert!(all.iter().any(|(k, _)| k == "old10"));
        assert!(!all.iter().any(|(k, _)| k == "t1"));
    }
}
