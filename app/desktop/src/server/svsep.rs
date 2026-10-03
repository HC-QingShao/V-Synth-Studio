//! 音轨分离 —— 在线（MVSEP）与离线（内嵌分离引擎）两条路
//!
//! 这一组路由**只是转发**：真正的活在 Python 那边的分离后端里
//! （见 `crate::svsep`）。转发的收益是前端只认一个后端、一套 CORS 与错误形状，
//! 而且「服务没起来」这类话说得比 Python 的英文堆栈清楚。
//!
//! 路由一览：
//!   GET  /api/svsep/status              运行时/模型/服务状态（前端轮询它）
//!   POST /api/svsep/start               起分离服务
//!   POST /api/svsep/stop                停分离服务
//!   POST /api/svsep/runtime/download    下运行时 zip（几 GB）并解压
//!   POST /api/svsep/models/download     下模型 zip 并解压
//!   POST /api/svsep/download/pause      暂停下载（留 .part，下次接着下）
//!   POST /api/svsep/download/stop       停止下载（删 .part，下次从头下）
//!   POST /api/svsep/deps/delete         一键删掉下下来的模型与运行时
//!   POST /api/svsep/separate            提交一次分离（multipart 原样转发）
//!   GET  /api/svsep/task/{id}           查任务（查到任务结束就把服务关掉）
//!   GET  /api/svsep/task/{id}/file/{n}  取输出文件（**读磁盘**，服务关着也能听）
//!   GET  /api/svsep/backend/inference   读/写推理方式（自动 / GPU / CPU）
//!
//! 用户不管服务的启停：提交时自动起（前端与 `separate` 各做一半），任务结束、
//! 队列空了就自动关（`auto_stop_when_idle`）—— 那服务占着约 5 GB 内存。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};

use super::{ok, ApiError, AppState};

/// 提交分离时允许的最大 body（600 MB）。
///
/// 上游后端自己的上限是 100 MB（`config.MAX_CONTENT_LENGTH`），这里放宽只是为了
/// 不让**工作站**先把它拦下来 —— 真正的判据在 Python 那边，超了它会回
/// 一句人能看懂的「不支持的文件类型 / 太大」。两层限制写不一样是有意的：
/// axum 这层超了只会回一个干巴巴的 413。
pub const SEPARATE_LIMIT: usize = 600 * 1024 * 1024;

/// 大包下载进度。前端每 2 秒轮询一次 `/api/svsep/status` 就能看到它动。
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
/// 终态一到就抄一份在这儿，服务停了也照样答得上（文件本身走 `output`，读盘）。
static DONE_TASKS: std::sync::Mutex<Vec<(String, Value)>> = std::sync::Mutex::new(Vec::new());
/// 留着几条 —— 够用户回看最近几次，也不会把内存当缓存使。
const KEEP_TASKS: usize = 8;
/// 「一键删除依赖」在跑吗
static DEL_ACTIVE: AtomicBool = AtomicBool::new(false);
static DEL_BYTES: AtomicU64 = AtomicU64::new(0);
static DEL_FILES: AtomicU64 = AtomicU64::new(0);

/// 新建下载任务时要挂上去的续传链接（`DownloadCtl::resume_url`）。
///
/// 续传必须拿**暂停时那一条链接**发 Range，不能现查配置：用户在暂停期间改了
/// `MODEL_URL` 的话，接着下的会是另一个包的文件，拼出来的 zip 要到解压时才炸。
///
/// ⚠️ **这个记忆只在内存里，重启就没了** —— 而盘上那半个包还在。所以「能不能
/// 接着下」的判据不看它，看盘（`svsep.rs::resume_point`，旁边那个 `.part.url`
/// 记号才是持久的出处）；`resume_for` 只是把盘上的结论翻成 `DownloadCtl` 要的形状。
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
       显示「继续下载」。第二版改成内存里的 `DL_RESUME`，暂停当下对了，但
       **工作站一重启记号就没了**，盘上 4.7 GB 的半个包界面看不见，用户一点就
       从头下。现在按盘上查，两边都对。 */
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

