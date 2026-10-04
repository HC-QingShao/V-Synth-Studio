// V-Synth-Studio  ·  QingMu39
// Tauri 桌面应用（纯 IPC，没有 HTTP 服务）
//
// 架构：
//   一个进程搞定所有事 —— 窗口、命令处理、转换编排、任务系统全在这里。
//   前端加载的是 Tauri 自己的资源协议（Windows 上是 `http://tauri.localhost`），
//   前后端交互**只走 `tauri::command`**（见 `src/ipc/`）。
//
//   这里以前有一个同进程的 axum HTTP 服务（56 条路由 + 静态文件伺服，5,610 行），
//   因为窗口是用 `WebviewUrl::External` 加载 `http://127.0.0.1:17878` 的 —— 那种
//   加载方式下页面拿不到 Tauri IPC，只能自己造一套 REST。2026-10 那次减法重构把它
//   整个删了：窗口改用 `WebviewUrl::App`，于是端口、静态伺服、SSE、上传、
//   Range 代理这一整层替身全部消失。
//
// ── 关于「为什么没有控制台窗口」──
// 这里**始终**用 windows 子系统，debug 版也不例外。
// 早先是 `cfg_attr(not(debug_assertions), ...)`，只有 release 版无窗口，
// 结果开发时天天挂着一个黑框。
//
// 代价是看不到 stdout —— 所以日志改成写文件（见 log_line）。
// 这反而更好用：日志能翻历史、能搜索，也不会因为关掉窗口就丢了。

#![windows_subsystem = "windows"]

mod audio;
mod bili;
mod data;
mod game;
mod ipc;
mod libresvip;
mod lyrics;
mod midi_transcribe;
mod net;
mod platform;
mod svsep;

