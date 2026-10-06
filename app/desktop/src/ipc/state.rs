//! 状态聚合 + 任务表。
//!
//! 这两样以前住在 `server/mod.rs` 与 `server/simple.rs` 里。搬过来是**必须**的：
//! `AppState` 与 `JobTable` 是「后端」的东西，而 HTTP 那一层要整个删掉 ——
//! 把它们留在被删的目录里就等于删功能。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};

use super::Cmd;

/* ══════════════════════════════════ 任务表 ══════════════════════════════════ */

/// 任务表 —— 长任务（转换 / 下载 / 分离 / 扒谱）共用的一份进度记录。
///
/// `tx` 是给**实时订阅**用的广播通道：任何任务状态变化都往里发一份**完整快照**。
/// 以前订阅这个通道的是 SSE（`GET /api/jobs/{id}/stream`），现在换成
/// [`crate::ipc::jobs::job_watch`] 的 `tauri::ipc::Channel` —— 通道换了，机制没换。
///
/// 用**一个全局广播**而不是「每个任务一个通道」：任务数少、订阅者更少，
/// 按 id 过滤的代价可以忽略，换来的是不用维护通道的创建与销毁。
///
/// ⚠️ 推的是**完整快照而不是增量**：订阅者偶尔卡顿丢一条也不影响正确性，
/// 所以容量给 256 就够，不必追求不丢消息。
pub struct JobTable {
    pub items: BTreeMap<String, Value>,
    pub seq: u64,
    pub tx: tokio::sync::broadcast::Sender<Value>,
}

impl Default for JobTable {
    fn default() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Self { items: BTreeMap::new(), seq: 0, tx }
    }
}

impl JobTable {
    /// 广播一份任务快照
    pub fn publish(&self, job: &Value) {
        let _ = self.tx.send(job.clone());
    }
}

/* ══════════════════════════════════ 全局状态 ══════════════════════════════════ */

/// 全局共享状态。极简 —— 只有真的需要跨命令共享的东西才放进来。
///
/// 在 `main.rs` 的 setup 里 `app.manage()` 进来，命令用 `tauri::State<Arc<AppState>>` 取。
pub struct AppState {
    /// 只读资源根目录（含 app/data/、tools/、app/web/）
    pub root: PathBuf,
    /// 可写目录 —— 配置与下载产物写这里。
    /// 绿色版就是 `app/data/`；安装版在 `%APPDATA%` 下。
    pub writable: PathBuf,
    /// 是否安装版 —— 界面上给恢复提示时用得上
    pub installed: bool,
    /// 配置
    pub config: Mutex<Value>,
    /// 任务表
    pub jobs: Mutex<JobTable>,
    /// 离线音轨分离服务（Python 子进程）。见 `crate::svsep`。
    pub svsep: crate::svsep::Svsep,
    /// 外部工具探测的缓存：`(editors, tools, 算完的时刻)`。
    ///
    /// **为什么要缓存**：`detect_tools` 会真的 spawn `yt-dlp --version` /
    /// `python --version`，还要逐段扫 `PATH`，机器忙时一次 2~7 秒；首屏的
    /// `get_state` 要是每次现算，用户看到的就是「启动卡死/白屏很久」。
    /// 现在启动时在后台线程预热一次，之后的调用直接读缓存。
    pub probe_cache: Mutex<Option<(Vec<Value>, Value, Instant)>>,
}

/// 探测结果的缓存时长。工具是随程序打包的，装好之后基本不变；
/// 用户手动补回 `tools/` 目录后最多等一分钟，或者点界面上的「重新检测」
/// （`tools_detect` 走 `probe_cached(true)` 绕过缓存）。
const PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