/// 起一个下载任务，立刻返回。两个下载端点共用。
///
/// 下载要跑几分钟到几小时（运行时几 GB），**不能占着请求** —— 回一句
/// 「开始了」，进度由前端轮询 `/api/svsep/status` 的 `download` 拿。
fn spawn_download<F>(kind: &'static str, job: F) -> Result<Json<Value>, ApiError>
where
    F: std::future::Future<Output = Result<crate::svsep::FetchOutcome, String>> + Send + 'static,
{
    if DL_ACTIVE.load(Ordering::Relaxed) == 1 {
        let now = DL_KIND.lock().ok().and_then(|k| *k).unwrap_or("包");
        let now = if now == "runtime" { "运行时" } else { "模型" };
        return Err(ApiError::bad_request(format!(
            "{now}正在下载中。想换一个就先暂停或停止它。"
        )));
    }
    if DEL_ACTIVE.load(Ordering::Relaxed) {
        return Err(ApiError::bad_request(
            "正在删除依赖文件，等它删完再下（删到一半开始下会互相拆台）。",
        ));
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
        match res {
            Err(e) => {
                /* 出错 = 那个 `.part` 不可信，别留着让下次去续。
                   ⚠️ 这里只清内存里的记号是**不够**的（判据已经改成看盘了），
                   真正删 `.part` 的是 `fetch_bundle`：它把「打不开 / 不是 206」
                   这些情况都归到「从 0 开始」，那条分支会把 `.part` 和
                   `.part.url` 一起删掉。 */
                if let Ok(mut slot) = DL_ERROR.lock() {
                    *slot = Some(e);
                }
            }
            // 下完 / 暂停 / 停止都走这一条：`.part` 与 `.part.url` 的收拾在
            // `fetch_bundle` 里，两个 `download` 端点只负责在收场后清掉
            // 「已经没有半个包了」的那种记号（`clear_resume_marker`）。
            Ok(_) => {}
        }
    });

    Ok(Json(ok(json!({ "started": true }))))
}

fn note_progress(got: u64, total: Option<u64>, stage: crate::svsep::Stage) {
    DL_BYTES.store(got, Ordering::Relaxed);
    if let Some(t) = total {
        DL_TOTAL.store(t, Ordering::Relaxed);
    }
    DL_STAGE.store(
        if stage == crate::svsep::Stage::Extract {
            1
        } else {
            0
        },
        Ordering::Relaxed,
    );
}

/// 暂停下载：`.part` 留着，下次点「继续下载」带 Range 接着下。
pub async fn download_pause() -> Result<Json<Value>, ApiError> {
    if DL_ACTIVE.load(Ordering::Relaxed) != 1 {
        return Err(ApiError::bad_request("现在没有在下载"));
    }
    DL_PAUSE.store(true, Ordering::Relaxed);
    Ok(Json(ok(json!({ "pausing": true }))))
}

/// 停止下载：把 `.part` 也删掉，下次从头下。
pub async fn download_stop() -> Result<Json<Value>, ApiError> {
    if DL_ACTIVE.load(Ordering::Relaxed) != 1 {
        return Err(ApiError::bad_request("现在没有在下载"));
    }
    DL_STOP.store(true, Ordering::Relaxed);
    Ok(Json(ok(json!({ "stopping": true }))))
}