/// 追加一行日志到 `<可写目录>/app.log`。
///
/// 为什么不用 stdout：程序是 windows 子系统（无控制台窗口），
/// 打出去的东西没人看得见。写文件反而更好用 —— 能翻历史、能搜索，
/// 也不会因为关掉窗口就丢了。
///
/// 刻意不引日志库：这里只有十来行输出，一个 `OpenOptions::append` 就够。
///
/// `pub(crate)`：别的模块里也有需要记一行的地方（见 `ipc/config.rs` 读配置那处）。
pub(crate) fn log_line(msg: &str) {
    use std::io::Write;

    // 启动早期路径还没解析出来，退回临时目录，保证日志不丢
    let dir = resolve_paths(None)
        .map(|p| p.writable)
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&dir);

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简单的时间戳：不引时间库，用「自纪元起的秒」也够定位
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("app.log"))
    {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

/// 把日志同时写到文件和 stdout（stdout 在无控制台时会被丢弃，无害）。
///
/// 用宏而不是函数：`println!` 的格式化参数直接转发，不用先拼字符串。
macro_rules! note {
    ($($arg:tt)*) => {{
        let s = format!($($arg)*);
        crate::log_line(&s);
        #[cfg(debug_assertions)]
        println!("{s}");
    }};
}

mod tools;

mod ytdlp;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const WINDOW_TITLE: &str = "V-Synth-Studio";

fn main() {
    /*
     * 界面只有一套，入口是 Tauri 的资源协议（见 `tauri.conf.json` 的 `frontendDist`）。
     * 2026-10-02：旧前端（`app/web/js` + `css` + 那个手写 `index.html`）整体退役；
     * 在那之前这里是一段 `--ui=next|old` 的选择逻辑，两套界面并存是为了并行搬迁。
     *
     * ⚠️ `--serve` / `--port=` 已经**不存在**了。以前那个「只跑服务不开窗口」的模式
     * 是为了给 `tests/contract` 与无头浏览器探针打端口用；HTTP 层删掉之后它没有意义，
     * 传了也只会照常开窗口。要跑自动化就用 Rust 侧的单测（`cargo test --bins`）。
     */
    note!("界面：React 前端（Tauri 资源协议 + IPC）");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();

            // 安装版的界面/工具在 Tauri 的 resource_dir 下；绿色版在 exe 旁边。
            // 两种都交给 resolve_paths 判断，不用在这里分叉。
            let paths = match resolve_paths(app.path().resource_dir().ok()) {
                Some(p) => p,
                None => {
                    show_error(
                        &handle,
                        "找不到程序文件（app/web/index.html）。\n\
                         这个文件是前端产物，也是「程序根目录」的判定依据。\n\
                         绿色版：请把整个目录一起解压，不要只拷 exe。\n\
                         安装版：安装可能不完整，建议重新安装。",
                    );
                    return Ok(());
                }
            };
            note!("  根目录：{}", crate::platform::clean_path(&paths.root));
            if paths.installed {
                note!("  配置目录：{}", crate::platform::clean_path(&paths.writable));
            }

            /*
             * 状态**必须** `manage` 进来（IPC command 靠 `State<Arc<AppState>>` 取它）。
             *
             * ⚠️ 这里和以前不一样，而且修掉了一段死代码：那时 `AppState` 是在
             * `serve()`（一个 spawn 出去的 task）里建的局部变量，从没 `manage` 过，
             * 所以 `RunEvent::Exit` 里的 `try_state::<Arc<AppState>>()` **永远回 None**
             * —— 那段「退出时收子进程」的代码看着在收、其实一次都没跑。
             * 现在它真的取得到了。
             */
            let state = ipc::AppState::new(paths.clone());
            app.manage(Arc::clone(&state));

            // 后台预热一次外部工具探测：`detect_tools` 会真的 spawn
            // `yt-dlp --version` / `python --version` 并逐段扫 PATH，机器忙时 2~7 秒。
            // 不预热的话，前端首屏那次 `get_state` 就要干等这几秒（用户看到的是白屏）。
            std::thread::spawn(move || {
                let _ = state.probe_cached(false);
            });

            /*
             * ⚠️ **不再关掉 Tauri 的拖放拦截。**
             *
             * 以前这里是 `.disable_drag_drop_handler()` —— Tauri 默认会把拖进来的文件
             * 截走、改发成 Tauri 事件，而那时页面拿不到 IPC，只能收 HTML5 的 drop 事件，
             * 所以必须关掉它。
             *
             * 现在反过来：**我们要的正是那个 Tauri 事件** —— `DragDropEvent::Drop`
             * 的载荷里直接带 `paths`（真路径），比 HTML5 那条「拿到 File 对象再上传
             * 换一个路径」的路少一整圈。所以用默认值（开着拦截）。
             */
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(WINDOW_TITLE)
                .inner_size(1360.0, 880.0)
                .min_inner_size(960.0, 640.0)
                .center()
                .build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            /* ── 状态与配置 ── */
            ipc::state::get_state,
            ipc::config::get_config,
            ipc::config::set_config,
            ipc::config::migrate_legacy_settings,
            /* ── 文件系统 ── */
            ipc::fs::pick_paths,
            ipc::fs::allow_path,
            ipc::fs::open_path,
            ipc::fs::open_url,
            ipc::fs::upload_dropped,
            ipc::fs::read_bytes,
            ipc::fs::mkdir,
            ipc::fs::remove_path,
            /* ── 文字 PV 分块落盘 ── */
            ipc::pv::pv_save_chunk,
            /* ── 任务与资源库 ── */
            ipc::jobs::list_jobs,
            ipc::jobs::get_job,
            ipc::jobs::cancel_job,
            ipc::jobs::job_watch,
            ipc::jobs::get_resources,
            ipc::jobs::check_resources,
            /* ── 外部工具 ── */
            ipc::tools::tools_detect,
            ipc::tools::tools_launch,
            /* ── 工程转换 ── */
            ipc::convert::convert_collect,
            ipc::convert::convert_inspect,
            ipc::convert::convert_preview,
            ipc::convert::convert_run,
            /* ── 视频解析下载 / 音频 / 预览缓存 ── */
            ipc::media::video_parse,
            ipc::media::video_download,
            ipc::media::audio_probe,
            ipc::media::audio_run,
            ipc::media::preview_fetch,
            ipc::media::preview_clear,
            /* ── B 站扫码登录 ── */
            ipc::bili::bili_qr_generate,
            ipc::bili::bili_qr_poll,
            ipc::bili::bili_logout,
            /* ── 歌词（网易云专栏）── */
            ipc::lyrics::lyrics_search,
            ipc::lyrics::lyrics_get,
            ipc::lyrics::lyrics_parse_link,
            ipc::lyrics::lyrics_import,
            ipc::lyrics::lyrics_save,
            ipc::lyrics::lyrics_cover,
            ipc::lyrics::lyrics_song,
            ipc::lyrics::lyrics_logout,
            ipc::lyrics::lyrics_login_sms,
            ipc::lyrics::lyrics_login_cellphone,
            /* ── 音轨分离 ── */
            ipc::svsep::svsep_status,
            ipc::svsep::svsep_start,
            ipc::svsep::svsep_stop,
            ipc::svsep::svsep_models_download,
            ipc::svsep::svsep_runtime_download,
            ipc::svsep::svsep_download_pause,
            ipc::svsep::svsep_download_stop,
            ipc::svsep::svsep_deps_delete,
            ipc::svsep::svsep_backend_status,
            ipc::svsep::svsep_inference_get,
            ipc::svsep::svsep_set_inference,
            ipc::svsep::svsep_separate,
            ipc::svsep::svsep_task,
            ipc::svsep::svsep_cancel,
            ipc::svsep::svsep_open_output,
            /* ── 人声转 MIDI ── */
            ipc::midi::midi_status,
            ipc::midi::midi_device_get,
            ipc::midi::midi_device_set,
            ipc::midi::midi_models_download,
            ipc::midi::midi_runtime_download,
            ipc::midi::midi_download_stop,
            ipc::midi::midi_deps_delete,
            ipc::midi::midi_transcribe,
            ipc::midi::midi_task,
            ipc::midi::midi_cancel,
            ipc::midi::midi_open_output,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri 应用构建失败")
        .run(|app, event| {
            /*
             * 真正收子进程（分离引擎的 python.exe）的是两套机制，缺一不可：
             *
             *   1. `impl Drop for Svsep` —— 正常退出时跑。`Svsep` 是 `AppState` 的字段，
             *      而下面的 `try_state` 现在真的拿得到它，所以正常退出这条路是可靠的。
             *   2. Windows 作业对象（`svsep.rs` 的 `job` 模块）—— 兜住任务管理器强杀：
             *      那种退出不会跑析构，只能靠「进程一死句柄被内核回收 → 作业里的进程
             *      一起死」。
             */
            if let tauri::RunEvent::Exit = event {
                note!("窗口关闭，正在收尾…");
                let _ = app.try_state::<Arc<ipc::AppState>>();
            }
        });
}

/* ────────────────────────────────── 路径 ────────────────────────────────── */

/// 程序的路径布局。两种形态共用一套代码：
///
/// - **绿色版**（整个目录解压，双击 bat）：根目录就是解压出来的那一层，
///   数据写在 `<根>/app/data/`。
/// - **安装版**（MSI/NSIS 装到 Program Files）：界面和工具在 Tauri 的
///   resource_dir 下，而**那里是只读的** —— 配置必须写到用户目录，
///   否则保存设置会失败（Program Files 需要管理员权限才能写）。
/// 只读数据目录（`resources.json`、`pinyin.json`，随包分发不改）是
/// `<root>/app/data/` —— 由 `AppState` 自己拼（`server/mod.rs::data_dir()`），
/// 所以这里不需要一个同名的访问器。
#[derive(Clone)]
pub struct AppPaths {
    /// 只读资源根目录（含 `app/web/`、`app/data/`、`tools/`）
    pub root: PathBuf,
    /// 可写目录（`config.json` 写这里）
    pub writable: PathBuf,
    /// 是否安装版 —— 决定出错提示怎么写
    pub installed: bool,
}

/// 定位程序的路径布局。
///
/// 查找顺序（先命中先赢）：
///   1. Tauri 的 `resource_dir()` —— 安装版走这条
///   2. 从 exe 所在目录往上找 —— 绿色版走这条
///   3. 当前工作目录往上找 —— 开发时直接 `cargo run` 走这条
///
/// 判据是 `app/web/index.html` 存在（前端的入口，两种形态都在）。
///
/// ⚠️ 这个文件**同时**是 `tauri.conf.json` 的 `frontendDist` 源目录 ——
/// 也就是说它既随包分发（Tauri 要把它嵌进 exe），也留在磁盘上当哨兵。
/// 别再把它改成「只嵌进 exe、磁盘上不留」的纯 `frontendDist` 布局：
/// 那样 `resolve_paths` 就找不到根目录了（`tools/`、`app/data/` 全在它旁边）。
fn resolve_paths(resource_dir: Option<PathBuf>) -> Option<AppPaths> {
    let has_web = |d: &Path| d.join("app").join("web").join("index.html").is_file();

    /*
     * 判据不能只看「resource_dir 里有没有 app/web」。
     *
     * 绿色版运行时，Tauri 的 resource_dir() 返回的**就是 exe 所在目录** ——
     * 那里当然有 app/web/index.html，于是会被误判成安装版，
     * 配置就被写到 %APPDATA% 去了，而绿色版应该写在程序旁边的 app/data/。
     *
     * 所以真正的判据是「程序目录能不能写」：
     *   - 能写（绿色版、解压在用户目录）→ 配置放旁边，整个目录可以拷着走
     *   - 不能写（装在 Program Files）→ 配置放 %APPDATA%
     */
    let mut root_from_resource: Option<PathBuf> = None;
    if let Some(rd) = resource_dir {
        if has_web(&rd) {
            root_from_resource = Some(rd);
        }
    }
    if let Some(rd) = root_from_resource {
        let local_data = rd.join("app").join("data");
        if is_writable(&local_data) {
            return Some(AppPaths {
                writable: local_data,
                root: rd,
                installed: false,
            });
        }
        return Some(AppPaths {
            writable: user_data_dir(),
            root: rd,
            installed: true,
        });
    }

    // ── 2/3. 绿色版 / 开发：从 exe 和 cwd 往上找 ──
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bases.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }

    for base in bases {
        let mut dir: Option<&Path> = Some(base.as_path());
        let mut depth = 0;
        while let Some(d) = dir {
            if has_web(d) {
                return Some(AppPaths {
                    root: d.to_path_buf(),
                    // 绿色版：配置就放在程序旁边，便于整个目录拷着走
                    writable: d.join("app").join("data"),
                    installed: false,
                });
            }
            if depth >= 5 {
                break;
            }
            dir = d.parent();
            depth += 1;
        }
    }
    None
}

