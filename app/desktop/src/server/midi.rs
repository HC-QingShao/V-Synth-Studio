//! 人声转 MIDI 的 HTTP 路由。
//!
//! 这一组和音轨分离那组长得像，但**底下完全不同**：那边是转发给一个 Python
//! HTTP 服务，这边是直接在进程里算（`crate::midi_transcribe`）。没有端口、没有
//! 健康检查、没有「服务没起来」这一类错误 —— 只有输入不对和模型没装。
//!
//! 路由一览：
//!   GET  /api/midi/status             动态库 / 模型 / 半个包的状态（前端轮询它）
//!   POST /api/midi/models/download    下 ONNX 权重包（364 MB）并解包
//!   POST /api/midi/runtime/download   下 ONNX Runtime（官方 zip 78 MB，只留里面那个 dll）
//!   POST /api/midi/download/pause     ⚠️ 等价于「停止」——这个包不支持续传，界面不用它
//!   POST /api/midi/download/stop      停止下载并删掉半截文件
//!   POST /api/midi/deps/delete        删掉下下来的模型与动态库（回 {files, bytes, note}）
//!   POST /api/midi/transcribe         提交一次扒谱（body 里给音频路径）
//!   GET  /api/midi/task/{id}          查任务
//!   POST /api/midi/task/{id}/cancel   取消任务
//!   POST /api/midi/open-output        在资源管理器里选中输出目录
//!   GET  /api/midi/task/{id}/file/{name}  取结果文件（.mid / .csv / .json）
//!
//! 为什么没有「上传音频」：音频本来就在用户盘上，多传一遍 600 MB 只是把同一份
//! 数据从磁盘搬到磁盘。音轨分离那边要 multipart 是因为它的后端只认字节流，
//! 这里没有那个约束。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::{convert, ok, ApiError, AppState};
use crate::game::engine;
use crate::midi_transcribe as mt;

/// 下载进度（前端每 2 秒轮询 `/api/midi/status`）。
///
/// 和音轨分离那三个静态量是**分开的一份**，不是共用：两边可以同时下（用户一边
/// 下分离引擎一边下扒谱模型完全合理），共用一份状态会让两条进度条互相踩。
static DL_ACTIVE: AtomicU64 = AtomicU64::new(0);
static DL_BYTES: AtomicU64 = AtomicU64::new(0);
static DL_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 0 = 在下字节，1 = 下完了、正在解压（`crate::svsep::Stage`）
static DL_STAGE: AtomicU64 = AtomicU64::new(0);
static DL_KIND: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);
static DL_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static DL_PAUSE: AtomicBool = AtomicBool::new(false);
static DL_STOP: AtomicBool = AtomicBool::new(false);

/// 正在跑的任务 id 与它的取消旗标。
///
/// 同时只允许一个：推理是**纯 CPU 的多秒级阻塞活**，两个一起跑只会让两边都慢
/// 一倍（还有 400 MB 的图各占一份内存）。前端也不给并发入口。
static RUNNING: std::sync::Mutex<Option<(String, Arc<mt::Cancel>)>> = std::sync::Mutex::new(None);

/* ══════════════════════════════════ 状态 ══════════════════════════════════ */

fn download_state() -> Value {
    let active = DL_ACTIVE.load(Ordering::Relaxed) == 1;
    json!({
        "active": active,
        "kind": DL_KIND.lock().ok().and_then(|k| *k),
        "stage": if DL_STAGE.load(Ordering::Relaxed) == 1 { "extract" } else { "download" },
        "done": DL_BYTES.load(Ordering::Relaxed) as f64,
        "total": DL_TOTAL.load(Ordering::Relaxed) as f64,
        "error": DL_ERROR.lock().ok().and_then(|e| e.clone()),
        // 这个包**不支持续传**（见 `svsep::fetch_to_file` 的注释），
        // 所以永远没有「继续下载」这一说 —— 前端据此把按钮文案固定成「下载模型」。
        "resumable": false,
    })
}