/// 一键删掉下下来的模型与运行时（**下完的、没下完的都删**）。
///
/// ⚠️ 删运行时等于「下次要重新下 4.7 GB」，所以前端必须让用户确认过。
/// ⚠️ 不删 `backend/`：那几个 .py 随程序打包，不属于「依赖」，删了就得重装。
pub async fn deps_delete(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    if DL_ACTIVE.load(Ordering::Relaxed) == 1 {
        return Err(ApiError::bad_request(
            "正在下载，先暂停或停止再删（边下边删只会留下一堆半截文件）。",
        ));
    }
    if DEL_ACTIVE.swap(true, Ordering::Relaxed) {
        return Err(ApiError::bad_request("正在删除中，等它删完"));
    }
    if st.svsep.probe().await {
        // 引擎正跑着就删运行时 = 删正在运行的 python.exe（必然一批文件删不掉）。
        // 先停服务；真停不掉也不硬来，删不掉的会照实报给用户。
        st.svsep.stop();
    }

    let root = st.root.clone();
    let writable = st.svsep.writable().to_path_buf();
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

    Ok(Json(ok(json!({ "started": true }))))
}

pub async fn status(State(st): State<Arc<AppState>>) -> Json<Value> {
    let s = &st.svsep;
    let running = s.probe().await;
    Json(ok(json!({
        "runtimeReady": s.runtime_ready(),
        "dir": s.dir().to_string_lossy(),
        "modelsDir": s.models().to_string_lossy(),
        "dataDir": s.data().to_string_lossy(),
        "outputsDir": s.outputs().to_string_lossy(),
        "runtime": crate::svsep::runtime_status(&st.root),
        "models": crate::svsep::models_status(s.writable()),
        "download": download_state(&st.root, s.writable()),
        "running": running,
        "port": s.port_hint(),
        "baseUrl": s.base_url(),
        "lastError": s.last_error(),
    })))
}

pub async fn start(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let (port, started) = st
        .svsep
        .start()
        .await
        .map_err(ApiError::bad_request)?;
    let status = st.svsep.get("/api/status").await.unwrap_or(json!({}));
    Ok(Json(ok(json!({
        "running": true,
        "started": started,
        "port": port,
        "baseUrl": st.svsep.base_url(),
        "backend": status,
    }))))
}

pub async fn stop(State(st): State<Arc<AppState>>) -> Json<Value> {
    st.svsep.stop();
    Json(ok(json!({ "running": false })))
}