/// 目录能不能写。
///
/// 判据是「真的建一个文件试试」而不是看只读属性 ——
/// Program Files 下 ACL 才是拦路虎，只读位看不出来。
fn is_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 安装版的可写目录：`%APPDATA%\<identifier>\`
fn user_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("com.qingmu.vocalworkstation")
}

/* ────────────────────────────────── 失败提示 ────────────────────────────────── */

/// 出错日志的完整路径，给用户看的。
///
/// 日志写在**可写目录**下（绿色版 `<根>\app\data\`，安装版
/// `%APPDATA%\com.qingmu.vocalworkstation\`），不是固定的 `data\` ——
/// 文案里写死 `data\desktop-error.log` 对安装版是错的。
fn error_log_hint() -> String {
    resolve_paths(None)
        .map(|p| p.writable.join("desktop-error.log"))
        .map(|p| crate::platform::clean_path(&p))
        .unwrap_or_else(|| "程序数据目录下的 desktop-error.log".to_string())
}

fn show_error(app: &tauri::AppHandle, message: &str) {
    // 日志路径由这里统一附在提示后面 —— `dist/index.html` 只负责渲染，
    // 它不知道路径（绿色版与安装版不同），所以那边不要写死。
    let message = format!("{message}\n详细信息见：{}", error_log_hint());
    let message = message.as_str();
    // 写日志
    let dir = resolve_paths(None)
        .map(|p| p.writable)
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("desktop-error.log"))
    {
        use std::io::Write;
        let _ = writeln!(f, "{message}");
    }
    note!("错误：{message}");

    // 开一个窗口把原因显示出来（走内嵌的起始页，原因通过 hash 传过去）。
    // `build()` 失败时不 panic —— 那时候真正的原因已经写进日志了。
    if let Ok(w) = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title(WINDOW_TITLE)
        .inner_size(720.0, 420.0)
        .center()
        .build()
    {
        let encoded = urlencode(message);
        let _ = w.eval(&format!("location.hash='{encoded}';location.reload();"));
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