pub async fn status(State(st): State<Arc<AppState>>) -> Json<Value> {
    let mut v = mt::status(&st.root, &st.writable);
    if let Some(o) = v.as_object_mut() {
        o.insert("download".into(), download_state());
        let running = RUNNING
            .lock()
            .ok()
            .and_then(|r| r.as_ref().map(|(id, _)| id.clone()));
        o.insert("running".into(), json!(running));
    }
    Json(ok(v))
}

/* ══════════════════════════════════ 下载 ══════════════════════════════════ */

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

/// 起一个后台下载，立刻返回。`kind` 是 `"models"` / `"runtime"`。
///
/// 和音轨分离那边同一套骨架：**不占着请求**，进度由前端轮询 `/api/midi/status`。
/// 那边还要照顾暂停续传、`.part` 记号、五轮重试的收场；这里因为不分段续传，
/// 收场简单得多（错了就删半截文件）。
fn spawn_download<F>(kind: &'static str, job: F) -> Result<Json<Value>, ApiError>
where
    F: std::future::Future<Output = Result<Value, String>> + Send + 'static,
{
    if DL_ACTIVE.load(Ordering::Relaxed) == 1 {
        return Err(ApiError::bad_request("已经有一个下载在跑了，等它结束。"));
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
    DL_BYTES.store(0, Ordering::Relaxed);
    DL_ACTIVE.store(1, Ordering::Relaxed);

    tokio::spawn(async move {
        let res = job.await;
        DL_ACTIVE.store(0, Ordering::Relaxed);
        if let Ok(mut k) = DL_KIND.lock() {
            *k = None;
        }
        if let Err(e) = res {
            if let Ok(mut slot) = DL_ERROR.lock() {
                *slot = Some(e);
            }
        }
    });

    Ok(Json(ok(json!({ "started": true }))))
}

pub async fn models_download(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    if !mt::missing_models(&st.root, &st.writable).is_empty()
        && mt::models_dir(&st.root, &st.writable).join("encoder.onnx").is_file()
    {
        // 已经装了就别再下 364 MB —— 前端按钮本来也会换掉，这是防手快 / 防旧页面。
        let missing = mt::missing_models(&st.root, &st.writable);
        if missing.is_empty() {
            return Err(ApiError::bad_request("模型已经装好了，不用再下。"));
        }
    }
    let writable = st.writable.clone();
    // `DownloadCtl` 收的是 `&'static AtomicBool`，而这两个旗标正是 `'static` ——
    // 直接借，不用 clone（clone 出来的是个临时值，借不成 'static）。
    let ctl = crate::svsep::DownloadCtl::new(&DL_PAUSE, &DL_STOP, None);
    spawn_download("models", async move {
        let v = mt::download_models(&writable, &ctl, note_progress).await?;
        crate::log_line(&format!("人声转 MIDI：模型下载完成 {:?}", v.get("dir")));
        Ok(v)
    })
}

pub async fn runtime_download(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    if mt::runtime_dll(&st.root, &st.writable).is_some() {
        return Err(ApiError::bad_request(
            "ONNX Runtime 已经能用了（本机已有，或已从音轨分离那边借到），不用再下。",
        ));
    }
    let writable = st.writable.clone();
    let ctl = crate::svsep::DownloadCtl::new(&DL_PAUSE, &DL_STOP, None);
    spawn_download("runtime", async move {
        let v = mt::download_runtime(&writable, &ctl, note_progress).await?;
        crate::log_line(&format!("人声转 MIDI：ONNX Runtime 就位 {:?}", v.get("dll")));
        Ok(v)
    })
}

pub async fn download_pause() -> Result<Json<Value>, ApiError> {
    if DL_ACTIVE.load(Ordering::Relaxed) != 1 {
        return Err(ApiError::bad_request("现在没有在下载"));
    }
    DL_PAUSE.store(true, Ordering::Relaxed);
    Ok(Json(ok(json!({ "pausing": true }))))
}

pub async fn download_stop() -> Result<Json<Value>, ApiError> {
    if DL_ACTIVE.load(Ordering::Relaxed) != 1 {
        return Err(ApiError::bad_request("现在没有在下载"));
    }
    DL_STOP.store(true, Ordering::Relaxed);
    Ok(Json(ok(json!({ "stopping": true }))))
}

pub async fn deps_delete(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    if DL_ACTIVE.load(Ordering::Relaxed) == 1 {
        return Err(ApiError::bad_request("正在下载，先停下再删。"));
    }
    if RUNNING.lock().ok().and_then(|r| r.as_ref().map(|_| ())).is_some() {
        return Err(ApiError::bad_request("正在扒谱，等它跑完再删模型。"));
    }
    let (files, bytes, note) = mt::delete_deps(&st.root, &st.writable);
    crate::log_line(&format!(
        "人声转 MIDI：删掉 {files} 个文件、{bytes} 字节；{note}"
    ));
    Ok(Json(ok(json!({ "files": files, "bytes": bytes, "note": note }))))
}

/* ══════════════════════════════════ 扒谱 ══════════════════════════════════ */

/// 提交一次扒谱。body：
///
/// ```json
/// { "input": "D:\\歌\\干声.wav", "outDir": "D:\\歌\\midi", "steps": 8, "language": 4 }
/// ```
///
/// `input` **是路径不是上传的字节**（见文件头的说明）；`outDir` 不给就写到音频
/// 同目录下的 `midi` 子目录。
pub async fn transcribe(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let input = body
        .get("input")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if input.is_empty() {
        return Err(ApiError::bad_request("没给音频路径"));
    }
    let input_path = std::path::PathBuf::from(&input);
    if !input_path.is_file() {
        return Err(ApiError::bad_request(format!("文件不存在：{input}")));
    }
    // 输出不能盖住输入：用户把 .mid 拖进来（想「再扒一遍」）时，
    // 输出主干和输入同名同目录，会直接把源文件写没。
    if input_path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase() == "mid")
        .unwrap_or(false)
    {
        return Err(ApiError::bad_request(
            "输入是 MIDI 文件，扒谱要的是音频（wav / mp3 / flac…）。",
        ));
    }

    let models = mt::models_dir(&st.root, &st.writable);
    let missing = engine::missing_models(&models);
    if !missing.is_empty() {
        return Err(ApiError::bad_request(format!(
            "模型还没装全（缺 {}）。先点「下载模型」。",
            missing.join("、")
        )));
    }
    let Some(dll) = mt::runtime_dll(&st.root, &st.writable) else {
        return Err(ApiError::bad_request(
            "缺 ONNX Runtime 动态库。点「下载运行库」，装了音轨分离的话它能直接借到。",
        ));
    };

    {
        let guard = RUNNING.lock().map_err(|_| ApiError::internal("任务表坏了"))?;
        if guard.is_some() {
            return Err(ApiError::bad_request(
                "已经有一次扒谱在跑了。这个功能一次只跑一个（CPU 都被它占着）。",
            ));
        }
    }

    let out_dir = body
        .get("outDir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            input_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("midi")
        });

    let opts = engine::Options {
        steps: clamp_usize(body.get("steps").and_then(|v| v.as_u64()), 8, 1, 32),
        language: clamp_usize(body.get("language").and_then(|v| v.as_i64().map(|i| i as u64)), 4, 0, 126)
            as i64,
        threads: clamp_usize(body.get("threads").and_then(|v| v.as_u64()), 4, 1, 32),
    };

    let title = format!("人声转 MIDI · {}", mt::output_stem(&input_path));
    let job_id = convert::new_job(&st, "transcribe", &title, "");
    convert::log_job(
        &st,
        &job_id,
        &format!("输入：{input}（去噪 {} 步）", opts.steps),
    );

    let cancel = mt::Cancel::new();
    if let Ok(mut slot) = RUNNING.lock() {
        *slot = Some((job_id.clone(), cancel.clone()));
    }

    let st2 = st.clone();
    let id2 = job_id.clone();
    let tools_dir = st.tools_dir();
    let stem = mt::output_stem(&input_path);
    let scratch = mt::data_dir(&st.writable).join("work");
    tokio::spawn(async move {
        run_job(st2, id2, input_path, out_dir, scratch, tools_dir, models, dll, opts, cancel, stem)
            .await;
    });

    Ok(Json(ok(json!({ "jobId": job_id }))))
}