/// 下模型（压缩包 462 MB，解压后 730 MB）。链接在 `crate::svsep::MODEL_URL`，
/// 用户上传后填。
///
/// 上次是**暂停**在这里的（同一个包、同一条链接）就带着 `Range` 接着下；
/// 换了包、或者上次是出错/停止结束的，就从头下（`fetch_bundle` 会把无效的
/// `.part` 删掉）。
pub async fn models_download(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let writable = st.svsep.writable().to_path_buf();
    let root = st.root.clone();
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
pub async fn runtime_download(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let root = st.root.clone();
    let writable = st.svsep.writable().to_path_buf();
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

/// 提交一次分离。
///
/// body 是**原始 multipart 字节**，原样转给 Python（见 `svsep::Svsep::submit`）。
/// 只从查询串里读 `engine` —— 前端把文件名写进 `Content-Disposition`，
/// 我们不解析它，解析了也只会引入转义 bug。
pub async fn separate(
    State(st): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let engine = q.get("engine").map(String::as_str).unwrap_or("roformer");
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !ct.starts_with("multipart/form-data") {
        return Err(ApiError::bad_request(
            "提交分离要带 multipart 表单（字段名 file）。",
        ));
    }
    if body.is_empty() {
        return Err(ApiError::bad_request("没有收到音频文件"));
    }

    if !st.svsep.probe().await {
        // 顺手把它起起来 —— 用户点「开始分离」时服务通常还没起
        st.svsep.start().await.map_err(ApiError::bad_request)?;
    }

    let mut out = st
        .svsep
        .submit(engine, body.to_vec(), &ct)
        .await
        .map_err(ApiError::bad_request)?;
    // 任务对象捋平：前端拿 `res.task` 直接当任务记录用（`Svsep.tsx::doSeparate`），
    // 而它读的是 `task.id` —— 上游发的是 `task_id`、还包在 `task` 里，见 `flat_task`
    if let Some(o) = out.as_object_mut() {
        if o.contains_key("task") {
            let t = flat_task(o.get("task").cloned().unwrap_or(Value::Null));
            o.insert("task".to_string(), t);
        }
    }
    Ok(Json(ok(out)))
}

pub async fn task(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    /* 服务已经自动关了（任务跑完就关，见 `auto_stop_when_idle`）：这时 `get()` 只会
       回「分离服务还没启动」，可界面要的恰恰是最后那道 `done` —— 先看快照。 */
    if !st.svsep.probe().await {
        if let Some(t) = recall_task(&id) {
            return Ok(Json(ok(t)));
        }
    }
    let v = st
        .svsep
        .get(&format!("/api/status/{id}"))
        .await
        .map_err(ApiError::bad_request)?;
    let t = flat_task(v);
    /* 任务结束了就把服务关掉 —— 用户不用记着点「停止服务」，也不用白占 5 GB 内存。
       隔一会儿、并且确认队列空了才真关（见 `auto_stop_when_idle`）。 */
    if matches!(
        t.get("status").and_then(Value::as_str),
        Some("done") | Some("failed") | Some("cancelled")
    ) {
        remember_task(&id, &t);
        tokio::spawn(auto_stop_when_idle(st.clone()));
    }
    Ok(Json(ok(t)))
}

/// 服务已经不在了（任务跑完自动关的）：把终态快照掏出来答。
///
/// 放在 `task` 的最前面 —— 没服务的时候 `get()` 只会回「分离服务还没启动」，
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

pub async fn cancel(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let v = st
        .svsep
        .post_json(&format!("/api/cancel/{id}"), &json!({}))
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(ok(v)))
}

pub async fn open_output(
    State(st): State<Arc<AppState>>,
    body: Option<Json<Value>>,
) -> Result<Json<Value>, ApiError> {
    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let v = st
        .svsep
        .post_json("/api/open-output", &body)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(ok(v)))
}



/// 取一轨分离结果。
///
/// **直接读磁盘**：`<数据目录>/outputs/<task_id>/<filename>`。不走分离服务，
/// 因为任务一结束服务就自动关了（见 `auto_stop_when_idle`），而结果必须在那之后
/// 还能试听、还能下载。上游 Python 也是从同一个目录发的（`app.py::api_download`）。
///
/// 不做 Range：读的是本机磁盘，整个文件几十 MB 一次性发完，`<audio>` 拖动进度条
/// 也只是让它重新拿一遍 —— 比在转发里维护 Range 语义省事，效果看不出差别。
pub async fn output(
    State(st): State<Arc<AppState>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    // 两个参数都会拼进路径，所以只认「纯名字」：id 是上游发的 uuid hex，
    // name 是 `outputs[].filename`（上游存的就是 basename，带扩展名）。
    if id.is_empty()
        || name.is_empty()
        || !id.chars().all(|c| c.is_ascii_alphanumeric())
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Err(ApiError::bad_request("输出文件名不合法"));
    }

    let path = st.svsep.output_path(&id, &name);
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        ApiError::not_found(format!("取不到这一轨（{}）：{e}", path.to_string_lossy()))
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, audio_mime(&path))
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .map_err(|e| ApiError::internal(format!("构造响应失败：{e}")))
}

/// 分离服务的原始状态（`/api/status`）—— 页面上「设备 / 队列 / 输出目录」那一条用
pub async fn backend_status(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let v = st
        .svsep
        .get("/api/status")
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(ok(v)))
}

pub async fn system_stats(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let v = st
        .svsep
        .get("/api/system-stats")
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(ok(v)))
}