impl AppState {
    pub fn new(paths: crate::AppPaths) -> Arc<Self> {
        // 可写目录可能还不存在（首次运行安装版），先建出来
        let _ = std::fs::create_dir_all(&paths.writable);
        let config = super::config_file::load_config(&paths.writable);
        /* 运行时落点要在建 `Svsep` **之前**定下来 —— `runtime_base()` 是进程级的，
           凡是拼运行时路径的地方都读它（见那个函数的注释）。
           ⚠️ 安装版默认落点是程序目录 = `Program Files` = 普通权限写不进去，
           4.7 GB 解压必然失败 —— MSI 用户「音轨分离用不了」有这一半原因。 */
        crate::svsep::init_runtime_base(
            &paths.root,
            &paths.writable,
            paths.installed,
            config
                .get("svsepRuntimeDir")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        /* 显卡加速（DirectML）按配置生效：改 `._pth` 里那一行 + 六轨补丁。
           ⚠️ 必须在 `init_runtime_base` **之后** —— 那两个文件都在运行时目录里，
           而运行时目录刚刚才定下来。 */
        crate::svsep::apply_dml(
            &paths.root,
            config.get("svsepDml").and_then(Value::as_str).unwrap_or("auto"),
            config
                .get("svsepDmlSix")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        let svsep = crate::svsep::Svsep::new(
            paths.root.clone(),
            paths.writable.clone(),
            paths.installed,
        );
        /*
         * ⚠️ 预热**不在这里**做，由调用方（`main.rs` 的 setup）起线程。
         * 这里原来是 spawn 一个线程自己预热 —— 那是「serve() 建状态、进程只有一条路」
         * 时代的写法。现在本函数在 setup 里被同步调用，而 setup 跑在主线程上，
         * 任何耗时动作都该由调用方显式决定要不要异步做。
         */
        Arc::new(Self {
            root: paths.root,
            writable: paths.writable,
            installed: paths.installed,
            config: Mutex::new(config),
            jobs: Mutex::new(JobTable::default()),
            svsep,
            probe_cache: Mutex::new(None),
        })
    }

    /// 外部工具探测（带缓存）。
    ///
    /// `force = true` 绕过缓存现算一遍：界面上的「重新检测」必须能反映真实情况，
    /// 否则用户补回 `tools/` 目录后会被缓存骗一分钟。
    pub fn probe_cached(&self, force: bool) -> (Vec<Value>, Value) {
        if !force {
            if let Ok(guard) = self.probe_cache.lock() {
                if let Some((editors, tools, at)) = guard.as_ref() {
                    if at.elapsed() < PROBE_TTL {
                        return (editors.clone(), tools.clone());
                    }
                }
            }
        }
        // 注意：**不持锁**跑探测 —— 它要几秒，持锁会把并发调用全串在身后。
        // 代价是冷启动时可能有两三个线程同时探一遍，可以接受（结果一样，最后写的赢）。
        let editors = crate::tools::detect_editors();
        let tools = crate::tools::detect_tools(&self.root);
        if let Ok(mut guard) = self.probe_cache.lock() {
            *guard = Some((editors.clone(), tools.clone(), Instant::now()));
        }
        (editors, tools)
    }

    pub fn config_snapshot(&self) -> Value {
        self.config
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|_| json!({}))
    }

    /// 只读数据目录（app/data）—— 资源库、拼音词典。配置在 writable，见上。
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("app").join("data")
    }

    /// 外部工具目录（tools/）
    pub fn tools_dir(&self) -> PathBuf {
        self.root.join("tools")
    }
}

/* ══════════════════════════════════ 命令 ══════════════════════════════════ */

/// 首屏聚合状态 —— 取代旧的三条（`/api/state` + `/api/health` + `/api/tools/detect`）。
///
/// 前端启动时**只要这一次调用**就能把界面点亮：版本、路径、工具、格式表、
/// 配置（Cookie 已打码）全在里面。`lib/ipc.ts` 的 `bootstrap()` 用它。
///
/// 旧 `/api/health` 那几样（`pid` / `uptimeSec`）**没有搬过来**：
/// 它们是「跑在一个 HTTP 端口上」才需要的信息（哪个进程、活了多久），
/// 而 IPC 下前端和 Rust 在同一个进程里、同一个生命周期，问这些没有意义。
/// 版本号与运行环境本来就在这一份里（`version` / `platform`）。
#[tauri::command]
pub async fn get_state(st: super::St<'_>) -> Cmd {
    let cfg = st.config_snapshot();
    // 工具探测走缓存（见 `AppState::probe_cached`）：它要 spawn 进程 + 扫 PATH，
    // 机器忙时一次 2~7 秒，而前端首屏就在等这个。启动时已经在后台线程预热过一次。
    let (editors, tools) = st.probe_cached(false);
    Ok(json!({
        // 前端侧边栏要显示版本号 —— 以前它只能自己写死，改了 APP_VERSION 界面完全不跟
        "version": super::config_file::APP_VERSION,
        "author": super::config_file::AUTHOR_TAG,
        "formats": crate::libresvip::list_formats(&st.root),
        "editors": editors,
        "tools": tools,
        "transformOps": crate::data::transform_ops(),
        "audioFormats": crate::data::audio_formats(),
        "pinyin": crate::data::pinyin_summary(&st.root),
        // Cookie 打码后再给前端：这一份会进前端全局状态
        "config": super::config_file::mask_secrets(&cfg),
        "paths": {
            "root": st.root.to_string_lossy(),
            "outputDir": cfg.get("outputDir").cloned().unwrap_or(json!("")),
            "downloadDir": cfg.get("downloadDir").cloned().unwrap_or(json!("")),
            "toolsDir": st.tools_dir().to_string_lossy(),
        },
        "platform": crate::platform::node_platform_name(),
        "platformDesc": super::config_file::platform_desc(),
        // 安装版（Program Files）还是绿色版（解压即用）—— 界面给恢复提示时用得上
        "installed": st.installed,
    }))
}