/// 任务主体：ffmpeg 转码 → 推理 → 落盘。所有失败都变成任务里的 `status: error`。
#[allow(clippy::too_many_arguments)]
async fn run_job(
    st: Arc<AppState>,
    job_id: String,
    input: std::path::PathBuf,
    out_dir: std::path::PathBuf,
    scratch: std::path::PathBuf,
    tools_dir: std::path::PathBuf,
    models: std::path::PathBuf,
    dll: std::path::PathBuf,
    opts: engine::Options,
    cancel: Arc<mt::Cancel>,
    stem: String,
) {
    let result = run_job_inner(
        st.clone(),
        job_id.clone(),
        &input,
        &out_dir,
        &scratch,
        &tools_dir,
        &models,
        &dll,
        &opts,
        &cancel,
        &stem,
    )
    .await;

    // 收工时把「正在跑」这一格腾出来，**而且要认 id**：取消之后用户可能已经
    // 提交了下一次任务，这时槽里装的是后来那个 —— 无条件清空会把新任务
    // 从「忙」变成「空闲」，界面就允许再提交一个了。
    if let Ok(mut slot) = RUNNING.lock() {
        if slot.as_ref().map(|(id, _)| id.as_str()) == Some(job_id.as_str()) {
            *slot = None;
        }
    }
    match result {
        Ok(notes) => convert::finish_job(&st, &job_id, &format!("完成：{notes} 个音符")),
        Err(e) if e == mt::CANCELLED => {
            convert::set_job(
                &st,
                &job_id,
                json!({ "status": "canceled", "message": "已取消" }),
            );
        }
        Err(e) => {
            convert::log_job(&st, &job_id, &e);
            // ⚠️ 状态词是 `"error"` 不是 `"failed"` —— 前端 `Job['status']` 的联合类型
            // 只有 `running | done | error | canceled`，别的词会让 `useJob` 永远
            // 收不到终态、进度条一直转。`server/media.rs:90` 是这个写法。
            convert::set_job(
                &st,
                &job_id,
                json!({ "status": "error", "error": e, "message": e }),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_job_inner(
    st: Arc<AppState>,
    job_id: String,
    input: &std::path::Path,
    out_dir: &std::path::Path,
    scratch: &std::path::Path,
    tools_dir: &std::path::Path,
    models: &std::path::Path,
    dll: &std::path::Path,
    opts: &engine::Options,
    cancel: &Arc<mt::Cancel>,
    stem: &str,
) -> Result<usize, String> {
    std::fs::create_dir_all(scratch).map_err(|e| format!("建临时目录失败：{e}"))?;
    // 临时 wav 用任务 id 命名：跑两次同一首歌不会撞车，异常退出留下的残骸
    // 也能一眼看出是哪次任务留下的。
    let wav = scratch.join(format!("{job_id}.wav"));

    // ── 1. 解码 ─────────────────────────────────────────────────────────
    //
    // ⚠️ 两个回调都必须**拿走所有权**（`move` + `Arc<AppState>` / `String`），
    //    不能借用 `&Arc<AppState>` 与 `&str`：`crate::audio::Progress` 是
    //    `dyn Fn(f64, f64) + Send + Sync`（隐式 `'static`），借用会让
    //    `borrowed data escapes outside of function`（实测 E0521）。
    //    代价只是每个闭包一份 `Arc` 的引用计数，很便宜。
    let probe = crate::audio::probe_media(tools_dir, &input.to_string_lossy()).await;
    let duration = probe
        .get("duration")
        .and_then(|v| v.as_f64())
        .filter(|d| *d > 0.0)
        .unwrap_or(0.0);
    let st_dec = st.clone();
    let id_dec = job_id.clone();
    convert::set_job(
        &st_dec,
        &id_dec,
        json!({ "message": "正在解码音频…", "percent": 1 }),
    );
    let ctl = cancel.clone();
    let cancel_flag = move || ctl.stopped();
    let on_dec = move |pct: f64, _sec: f64| {
        convert::set_job(
            &st_dec,
            &id_dec,
            json!({ "message": "正在解码音频…", "percent": (pct * 0.05).min(5.0) }),
        );
    };
    mt::decode_to_wav(tools_dir, input, &wav, duration, &cancel_flag, &on_dec).await?;
    if cancel.stopped() {
        let _ = std::fs::remove_file(&wav);
        return Err(mt::CANCELLED.into());
    }

    // ── 2. 推理（阻塞，必须 spawn_blocking）──────────────────────────────
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, f64)>();
    let models = models.to_path_buf();
    let dll = dll.to_path_buf();
    let wav2 = wav.clone();
    let opts2 = opts.clone();
    let cancel2 = cancel.clone();
    let started = std::time::Instant::now();
    let handle = tokio::task::spawn_blocking(move || {
        mt::transcribe_blocking(&models, &dll, &wav2, &opts2, &cancel2, move |what, pct| {
            // 推理只占整条时间线的 5%..95%（前面解码、后面落盘都要留位置）
            let _ = tx.send((what.to_string(), 5.0 + pct * 90.0));
        })
    });

    // 边等边把进度灌进任务表：`spawn_blocking` 的返回值要 await，
    // 而进度是从另一个线程经 channel 过来的 —— 轮询式 `try_recv` + 短 sleep
    // 是最省事的写法（进度本来就只有几十条，不值得为它上 select!）。
    let report = loop {
        while let Ok((what, pct)) = rx.try_recv() {
            convert::set_job(&st, &job_id, json!({ "message": what, "percent": pct }));
        }
        if handle.is_finished() {
            break handle.await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    };
    let _ = std::fs::remove_file(&wav);

    let report = report.map_err(|e| format!("推理线程崩了：{e}"))??;

    // ── 3. 落盘 ─────────────────────────────────────────────────────────
    convert::set_job(&st, &job_id, json!({ "message": "正在写文件…", "percent": 96 }));
    let written = mt::write_outputs(out_dir, stem, &report)?;
    let names: Vec<String> = written
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    convert::log_job(
        &st,
        &job_id,
        &format!(
            "写出 {}（encoder {:.1}s / segmenter {:.1}s / estimator {:.1}s，总 {:.1}s）",
            names.join("、"),
            report.encoder_seconds,
            report.segmenter_seconds,
            report.estimator_seconds,
            started.elapsed().as_secs_f64(),
        ),
    );

    let notes = report.notes.len();
    convert::set_job(
        &st,
        &job_id,
        json!({
            "message": format!("完成：{notes} 个音符"),
            "percent": 100,
            "result": {
                "dir": crate::platform::clean_path(out_dir),
                "files": names,
                "notes": notes,
                "seconds": {
                    "encoder": report.encoder_seconds,
                    "segmenter": report.segmenter_seconds,
                    "estimator": report.estimator_seconds,
                },
                // 前 200 个音符给界面画钢琴卷帘 —— 一首歌几千个音符没必要全塞进
                // 轮询响应里，要全的都在 .json 里。
                "preview": engine::note_rows(&report.notes).into_iter().take(200)
                    .map(|(o, e, p, m)| json!({ "onset": o, "offset": e, "pitch": p, "midi": m }))
                    .collect::<Vec<_>>(),
            },
        }),
    );
    Ok(notes)
}

fn clamp_usize(v: Option<u64>, default: usize, lo: usize, hi: usize) -> usize {
    match v {
        Some(n) => (n as usize).clamp(lo, hi),
        None => default,
    }
}

/* ══════════════════════════════════ 任务查询 ══════════════════════════════════ */

pub async fn task(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let guard = st.jobs.lock().unwrap();
    let job = guard.items.get(&id).cloned();
    drop(guard);
    match job {
        Some(mut j) => {
            if let Some(o) = j.as_object_mut() {
                o.insert("id".into(), json!(id));
            }
            Ok(Json(ok(j)))
        }
        None => Err(ApiError::not_found(format!("没有这个任务：{id}"))),
    }
}

pub async fn cancel(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Json<Value> {
    let mut hit = false;
    // 只读地看一眼槽里是谁 —— 不去改它：**这里不该把槽清空**，清空是
    // `run_job` 收工时的活（它要认 id）。否则任务还在跑，`status.running`
    // 就已经是空的，界面会放开「再提交一个」。
    if let Ok(slot) = RUNNING.lock() {
        if let Some((cur, cancel)) = slot.as_ref() {
            if *cur == id {
                cancel.stop();
                hit = true;
            }
        }
    }
    // 旗标立了，真正的收场在推理那一侧（它在每个切片边界看一眼）。
    // 这里**不改任务状态** —— 由 `run_job` 统一写，免得两边打架。
    if hit {
        convert::log_job(&st, &id, "收到取消请求，正在当前这一段结束后停下…");
    }
    Json(ok(json!({ "canceled": hit })))
}

/// 在资源管理器里选中一个目录。
///
/// ⚠️ 和音轨分离那边不同：结果**不在固定的 `<数据目录>/outputs/<id>`**，
/// 而是落在用户选的目录（或音频同目录下的 `midi/`）。所以前端必须把
/// 任务 `result.dir` 原样传回来；空串只兜底开本功能的数据目录。
pub async fn open_output(
    State(st): State<Arc<AppState>>,
    body: Option<Json<Value>>,
) -> Result<Json<Value>, ApiError> {
    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").trim();
    let target = if dir.is_empty() {
        // 不给就开本功能的数据目录（模型、动态库都在这儿，用户想翻也翻得到）
        mt::data_dir(&st.writable).to_string_lossy().to_string()
    } else {
        dir.to_string()
    };
    let p = std::path::Path::new(&target);
    if !p.exists() {
        return Err(ApiError::bad_request(format!("目录不存在：{target}")));
    }
    crate::platform::reveal_in_explorer(&target, false).map_err(ApiError::from)?;
    Ok(Json(ok(json!({ "path": target }))))
}

/// 取结果文件。
///
/// 输出目录是用户任选的、记在任务的 `result.dir` 里，所以这里得回任务表查 ——
/// 不能像音轨分离那样按 `<数据目录>/outputs/<id>/` 拼出来。
///
/// **不检查它是不是这次任务的产物**：用户要听的就是那个 `.mid` / 看那个 `.csv`，
/// 路径本来就由 `task` 的 `result.files` 给出，这里只把字节发出去。
/// 但仍然挡住路径穿越（`..`、分隔符、绝对路径）—— 参数会拼进路径，不能白信任。
pub async fn output(
    State(st): State<Arc<AppState>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    if id.is_empty()
        || name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
    {
        return Err(ApiError::bad_request("文件名不合法"));
    }
    let dir = job_output_dir(&st, &id)
        .ok_or_else(|| ApiError::not_found("任务不在，或它还没写出文件（工作站重启过？）"))?;
    let path = std::path::Path::new(&dir).join(&name);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::not_found(format!("读不到 {}：{e}", path.display())))?;
    let mime = match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("mid") | Some("midi") => "audio/midi",
        Some("csv") => "text/csv; charset=utf-8",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// 从任务表里挖出这次任务写到哪个目录。
fn job_output_dir(st: &Arc<AppState>, id: &str) -> Option<String> {
    let guard = st.jobs.lock().ok()?;
    let job = guard.items.get(id)?;
    job.get("result")?
        .get("dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