/// 推理方式：自动 / GPU / CPU。
///
/// 服务在跑就问它（它会顺手探一下硬件，给出 `badge` / `hardware`）；**服务没跑
/// 就读盘**上的 `<数据目录>/inference_settings.json` —— 任务一结束服务就自动关
/// 了（`auto_stop_when_idle`），可这个设置项在界面上得一直看得见、改得动。
pub async fn inference_get(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    if st.svsep.probe().await {
        if let Ok(v) = st.svsep.get("/api/inference-settings").await {
            return Ok(Json(ok(v)));
        }
    }
    let mode = read_infer_mode(&st);
    Ok(Json(ok(infer_reply(&mode, true))))
}

pub async fn inference_set(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let raw = body.get("mode").and_then(Value::as_str).unwrap_or("");
    if !["auto", "cpu", "gpu"].contains(&raw.trim().to_ascii_lowercase().as_str()) {
        return Err(ApiError::bad_request("无效模式，请选择 auto / cpu / gpu"));
    }
    let mode = normalize_mode(raw);
    write_infer_mode(&st, &mode)?;
    /* 服务在跑就再告诉它一声：它把 mode 缓存在进程内存里（`inference_settings.py`
       的 `_cached_mode`），光改文件它不认。它没起来、或者答错了都不影响结果 ——
       下次启动读的就是这个文件。 */
    if st.svsep.probe().await {
        if let Ok(v) = st
            .svsep
            .post_json("/api/inference-settings", &json!({ "mode": mode }))
            .await
        {
            return Ok(Json(ok(v)));
        }
    }
    Ok(Json(ok(infer_reply(&mode, true))))
}

/* ══════════════════════════════ 小工具 ══════════════════════════════ */

/// 把上游的任务对象捋平。
///
/// 上游 `GET /api/status/<id>` 回的是 `{"ok":true,"task":{…}}`，而且里面的任务
/// id 叫 `task_id`；前端（`api.ts::SvsepTask`）按**扁平 + `id`** 读。2026-10-03
/// 之前这里是原样透传的 —— 前端第一轮轮询拿到的是外壳对象，`task.id` 变
/// undefined，任务轮询自己停掉：界面永远停在提交那一刻（用户报的「分离完了
/// 进度还卡在 5%」就是这个）。
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
/// （`queues_busy` 会拦住）。`stop()` 是幂等的（`svsep.rs`），重复调用无害。
async fn auto_stop_when_idle(st: Arc<AppState>) {
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
fn inference_file(st: &AppState) -> std::path::PathBuf {
    st.svsep.data().join("inference_settings.json")
}

/// 读盘上的推理方式（没有文件、文件坏了都算 `auto`，跟上游一致）。
fn read_infer_mode(st: &AppState) -> String {
    std::fs::read_to_string(inference_file(st))
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
fn write_infer_mode(st: &AppState, mode: &str) -> Result<(), ApiError> {
    let path = inference_file(st);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ApiError::internal(format!("建目录失败：{e}")))?;
    }
    let body = serde_json::to_string_pretty(&json!({ "mode": mode })).unwrap_or_default();
    std::fs::write(&path, body).map_err(|e| ApiError::internal(format!("写推理设置失败：{e}")))
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

/// 输出文件的内容类型。上游只产音频（wav/mp3/flac…），认不出就当二进制。
///
/// ⚠️ 这里写全路径 `std::path::Path`：本模块顶上的 `Path` 是 axum 的提取器。
fn audio_mime(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("m4a") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("ogg") => "audio/ogg",
        Some("wma") => "audio/x-ms-wma",
        _ => "application/octet-stream",
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
    fn audio_mime_covers_what_the_backend_writes() {
        use std::path::Path as StdPath;
        assert_eq!(audio_mime(StdPath::new("a/vocals_x.wav")), "audio/wav");
        assert_eq!(audio_mime(StdPath::new("a.mp3")), "audio/mpeg");
        assert_eq!(audio_mime(StdPath::new("A.FLAC")), "audio/flac");
        assert_eq!(
            audio_mime(StdPath::new("notes.txt")),
            "application/octet-stream"
        );
        assert_eq!(
            audio_mime(StdPath::new("noext")),
            "application/octet-stream"
        );
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
