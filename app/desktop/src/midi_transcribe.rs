//! 人声转 MIDI —— GAME（<https://github.com/openvpi/GAME>）的 Windows 原生移植。
//!
//! # 和音轨分离长得很不一样，是故意的
//!
//! | | 音轨分离（`svsep.rs`） | 人声转 MIDI（这里） |
//! |---|---|---|
//! | 引擎 | Python HTTP 服务 | **进程内**，没有子进程 |
//! | 端口 | 17879 起找一个空闲的 | **不占端口** |
//! | 依赖 | 7.8 GB 运行时 | **零依赖**（借用上面那个运行时里的 ORT，或另下 78 MB 的 CPU 版）+ 364 MB 模型包 |
//!
//! 「起进程 → 轮询健康 → 转发 → 收尸」那一整套在那边是必需的（引擎是别人的
//! Python 程序），在这里全是白交的复杂度：推理就是一次函数调用。
//!
//! # 算子在哪
//!
//! **神经网络在 ONNX 图里，其余全在这儿**（见 `game::algo` 顶部的表）：
//! 波形切片、D3PM 采样环、边界解码、区间→时值、音符抽取、MIDI 写出。
//! 这就是「port」的实质 —— 不是把 Python 包进 exe，是把算法重写一遍。
//!
//! # 模型与许可
//!
//! 代码 MIT，**权重 CC BY-NC-SA 4.0（非商业）**。所以模型不进仓库、不随包
//! 分发，由用户在界面上点一下下载（`MODEL_URL`，我们自己托管在 123 云盘 CDN 上，
//! 内容与官方 release 那个 zip 逐字节相同）—— 和音轨分离「运行时进包、
//! 模型按需下」是同一条规矩。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::game::{algo, engine};

/// ONNX 权重包（opset 20，由上游 `deployment/exporter.py` 导出）。
///
/// ⚠️ 上游 release 的 `GAME-1.0-large.zip` 里是 **PyTorch 的 `model.pt`**，不是
/// ONNX —— 要用 ONNX 就得认准 `-onnx` 这几个包（v1.0.3 才发布的）。
///
/// # 为什么下的是自己的包，而不是上游 release
///
/// 内容一样（都是官方导出的那三个图），但**托管在国内**。实测 GitHub 的
/// release 资产在这台机器上根本下不动：`github.com` 与 `api.github.com` 直连
/// 都通（`curl -I` 200），可 release 资产会 302 跳到
/// `objects.githubusercontent.com`，跟过去之后 **TLS 握手直接失败**
/// （`curl: (35) schannel: failed to receive handshake`），带 `-L` 则是
/// `Failed to connect to github.com port 443: Timed out`（`final_code=000`）。
/// 这台机器上没有代理（`127.0.0.1:7890` connection refused）、环境变量里没有
/// `*_PROXY`、`hosts` 文件都不存在 —— 不是配置问题，是线路问题。
/// 音轨分离的运行时与模型早就放在 123 云盘 CDN 上了，模型包跟着一起走。
///
/// **两个包可以互换**：自建包沿用上游那个顶层目录名 `GAME-1.0.3-large-onnx/`，
/// 所以 `download_models` 里那个 `strip` 常量两边都命得中（见那里的注释）。
/// 自建包由 `tools\game-pack.ps1` 产出，白名单四个文件、每个都校验实测字节数。
///
/// ⚠️ 末尾那个 `#` 不要删：123 云盘直链的原始形状就是带尾巴的，去掉可能 404。
pub const MODEL_URL: &str =
    "https://1856610041.cdn.123clouddisk.com/1856610041/V-Synth-Studio/GAME-1.0.3-large-onnx.zip#";

/// 整包大小，只用来显示与校验「下完没有」。实测值。
///
/// 上游那份是 361,619,205 B；自建包（`tools\game-pack.ps1`，`Optimal` 档）
/// 是 **364,093,888 B** —— 大 2.4 MB 是因为 .NET 的 deflate 比上游用的压得松，
/// 属正常。这个常量会作为「下完没有」的判据传给 `fetch_to_file`，
/// **换包就一定要同步改**，否则下到一半就报完成（或永远等不到那几字节）。
pub const MODEL_ZIP_BYTES: u64 = 364_093_888;
/// 解包后那几个文件的总大小（实测值，界面用来解释「下 347 MB、占 376 MB」）。
///
/// 这个数对应**上游**那份包（81,312,536 + 160,373,028 + 152,478,761 + 198 =
/// 393,794,532 —— 两边逐字节相同，所以自建包解出来也是这个数）。
pub const MODEL_BYTES: u64 = 393_794_532;

/// 下载 ONNX Runtime 动态库的地方。
///
/// 走 **onnxruntime 官方的 GitHub release**（CPU 版 zip），解包后只留
/// `lib/onnxruntime.dll` 与相邻的 `onnxruntime_providers_shared.dll`。
/// 为什么不装 PyPI 的 wheel、也不引 NuGet：前者要为了一个库拖进来 200 MB
/// 还得自己剖 wheel，后者给的是 `.targets` 而不是裸 DLL。
///
/// ⚠️ **实测这个 zip 是 78 MB，不是 14 MB**（曾经按「单文件大概十几 MB」猜过，
/// 猜错了）。它里面带 C/C++ 头文件、`onnxruntime.lib` 等一大堆用不上的东西 ——
/// 但官方没有「只给 DLL」的资产，所以这就是最小的一条路。
/// 装了音轨分离的用户**根本不会走到这里**（`runtime_dll` 会借它运行时里那一份），
/// 所以这 78 MB 只在「没装音轨分离、又想要这个功能」时才付。
pub const RUNTIME_URL: &str =
    "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-win-x64-1.23.2.zip";
/// 那个 zip 的实测大小（`onnxruntime-win-x64-1.23.2.zip`，CPU 版）。
pub const RUNTIME_ZIP_BYTES: u64 = 78_127_794;
/// 解包后真正要留下的那个 DLL 多大（实测 15,551,032 B，界面用来解释「78 MB 里
/// 我们只要这 15 MB」）。
pub const RUNTIME_DLL_BYTES: u64 = 15_551_032;

/* ⚠️ 这里**没有** `MODEL_FILES` —— 三个图的名字只有一份，在
   `game::engine::MODEL_FILES`（判「装齐了没」的是 `engine::missing_models`）。
   曾经在这儿写过一份 `pub const MODEL_FILES: [&str; 3] = engine::MODEL_FILES;`，
   结果两边都没人用、各报一次 dead_code，删掉。 */

// ---------------------------------------------------------------------------
// 下载地址（编译期常量 + 只给开发机的临时覆盖）
// ---------------------------------------------------------------------------

/// **只给开发机用的临时覆盖**（`VSS_MIDI_MODEL_URL` / `VSS_MIDI_RUNTIME_URL`）。
///
/// 与 `svsep.rs` 同一套规矩、同一个理由：链接是编译期常量，而「下载到一半
/// 按删除」「进度条卡在 99%」这类事**必须真下着才验得出来**。要是每次改常量
/// 重编，测完还得记得改回来 —— 漏一次就把本地文件路径发出去。
/// ⚠️ 发布版**不要设这两个变量**，设了就是拿本地文件当下载源。
#[cfg(debug_assertions)]
fn url_override(key: &str, default: &'static str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => default.to_string(),
    }
}
#[cfg(not(debug_assertions))]
fn url_override(_key: &str, default: &'static str) -> String {
    default.to_string()
}

/// 这一次真的要用的模型包地址（常量，或开发机用环境变量顶掉的那个）。
///
/// 只在 debug 构建里读环境变量 —— 发布版永远走 `MODEL_URL`。
fn model_url() -> String {
    url_override("VSS_MIDI_MODEL_URL", MODEL_URL)
}

/// 这一次真的要用的 ONNX Runtime 地址。
fn runtime_url() -> String {
    url_override("VSS_MIDI_RUNTIME_URL", RUNTIME_URL)
}

/// 一个地址的**境内镜像**（`ghfast.top` 前缀），拿不到就回 `None`。
///
/// 主地址是 GitHub release：境内直连经常是「开头连得上、下到一半断」或者干脆超时。
/// 音轨分离那边的 Python 后端早就在用同一招（`CHIXIAOYANG_GITHUB_DOWNLOAD_MIRROR`，
/// 默认就是这个域名），这里只是给这 78 MB 的包补一条退路。
/// ⚠️ **只加退路、不换主地址** —— 镜像自己也会挂，能直连的人不该被它拖累。
fn mirrored(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    Some(format!("https://ghfast.top/https://github.com/{rest}"))
}

/// 这个包按顺序要试的地址：先官方，再镜像。
fn runtime_urls() -> Vec<String> {
    let primary = runtime_url();
    match mirrored(&primary) {
        Some(m) => vec![primary, m],
        None => vec![primary],
    }
}

/// 依次试 `urls`，第一个成功就返回；全失败就把最后一条错误抛出去。
///
/// ⚠️ 与 `fetch_to_file` 的分工要清楚：**它**负责「同一个地址重试 5 轮 + 删掉半个文件」
/// （抖动用那个），**这里**负责「这个源根本不通就换一个源」。两层都别省：
/// 只重试同一地址会让被墙的用户白等五轮，只换源会让一次网络抖动直接劝退。
async fn fetch_first_ok(
    urls: &[String],
    out: &Path,
    ctl: &crate::svsep::DownloadCtl,
    on_progress: &(impl Fn(u64, Option<u64>, crate::svsep::Stage) + Send + Sync),
) -> Result<u64, String> {
    let mut last = String::from("没有可用的下载地址");
    for (i, url) in urls.iter().enumerate() {
        if ctl.cancelled() {
            return Err("已停止".into());
        }
        if i > 0 {
            crate::log_line(&format!("MIDI 运行时：上一个地址失败，换镜像重试：{url}"));
        }
        match crate::svsep::fetch_to_file(url, out, None, ctl, |got, total, stage| {
            on_progress(got, total, stage)
        })
        .await
        {
            Ok(n) => return Ok(n),
            Err(e) => last = e,
        }
    }
    Err(last)
}

// ---------------------------------------------------------------------------
// 推理方式（自动 / GPU / CPU）
// ---------------------------------------------------------------------------

/// 推理方式。界面上的三选一，落到盘上就是这三个字符串。
///
/// `auto` 与 `cpu` 在**今天**完全等价（`engine::ort_session` 只在 `gpu` 时挂
/// CUDA，其余一律纯 CPU）—— 留着 `auto` 是为了将来接 CoreML / DirectML 时能
/// 把「让引擎自己挑」与「我就要 CPU」分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Device {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

impl Device {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "gpu" => Self::Gpu,
            "cpu" => Self::Cpu,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

/// 设置文件：`<可写>/midi/midi_settings.json`。
///
/// ⛔ 和音轨分离那个 `<可写>/svsep/data/inference_settings.json` **是两个文件，
/// 别想着共用**：那个文件是 Python 的 `inference_settings.py::_SETTINGS_PATH`
/// 在管，键名与合法值都由它定义；我们写进去的东西它不认识，反过来也一样。
pub fn settings_file(writable: &Path) -> PathBuf {
    data_dir(writable).join("midi_settings.json")
}

/// 读盘上的推理方式。没有文件、文件坏了、值认不出，一律回 `auto`。
pub fn read_device(writable: &Path) -> Device {
    std::fs::read_to_string(settings_file(writable))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("device").and_then(Value::as_str).map(Device::parse))
        .unwrap_or_default()
}

/// 写盘上的推理方式。
pub fn write_device(writable: &Path, dev: Device) -> Result<(), String> {
    let path = settings_file(writable);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{e}"))?;
    }
    let body = serde_json::to_string_pretty(&json!({ "device": dev.as_str() }))
        .map_err(|e| format!("序列化推理方式失败：{e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("写推理方式失败：{e}"))
}

/// CUDA 那边探到的东西。探成功之后**常驻**（见 `cuda_info` 的注释）。
#[derive(Debug, Clone)]
pub struct CudaInfo {
    /// 现在到底能不能用 CUDA。
    pub available: bool,
    /// 给界面直接显示的中文。
    ///
    /// ⛔ 信息**全塞在这一句里**，别再单开一个「组件目录」字段：单开的那个字段
    /// 没有读者（会吃 `field is never read` 警告），而这段中文是**原样显示给
    /// 用户**的，路径写在这儿才真的有人看。
    pub detail: String,
}

/// 当前推理方式 + CUDA 能不能用，给 `/api/midi/status` 一并发出去。
pub struct DeviceStatus {
    pub device: Device,
    pub cuda_ok: bool,
    pub cuda_detail: String,
    /// 用户选了 GPU 但这台机器用不了时的补救话术（空串 = 没什么好说的）。
    pub note: String,
}

pub fn device_status(writable: &Path, dll: Option<&Path>) -> DeviceStatus {
    let device = read_device(writable);
    let info = cuda_info(dll);
    let note = if device == Device::Gpu && !info.available {
        format!("现在这一台用不了 GPU：{}。先按 CPU 跑。", info.detail)
    } else {
        String::new()
    };
    DeviceStatus { device, cuda_ok: info.available, cuda_detail: info.detail, note }
}

/// 探测结果的缓存：成功的一直留，失败的**最多留 [`FAILED_TTL`]**。
static CUDA_PROBE: std::sync::Mutex<Option<(std::time::Instant, CudaInfo)>> =
    std::sync::Mutex::new(None);

/// 失败结果能留多久。见 [`cuda_info`] 里那段「为什么要缓存、为什么两档不一样」。
const FAILED_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// 这台机器现在能不能用 CUDA 跑 ONNX Runtime。
///
/// ⚠️ **两档缓存，别改成一样**：`/api/midi/status` 是前端每 2 秒轮询的接口，
/// 而探一次 CUDA 会话在**没有 N 卡**的机器上要 1 秒多（真会去初始化 CUDA，
/// 拿回 `CUDA failure 35: CUDA driver version is insufficient`）。每 2 秒来一遍
/// 就是白白烧掉一半核。
///
/// - **成功** ⇒ 永久缓存：那几百 MB 的 `cublasLt64_12.dll` 已经进了进程，
///   不会退回去，也没有再探的意义。
/// - **失败** ⇒ 只留 [`FAILED_TTL`]：用户可能正开着这个界面去装音轨分离，
///   装完不重启就该能解锁；30 秒够短，也够挡住轮询。
pub fn cuda_info(dll: Option<&Path>) -> CudaInfo {
    if let Ok(cache) = CUDA_PROBE.lock() {
        if let Some((at, info)) = cache.as_ref() {
            let fresh = info.available || at.elapsed() < FAILED_TTL;
            if fresh {
                return info.clone();
            }
        }
    }
    let info = probe_cuda(dll);
    if let Ok(mut cache) = CUDA_PROBE.lock() {
        *cache = Some((std::time::Instant::now(), info.clone()));
    }
    info
}

fn probe_cuda(dll: Option<&Path>) -> CudaInfo {
    let dll = match dll {
        Some(p) => p,
        None => {
            return CudaInfo {
                available: false,
                detail: "还没找到 ONNX Runtime 动态库，先下运行库".into(),
            }
        }
    };
    /* ⛔⛔ **必须先把 ONNX Runtime 本体加载进来，否则下面那句 `is_available()`
       会让整个进程 panic**。这是真机上抓到的：
       `thread 'tokio-rt-worker' panicked at ort-2.0.0-rc.13/src/lib.rs:234:
        Failed to load ONNX Runtime dylib: MissingApi { path: "onnxruntime.dll" }`

       原因在 ort 内部：任何 ORT 调用（含 `ExecutionProvider::is_available`）都会
       走 `ort::api()`，而它第一次被调用时是**惰性初始化** —— 如果此前没人调过
       `ort::init_from(...).commit()`，它就退回去找**裸文件名** `onnxruntime.dll`，
       PATH 里当然没有。`transcribe` 里那句 `load_runtime(dll)` 来得太晚：
       `/api/midi/status` 是首页一进就轮询的接口，**它会先到这儿**。

       症状极具误导性：**整个 HTTP 连接被直接掐断**（curl 报
       `Empty reply from server`、`HTTP 000`），没有 500、没有 JSON、没有日志，
       `Get-Process` 里进程还活得好好的（那是 tokio 的 worker 线程 panic 了，
       不是进程崩了）—— 看起来像「路由没注册」或「网络出问题」。
       ⇒ 遇到「某条 IPC 命令**永远不返回**」（前端 invoke 一直挂着、别的命令却正常），
        去翻程序日志（`<可写目录>/app.log`，或从控制台起时看 stderr）—— 别盯着命令注册表，
        注册表里明明有它、代码也没写错。

       加载一次就够（`ort::init_from` 内部是 `OnceLock`，重复调用无害），所以
       这里用 `RUNTIME_LOADED` 只做一次；`/api/midi/status` 是 2 秒轮询的。 */
    static RUNTIME_LOADED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if RUNTIME_LOADED.get().is_none() {
        if let Err(e) = crate::game::engine::load_runtime(dll) {
            return CudaInfo { available: false, detail: e };
        }
        let _ = RUNTIME_LOADED.set(());
    }

    /* ⚠️ **那 12 个 CUDA 组件 dll 分在四个目录里**，不是一个：
         cudart64_12.dll   → nvidia/cuda_runtime/bin/
         cublas{,Lt}64_12.dll → nvidia/cublas/bin/
         cufft64_11.dll    → nvidia/cufft/bin/
         8 个 cudnn*_9.dll → nvidia/cudnn/bin/

       ⇒ **不能指望 `ort::ep::cuda::preload_dylibs`**：它只吃**一个** CUDA 根目录
       和**一个** cuDNN 根目录，然后把 `CUDA_DYLIBS`（4 个）/ `CUDNN_DYLIBS`（8 个）
       里的每个名字都 `root.join(name)` 去加载 —— 而真机上这 4 个 CUDA dll
       **根本不在同一个目录**。实测（2026-10-03）只喂第一个目录时它会这样报：
       `Failed to preload `cublasLt64_12.dll`: LoadLibraryExW failed
        （…\nvidia\cuda_runtime\bin）` —— 它拿 `cuda_runtime\bin` 去找 `cublasLt`，
       当然找不到。

       ✅ **和 Python 侧一模一样的做法：把这四个目录加进进程的 `PATH`。**
       `onnxruntime_providers_cuda.dll` 内部是 `LoadLibraryExW` 找依赖，而
       `LoadLibraryExW`（不带 `LOAD_LIBRARY_SEARCH_*` 标志时）**会搜 `PATH`**，
       所以 CUDA EP 真去加载自己那些依赖时就能找到。
       （`svsep` 那边的 Python 用的是 `os.add_dll_directory`，同一个道理。） */
    static NVIDA_BIN_DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    let dirs = NVIDA_BIN_DIRS.get_or_init(|| {
        let mut out: Vec<PathBuf> = Vec::new();
        for name in [
            "cudart64_12.dll",
            "cublas64_12.dll",
            "cufft64_11.dll",
            "cudnn64_9.dll",
        ] {
            if let Some(d) = find_dylibs_dir(dll, &crate::tools::dll(name.trim_end_matches(".dll")))
            {
                if !out.contains(&d) {
                    out.push(d);
                }
            }
        }
        out
    });

    /* 只在第一次探的时候动 PATH（`/api/midi/status` 是 2 秒轮询的）。
       加入的是**四条**路径，不是一条 —— 别图省事只取 `find_dylibs_dir` 的第一个结果。

       ⚠️ 手拼 `;`，别用 `std::env::join_paths`：后者见到路径里含 `;` 会直接报
       `InvalidInput`，而这里四条路径全是进程外的东西，不值当为它写一条错误分支。 */
    static PATH_PATCHED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if !dirs.is_empty() && PATH_PATCHED.get().is_none() {
        let mut patched = std::env::var("PATH").unwrap_or_default();
        for d in dirs {
            let d = d.to_string_lossy();
            if !patched.to_lowercase().contains(&d.to_lowercase()) {
                patched = format!("{d};{patched}");
            }
        }
        std::env::set_var("PATH", patched);
        let _ = PATH_PATCHED.set(());
    }
    let found = dirs.iter().map(|d| crate::platform::clean_path(d)).collect::<Vec<_>>();

    /* ⛔⛔ **`is_available()` 本身不足以当判据** —— 它只回答「这份 ORT 构建里
       编进了 CUDA provider 没有」，**与这台机器有没有 N 卡无关**。实测（2026-10-03，
       本机是 **AMD RX 580**）：把四个目录加进 `PATH` 之后它回 **`Ok(true)`** ——
       于是「默认锁死」这条需求当场失效，A 卡用户也会看到 GPU 那格是亮的。

       ✅ 真判据只有一条：**真拿 CUDA 建一个会话看看**。
       `SessionBuilder::with_execution_providers([CUDA.error_on_failure()])` 会
       让 ORT 当场去初始化 CUDA EP：没有 N 卡 / 驱动太老 / 组件缺失都会在这一步
       返回 Err，而这正是我们要的那句话（它的原文还会原样显示在界面上）。
       `error_on_failure()` 在这里是必须的 —— 默认的 `fail_silently` 会把
       「CUDA 没装上」吞掉、悄悄给一个 CPU 会话。

       图用**内存里拼的最小 ONNX**（一个 `Identity`，见 `mini_onnx`）：不依赖
       `app/data/game/models` 里那 376 MB 模型在不在 —— **GPU 那一格该不该亮，
       跟用户下没下模型是两回事**（状态接口在没模型时也要答得出来）。 */
    use ort::ep::ExecutionProvider as _;
    match ort::ep::CUDA::default().is_available() {
        Ok(true) => {}
        Ok(false) => {
            return CudaInfo {
                available: false,
                detail: "这份 ONNX Runtime 里没有 CUDA provider（官方 CPU 包）".into(),
            }
        }
        Err(e) => {
            return CudaInfo {
                available: false,
                detail: format!("问 ONNX Runtime「有没有 CUDA」时出错：{e}"),
            }
        }
    }
    /* ⚠️ 这里**不能用 `?` 或 `and_then` 串**：`with_execution_providers` 失败时
       回的是 `ort::Error<SessionBuilder>`（它能把 builder 还给你），与后面那句的
       `ort::Error<()>` 不是一个类型。手工两步、各自 `map_err` 成字符串最省事。 */
    let probe = ort::session::Session::builder().map_err(|e| e.to_string()).and_then(|b| {
        b.with_execution_providers([
            ort::ep::CUDA::default().build().error_on_failure(),
            ort::ep::CPU::default().build(),
        ])
        .map_err(|e| e.to_string())
        .and_then(|mut b| b.commit_from_memory(&mini_onnx()).map_err(|e| e.to_string()))
    });
    match probe {
        Ok(_) => CudaInfo {
            available: true,
            /* 这一条会**原样显示在界面上**（`device.cuda.detail`），所以把找到的
               目录带上：解锁之后万一建会话还是失败，用户手里得有能对得上的线索。 */
            detail: if found.is_empty() {
                "这台机器的 ONNX Runtime 能用 CUDA（组件走的是系统里那一套）".into()
            } else {
                format!("这台机器能用 CUDA 跑 ONNX Runtime，组件来自 {}", found.join("、"))
            },
        },
        Err(e) => CudaInfo {
            available: false,
            detail: format!("这台机器上 CUDA 建不出会话（多半是没有 N 卡或驱动太老）：{e}"),
        },
    }
}

/// 一张**内存里拼的最小 ONNX 图**：`1×1 float` → `Identity` → `1×1 float`。
///
/// 只用来试「CUDA 能不能建出会话来」（见 `probe_cuda`）—— 所以刻意不碰磁盘上
/// 那 376 MB 的模型，也不做任何计算。
///
/// 手写这十几个字节是因为**没有 `onnx` crate 可依赖**（为这一件事引一个依赖不划算），
/// 而 protobuf 的线格式对这么小的图是能手写的：每个字段都是
/// `(field_number << 3) | wire_type`。
/// 结构：`ModelProto{ ir_version=7(1,varint), opset_import(8){version=13(2,varint)},
/// graph(7){ name(2,LEN), node(1){ op_type="Identity"(4), input=["x"](1),
/// output=["y"](2) }, input(5){ name="x", type(1){ tensor_type(1){ elem_type=1(1,varint),
/// shape(2){ dim(1){ dim_value=1(1,varint) } } } } },
/// output(2){ name="y", type(1){ tensor_type(1){ elem_type=1, shape(2){ dim(1){…=1} } } } } } }`
fn mini_onnx() -> Vec<u8> {
    /// protobuf 的 varint。
    fn vi(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }
    /// 一个 LEN 字段（tag + 长度 + 内容）。
    fn ld(field: u64, body: &[u8], out: &mut Vec<u8>) {
        vi((field << 3) | 2, out);
        vi(body.len() as u64, out);
        out.extend_from_slice(body);
    }
    /// 一个 varint 字段。
    fn vf(field: u64, v: u64, out: &mut Vec<u8>) {
        vi(field << 3, out);
        vi(v, out);
    }
    /// 一串 UTF-8 的 LEN 字段。
    fn ss(field: u64, s: &str, out: &mut Vec<u8>) {
        ld(field, s.as_bytes(), out);
    }
    /// `TypeProto` 的字节：`tensor_type(1){ elem_type=FLOAT(1), shape(2){ dim(1){ dim_value=1 } } }`。
    ///
    /// ⛔ 这个 `[1]` 形状是**必须的**，而且**极易写错**（两种写法都会产出「能解析、但 ORT 拒收」的
    /// 字节，症状一模一样）：
    /// - 少了 `shape` → ORT 报 `Tensor does not have type information.`；
    /// - 形状里的 `dim_value` 丢了 → 变成动态维度，图在法律上仍然成立。
    ///
    /// 更坑的是**返回值**：调用方必须把它包成 `ValueInfoProto.type`，而那是**字段 2**
    /// （`ValueInfoProto{ name = 1, type = 2 }`）。写成一（`ld(1, …)`）会拼出
    /// 「两个 `name`、没有 `type`」的 ValueInfoProto —— 手写的线格式自检测试**照样全绿**
    /// （它拆出来的就是两个字段 1），只有真拿 ONNX Runtime 建一次会话才现形。
    fn tensor_type() -> Vec<u8> {
        let mut shape_dim = Vec::new();
        vf(1, 1, &mut shape_dim); // dim_value = 1
        let mut shape = Vec::new();
        ld(1, &shape_dim, &mut shape); // shape.dim[0] = {dim_value: 1}
        let mut tensor = Vec::new();
        vf(1, 1, &mut tensor); // elem_type = FLOAT
        ld(2, &shape, &mut tensor); // shape
        let mut out = Vec::new();
        ld(1, &tensor, &mut out); // TypeProto.tensor_type
        out
    }

    let mut node = Vec::new();
    ss(1, "x", &mut node); // input
    ss(2, "y", &mut node); // output
    ss(4, "Identity", &mut node); // op_type

    let mut input_vi = Vec::new();
    ss(1, "x", &mut input_vi);
    ld(2, &tensor_type(), &mut input_vi); // ValueInfoProto.type —— **字段 2**，不是 1
    let mut output_vi = Vec::new();
    ss(1, "y", &mut output_vi);
    ld(2, &tensor_type(), &mut output_vi);

    let mut graph = Vec::new();
    ss(2, "cuda-probe", &mut graph); // graph.name
    ld(1, &node, &mut graph); // node
    ld(11, &input_vi, &mut graph); // input
    ld(12, &output_vi, &mut graph); // output

    let mut model = Vec::new();
    vf(1, 7, &mut model); // ir_version = 7
    ld(7, &graph, &mut model); // graph
    let mut opset = Vec::new();
    vf(2, 13, &mut opset); // opset_import.version = 13
    ld(8, &opset, &mut model);
    model
}

/// 从 `dll` 往上逐层看，找那一层**下面的** `nvidia/`，再在它里面定位某个组件 dll，
/// 返回**装那个 dll 的目录**（`nvidia/<组件>/bin`，加进 `PATH` 用的就是它）。
///
/// **真实布局**（`pip install nvidia-*` 之后就是这样，一层不多一层不少）：
/// ```text
/// <可写>/midi/onnxruntime.dll                        ← 自己下的 ORT：本体在 `midi/` 根上
/// <音轨分离运行时>/runtime/Lib/site-packages/        ← 往上一层
///     nvidia/cublas/bin/cublas64_12.dll              ← 组件在 `nvidia/<组件>/bin/`
///     nvidia/cufft/bin/cufft64_11.dll
///     nvidia/cuda_runtime/bin/cudart64_12.dll
///     nvidia/cudnn/bin/cudnn64_9.dll …
///     onnxruntime/capi/onnxruntime.dll               ← 借用的就是这一个
/// ```
/// ⛔ **是「哪一层的下面有 `nvidia/`」，不是「哪一层叫 `nvidia`」** ——
/// 两个条件写成一样都不报错，只是永远找不到（这里就这么错过一次：
/// 从 ORT 本体往上走是 `capi → onnxruntime → site-packages → …`，
/// `site-packages` 自己叫 `site-packages`，而 `nvidia` 是它**下面**的一层，
/// 于是条件恒假 ⇒ 四个目录一个都找不到 ⇒ GPU 永远解不了锁）。
///
/// 返回**目录**而不是文件，是因为调用方要把它加进 `PATH`（四个组件分在四个
/// `bin/` 里，见 `probe_cuda`）。找不到就回 `None`（调用方据此判「没装音轨分离
/// 运行时」，再往下问 ORT 自己）。
fn find_dylibs_dir(dll: &Path, probe_name: &str) -> Option<PathBuf> {
    let mut dir = dll.parent();
    while let Some(d) = dir {
        /* 只认 `site-packages/nvidia` 这一种形状。
           ⛔ 别放宽成「任何含 probe_name 的目录」：`<可写>/midi/` 里如果用户自己
           放了 CUDA 版 ORT，它的依赖躺在系统目录、不在旁边，那样找出来的「路径」
           是假的，反而会把一个没有 dll 的目录塞进 PATH。 */
        if let Some(hit) = find_file(&d.join("nvidia"), probe_name, 3) {
            return hit.parent().map(Path::to_path_buf);
        }
        dir = d.parent();
    }
    None
}

/// 在 `dir` 底下（含 `dir` 自己）最多往下 `depth` 层找名为 `name` 的文件。
/// 找到第一个就回 —— Win32 的文件名不区分大小写，这里也按不区分处理。
fn find_file(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
        } else if e
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs.into_iter().find_map(|d| find_file(&d, name, depth - 1))
}

// ---------------------------------------------------------------------------
// 路径
// ---------------------------------------------------------------------------

/// 动态库与模型的家：`<可写目录>/midi/`。
///
/// 和音轨分离的 `<可写>/svsep/` 同一个道理 —— 绿色版落在 `app/data/`，
/// 安装版落在 `%APPDATA%`，随包内容只读、下下来的东西可写。
pub fn data_dir(writable: &Path) -> PathBuf {
    writable.join("midi")
}

/// ONNX Runtime 动态库的候选位置，按优先级：
///
/// 1. `<可写>/midi/onnxruntime.dll` —— 本功能自己下的那份，最干净；
/// 2. 音轨分离运行时里那份（`runtime/Lib/site-packages/onnxruntime/capi/`）——
///    **已经装着音轨分离的用户不用再下 15 MB**；
/// 3. `<root>/app/data/onnx/onnxruntime.dll` —— 随包分发的位置，留给以后
///    把 ORT 直接打进安装包（体积可以接受，就不必让任何用户等这一下）。
///
/// debug 构建可以用 `VSS_MIDI_DLL` 强行指定，和 `svsep.rs::url_override` 同一
/// 套规矩：**发布版不认环境变量**。
pub fn runtime_candidates(root: &Path, writable: &Path) -> Vec<PathBuf> {
    #[cfg(debug_assertions)]
    if let Ok(p) = std::env::var("VSS_MIDI_DLL") {
        if !p.trim().is_empty() {
            return vec![PathBuf::from(p)];
        }
    }
    vec![
        data_dir(writable).join(crate::tools::dll("onnxruntime")),
        crate::svsep::runtime_base(root)
            .join("runtime")
            .join("Lib")
            .join("site-packages")
            .join("onnxruntime")
            .join("capi")
            .join(crate::tools::dll("onnxruntime")),
        root.join("app")
            .join("data")
            .join("onnx")
            .join(crate::tools::dll("onnxruntime")),
    ]
}

/// 现在能用哪个动态库。一个都没有就是 `None`。
///
/// ⚠️ **优先挑「带 CUDA provider」的那一份，不是简单取候选里的第一个。**
///
/// 踩过的坑（2026-10，用户报「N 卡点不亮转 MIDI 的 GPU」）：界面提示「缺运行库」
/// 时用户下了那份 **78 MB 的官方 CPU 包**，它落在 `<可写>/midi/`、排候选第一；
/// 后来他为了 GPU 又装了音轨分离（运行时里那份 ORT 是 **GPU 构建**、能跑 CUDA），
/// 可候选顺序让 CPU 包继续遮蔽它 —— `probe_cuda` 于是永远回
/// 「这份 ONNX Runtime 里没有 CUDA provider（官方 CPU 包）」，GPU 那一格永远锁着。
///
/// 按**能力**挑而不是按下单顺序挑，这个遮蔽就不成立了。注意这不等于强制用 GPU：
/// 用户选的推理方式（`device`）照旧说了算，这里只是别让「手里明明有一份能跑 CUDA 的」
/// 被一份跑不了的顶掉。
pub fn runtime_dll(root: &Path, writable: &Path) -> Option<PathBuf> {
    let existing: Vec<PathBuf> = runtime_candidates(root, writable)
        .into_iter()
        .filter(|p| p.is_file())
        .collect();
    existing
        .iter()
        .find(|p| has_cuda_provider(p))
        .cloned()
        .or_else(|| existing.into_iter().next())
}

/// 这份 ORT 旁边有没有 CUDA provider（`onnxruntime_providers_cuda.dll`）。
///
/// 这是**静态分辨「官方 CPU 包」与「GPU 构建」**的唯一办法：两个 zip 里的
/// `onnxruntime.dll` 同名，只有 GPU 构建多出这几个 provider dll
/// （音轨分离那份在 `onnxruntime/capi/` 下，实测有 cuda / tensorrt / shared）。
fn has_cuda_provider(dll: &Path) -> bool {
    dll.parent()
        .map(|d| d.join(crate::tools::dll("onnxruntime_providers_cuda")).is_file())
        .unwrap_or(false)
}

/// 模型目录：优先用现成的那一份（可写目录里下好的，或随包的）。
pub fn models_dir(root: &Path, writable: &Path) -> PathBuf {
    engine::resolve_model_dir(root, writable)
}

/// 还缺哪几个模型文件。
pub fn missing_models(root: &Path, writable: &Path) -> Vec<&'static str> {
    engine::missing_models(&models_dir(root, writable))
}

/// `/api/midi/status` 的正文。前端靠它决定显示「下载模型」还是「开始扒谱」。
pub fn status(root: &Path, writable: &Path) -> Value {
    let dll = runtime_dll(root, writable);
    let missing = missing_models(root, writable);
    /* 引擎这一刻用的模型**在哪儿**：`"downloaded"` / `"bundled"` / `"local"`。
       ⚠️ 别想在这里区分「这是下下来的还是随手放进去的」—— **分不出来**。
       绿色版 `writable == <root>/app/data`，两层是**同一个绝对路径**，一个目录同时
       扮演「下载落点」和「随包层」，任何按路径或按目录状态的判据都会误报。
       所以只报「引擎在用哪个路径」，界面据此说人话：
         · `downloaded` —— 用的是 `<可写>/game/models`（下载物就落这儿）
         · `bundled`    —— 用的是 `<root>/app/data/game/models`，**且它不是**下载落点
           （只可能出现在安装版：可写目录在 `%APPDATA%`）。这时点「删除依赖」不会
           动它，状态还是「就绪」—— 界面**必须**写清，否则用户以为按钮坏了（真报过）
         · `local`      —— 两层同一路径。没有第二层可回落，所以那边什么都不用说
       ⛔ 别在前端按 `dir` 的**尾巴**猜：三种情况的路径都以 `\game\models` 结尾。 */
    let dirs = engine::model_dirs(root, writable);
    let bundled = dirs[1].clone();
    let active = models_dir(root, writable);
    let models_origin = if active != bundled {
        "downloaded"
    } else if bundled == dirs[0] {
        "local"
    } else {
        "bundled"
    };
    /* 已经下了一半的包（`runtime.part` / `game-models.part`）。
       ⚠️ 这两个数**只是给用户看的**（「上次没下完，还剩 200 MB 在盘上」），
       不参与任何判断 —— 这个包不支持续传，`.part` 只会在下次下载时被覆盖。 */
    let data = data_dir(writable);
    let leftover = |name: &str| {
        std::fs::metadata(data.join(name))
            .map(|m| m.len())
            .unwrap_or(0)
    };
    /* 推理方式与 CUDA 可用性。`device` 是**盘上的设置**（用户选的那个），
       `cuda.ok` 是**这台机器现在能不能用**，两者可以互相矛盾（选了 GPU 但没装
       音轨分离）—— 界面必须照 `cuda.ok` 决定锁不锁那一格，别照 `device`。 */
    let ds = device_status(writable, dll.as_deref());
    json!({
        "runtime": {
            "ready": dll.is_some(),
            "dll": dll.as_ref().map(|p| crate::platform::clean_path(p)),
            // 用户在下模型之前**不需要**先下这个：找不到时会自己张罗。
            // `borrowed` 说明「白捡的」—— 从音轨分离那边借来的。
            "borrowed": dll.is_some()
                && !dll.as_ref().unwrap().starts_with(data_dir(writable)),
            /* 解出来那个 DLL 有多大（实测 15.55 MB），以及要下的 zip 有多大
               （实测 78 MB —— 官方只给整包，见 `RUNTIME_ZIP_BYTES` 的注释）。
               界面写「下载运行库（78 MB）」用的是这两个数。 */
            "dllBytes": RUNTIME_DLL_BYTES,
            "zipBytes": RUNTIME_ZIP_BYTES,
            "partBytes": leftover("ort.part"),
        },
        "models": {
            "ready": missing.is_empty(),
            "missing": missing,
            "dir": crate::platform::clean_path(&active),
            /* `"origin"` 不是 `"source"` —— `"source"` 这个名字已经被下面那行
               （上游仓库地址）占了。同一个对象里两个同义键，将来谁把它拍平谁踩坑。 */
            "origin": models_origin,
            // 下多少 / 解开多少。两个数差着 34 MB 的 zip 压缩量，界面要说清。
            "zipBytes": MODEL_ZIP_BYTES,
            "extractBytes": MODEL_BYTES,
            "partBytes": leftover("game-models.part"),
        },
        "license": "模型权重 CC BY-NC-SA 4.0（非商业）",
        "source": "https://github.com/openvpi/GAME",
        "device": {
            "mode": ds.device.as_str(),
            "cuda": { "ok": ds.cuda_ok, "detail": ds.cuda_detail },
            "note": ds.note,
        },
    })
}

// ---------------------------------------------------------------------------
// 下载
// ---------------------------------------------------------------------------

/* ⚠️ **这里没有续传，`resume_from` 恒为 0，`.pause` 与 `.stop` 效果一样。**
 *
 * 音轨分离那份（`svsep::fetch_bundle`）有完整的续传：`.part` + `.part.url` 记号、
 * 服务端回 206 才追加、200 就归零 —— 因为它的包是 4.7 GB，断一次就白下几小时。
 * 这里两个包分别是 364 MB 与 78 MB，为此再养一套续传逻辑不划算，
 * 所以 `fetch_to_file` 一律从头下，状态里 `resumable` 也**恒为 false**（界面据此
 * 不画「继续下载」按钮）。
 *
 * ⛔ **别在界面上给它加「暂停 / 继续」**：`/api/midi/download/pause` 这条路由留着
 * 只是为了与音轨分离的接口形状一致（`DownloadCtl` 要求两个 flag），
 * 真语义是「停下、`.part` 留着、下次重新下」—— 写「继续」就是骗用户。
 */

/// 下载并解包 ONNX 权重（364 MB → 解出 376 MB 的三个图）。
///
/// `on_progress(已下字节, 可选总字节, 阶段)`，阶段 `"download"` / `"extract"`，
/// 与音轨分离那份状态形状一致，前端两个页面可以共用同一套进度条。
pub async fn download_models(
    writable: &Path,
    ctl: &crate::svsep::DownloadCtl,
    on_progress: impl Fn(u64, Option<u64>, crate::svsep::Stage) + Send + Sync + 'static,
) -> Result<Value, String> {
    let dir = data_dir(writable);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
    let zip = dir.join("game-models.part");

    crate::svsep::fetch_to_file(
        &model_url(),
        &zip,
        Some(MODEL_ZIP_BYTES),
        ctl,
        // ⚠️ `fetch_to_file` 的回调是**三参**的（`got, total, Stage`），阶段它自己带 ——
        // 别再手工把 `Stage::Download` 塞进去，那样后面解压那一段没有进度。
        &|got, total, stage| on_progress(got, total, stage),
    )
    .await?;
    on_progress(0, None, crate::svsep::Stage::Extract);
    /* 落到**可写那一层的 `game/models`**（`engine::model_dirs` 的第一个候选）。
       ⚠️ 不要图省事写 `data_dir(writable)`（那是 `midi/`）：下载能成功、解包能成功，
       唯独 `engine::resolve_model_dir` 找不到它 —— 用户点「开始扒谱」时才报
       「模型还没装全」，而状态页明明显示已就绪。 */
    let dest = writable.join("game").join("models");
    std::fs::create_dir_all(&dest).map_err(|e| format!("建目录失败：{e}"))?;
    /* 包里是 `GAME-1.0.3-large-onnx/<文件>`，所以剥掉这一层。
       ⚠️ 剥错不会报错，只会在用户点「开始扒谱」时才现形：「模型没下全」。
       实测过中央目录，前缀就是这一个。 */
    let strip = "GAME-1.0.3-large-onnx/";
    let report = crate::svsep::extract_zip(&zip, &dest, strip, |done, all| {
        on_progress(done, Some(all), crate::svsep::Stage::Extract)
    })?;
    let _ = std::fs::remove_file(&zip);

    let missing = engine::missing_models(&dest);
    if !missing.is_empty() {
        return Err(format!(
            "模型包解开了，但缺 {} —— 包的结构和预期不一样（是不是换了 release？）",
            missing.join("、")
        ));
    }
    Ok(json!({
        "ok": true,
        "dir": crate::platform::clean_path(&dest),
        "files": report.files,
        "bytes": report.bytes,
    }))
}

/// 下载 ONNX Runtime 那一个 DLL。
///
/// 只在下「单文件 ORT」那条路时用；用户装了音轨分离的话 `runtime_dll` 会直接
/// 借到它，这个函数压根不会被叫到。
pub async fn download_runtime(
    writable: &Path,
    ctl: &crate::svsep::DownloadCtl,
    on_progress: impl Fn(u64, Option<u64>, crate::svsep::Stage) + Send + Sync + 'static,
) -> Result<Value, String> {
    let dir = data_dir(writable);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
    let zip = dir.join("ort.part");

    /* ⚠️ 走 `fetch_first_ok` 而不是直接 `fetch_to_file`：官方那个 GitHub 地址在境内
       经常下不动，而 `fetch_to_file` 只会在**同一个**地址上重试五轮（见它的注释）。
       这是用户报「70 MB 运行库下不下来」之后加的退路。 */
    fetch_first_ok(&runtime_urls(), &zip, ctl, &|got, total, stage| {
        on_progress(got, total, stage)
    })
    .await?;    on_progress(0, None, crate::svsep::Stage::Extract);
    // 整包解到临时目录，再把要的那一个 DLL 拎出来 —— ORT 的 zip 里
    // `onnxruntime-win-x64-1.23.2/lib/onnxruntime.dll`，还带着头文件与
    // `onnxruntime_providers_shared.dll`（那个 dll 用不上，但同目录放着无害）。
    let tmp = dir.join("ort-unpack");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("建目录失败：{e}"))?;
    let strip = "onnxruntime-win-x64-1.23.2/";
    let report = crate::svsep::extract_zip(&zip, &tmp, strip, |done, all| {
        on_progress(done, Some(all), crate::svsep::Stage::Extract)
    })?;
    let _ = std::fs::remove_file(&zip);

    let src = tmp.join("lib").join(crate::tools::dll("onnxruntime"));
    if !src.is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("ONNX Runtime 包里没找到 lib/onnxruntime.dll".into());
    }
    let out = dir.join(crate::tools::dll("onnxruntime"));
    std::fs::copy(&src, &out).map_err(|e| format!("复制动态库失败：{e}"))?;
    // 相邻的 provider 共享库也留一份：ORT 会在**动态库同目录**找它，
    // 找不到时大多数算子仍然能跑，但会有一次没必要的失败重试。
    let shared = tmp.join("lib").join(crate::tools::dll("onnxruntime_providers_shared"));
    if shared.is_file() {
        let _ = std::fs::copy(&shared, dir.join(crate::tools::dll("onnxruntime_providers_shared")));
    }
    let _ = std::fs::remove_dir_all(&tmp);

    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    Ok(json!({
        "ok": true,
        "dll": crate::platform::clean_path(&out),
        "size": size,
        "files": report.files,
    }))
}

/// 删掉本功能下下来的东西（模型 + 动态库）。用于「一键清理依赖」。
///
/// 返回 `(文件数, 字节数, 说明)`，说明是**给界面直接显示的中文**。
///
/// # ⚠️ 为什么必须同时删 `<可写>/game/models`
///
/// 第一版只清了 `data_dir(writable)`（= `<可写>/midi/`），而那里**只有**
/// 下载物 `ort.part` 与 `ort-unpack\` —— 模型其实落在 `<可写>/game/models/`
/// （见 `download_models` 里的长注释）。后果是：用户点「删除依赖」，347 MB 的
/// 模型一个字节没少，`status` 照样回 `models.ready = true`，
/// **下载按钮再也不出现**，看着像按钮坏了。
///
/// # ⚠️ 为什么删完还要回头看一眼盘上的真状态
///
/// `engine::model_dirs` 是两层：可写的 `<可写>/game/models` 与随包只读的
/// `<root>/app/data/game/models`，谁先齐用谁。**绿色版这两层是同一个路径**
/// （`writable == <root>/app/data`），所以把开发机的模型放进那一层之后，
/// 删掉 = 引擎回落到「随包自带」= 状态仍然「就绪」。
/// 这不是 bug（本就该能跑），但**对用户完全说不通** —— 点了删除、按钮没出来、
/// 也看不出为什么。所以这里把真实原因查出来，交给界面写清楚：
/// 是「随包自带的那份留着」，还是「下下来那份已删、按钮马上就出来」。
pub fn delete_deps(root: &Path, writable: &Path) -> (u64, u64, String) {
    fn rm_tree(p: &Path, files: &mut u64, bytes: &mut u64) {
        if !p.exists() {
            return;
        }
        let (f, b) = dir_size(p);
        if std::fs::remove_dir_all(p).is_ok() {
            *files += f;
            *bytes += b;
        }
    }

    let mut files = 0u64;
    let mut bytes = 0u64;

    // ① 下载物：`.part` 残留、`ort-unpack\`、下下来的那个 dll
    let dir = data_dir(writable);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                rm_tree(&p, &mut files, &mut bytes);
            } else if let Ok(m) = p.metadata() {
                if std::fs::remove_file(&p).is_ok() {
                    files += 1;
                    bytes += m.len();
                }
            }
        }
    }

    // ② 下下来的模型。⚠️ 只删**可写那一层**，随包只读的那层一律不碰
    //    （那是安装内容，删了就是把程序拆坏 —— 何况安装版下它根本不可写）
    let dirs = engine::model_dirs(root, writable);
    let downloaded = dirs[0].clone();
    let bundled = dirs[1].clone();
    rm_tree(&downloaded, &mut files, &mut bytes);

    /* ⚠️ 这段判断必须**在删完之后**做（`models_dir` 是拿盘上真状态算的）。
       ⚠️ `bundled == downloaded` 那一条**必须先判**：绿色版两层同路径（`status` 里报
       `origin = "local"`），删掉就是真删掉了、引擎没有第二层可回落，状态随即变成
       「缺 3 个」—— 那时还说「引擎会接着用随包那份」是**错话**。
       只有安装版（两层是两个目录）才可能出现「删完仍就绪」这件事。 */
    let note = if bundled == downloaded {
        String::new()
    } else if engine::missing_models(&models_dir(root, writable)).is_empty() {
        format!(
            "但随包自带的那份模型仍在 {}，所以状态还是「就绪」、不会出现下载按钮 —— \
             那份是安装内容，这个按钮不碰它。",
            crate::platform::clean_path(&bundled)
        )
    } else {
        "现在可以点「下载模型」重新下。".to_string()
    };

    (files, bytes, note)
}

fn dir_size(dir: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (files, bytes);
    };
    for ent in rd.flatten() {
        let p = ent.path();
        match p.metadata() {
            Ok(m) if m.is_dir() => {
                let (f, b) = dir_size(&p);
                files += f;
                bytes += b;
            }
            Ok(m) => {
                files += 1;
                bytes += m.len();
            }
            Err(_) => {}
        }
    }
    (files, bytes)
}

// ---------------------------------------------------------------------------
// 推理编排
// ---------------------------------------------------------------------------

/// 一次转录任务里可以被取消的那个旗标。
///
/// 粒度是**切片**：一个切片内部的 8 步去噪中途停不下来（每步都在 ORT 里跑，
/// 没有回头的机会），但一首歌通常就一两个切片。真要更细的粒度得把回调塞进
/// `engine::transcribe` 的每一步 —— 那会让那条已经逐位验证过的路径多一层
/// 间接，不值当。
pub struct Cancel(AtomicBool);

impl Cancel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(AtomicBool::new(false)))
    }
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self(AtomicBool::new(false))
    }
}

/// 一个输入文件转成 44.1 kHz 单声道 f32。
///
/// **解码交给 ffmpeg**（随包在 `tools/ffmpeg/`）：用户拖进来的可能是 mp3 / m4a /
/// flac / 视频，甚至是采样率不对的 wav。自己写解码器没有任何好处，而 ffmpeg
/// 本来就是这个程序的一部分（`audio.rs` 全在用）。
///
/// 输出写成 **32 位浮点 WAV**（`pcm_f32le`）：GAME 的输入是 f32 波形，
/// 走 16 位会在量化上白丢精度 —— 而 `game::fixture::read_wav_mono_f32` 正好
/// 就能读这一种，不用再引解码依赖。
pub async fn decode_to_wav(
    tools_dir: &Path,
    input: &Path,
    out: &Path,
    duration_sec: f64,
    cancel: &crate::net::Cancel,
    on_progress: &crate::audio::Progress,
) -> Result<(), String> {
    // ⚠️ `-map 0:a:0` 而不是让它自己挑流：拖进来的是视频时，不加这个会选中
    //    视频流然后抱怨「没有音频」（其实有）。取第一条音轨也正是用户想要的。
    let args: Vec<String> = vec![
        "-i".into(),
        input.to_string_lossy().to_string(),
        "-map".into(),
        "0:a:0".into(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        algo::SAMPLE_RATE.to_string(),
        "-c:a".into(),
        "pcm_f32le".into(),
        "-f".into(),
        "wav".into(),
        out.to_string_lossy().to_string(),
    ];
    crate::audio::run_ffmpeg(tools_dir, &args, duration_sec, cancel, on_progress).await
}

/// 跑一次转录，返回音符表与耗时。
///
/// ⚠️ **这个函数是阻塞的**（CPU 推理，几秒到几分钟）。调用方必须用
/// `tokio::task::spawn_blocking` 包起来，否则会把整个 tokio 运行时卡住 ——
/// 表现是界面所有接口一起失联，而进度条停在原地。
pub fn transcribe_blocking(
    models: &Path,
    dll: &Path,
    wav: &Path,
    opts: &engine::Options,
    cancel: &Cancel,
    on_progress: impl Fn(&str, f64) + Send + Sync,
) -> Result<engine::Report, String> {
    let wave = crate::game::fixture::read_wav_mono_f32(wav)
        .map_err(|e| format!("读音频失败（{}）：{e}", wav.display()))?;
    if wave.is_empty() {
        return Err("这段音频是空的".into());
    }
    if cancel.stopped() {
        return Err(CANCELLED.into());
    }
    let report = engine::transcribe(models, dll, &wave, opts, &|what, pct| {
        on_progress(what, pct)
    })?;
    if cancel.stopped() {
        return Err(CANCELLED.into());
    }
    Ok(report)
}

/// 取消时返回的错误文案。`server/midi.rs` 认这个串把任务标成「已取消」而不是「失败」。
pub const CANCELLED: &str = "已取消";

// ---------------------------------------------------------------------------
// 结果落盘
// ---------------------------------------------------------------------------

/// 把 `Report` 写成三样东西：`.mid`、`.csv`、`.json`，返回文件名与路径。
///
/// 为什么三样都给：
///   * `.mid` 是终点（拖进 DAW / 编辑器直接用）；
///   * `.csv` 是给人的（哪一秒哪个音、偏高多少音分，表格里一眼看得出）；
///   * `.json` 是给程序的（保留浮点音高，别的工具要接着处理就用它）。
///
/// `base` 是不带扩展名的文件名主干（调用方已经从输入文件名取好并清过）。
pub fn write_outputs(out_dir: &Path, base: &str, report: &engine::Report) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("建输出目录失败：{e}"))?;
    let mut written = Vec::new();

    let mid = out_dir.join(format!("{base}.mid"));
    std::fs::write(&mid, engine::notes_to_midi(&report.notes))
        .map_err(|e| format!("写 MIDI 失败：{e}"))?;
    written.push(mid);

    let mut csv = String::from("index,onset,offset,duration,pitch,midi\n");
    for (i, (onset, offset, pitch, note)) in engine::note_rows(&report.notes).iter().enumerate() {
        csv.push_str(&format!(
            "{},{:.4},{:.4},{:.4},{:.4},{}\n",
            i + 1,
            onset,
            offset,
            offset - onset,
            pitch,
            note
        ));
    }
    let csv_path = out_dir.join(format!("{base}.csv"));
    std::fs::write(&csv_path, csv).map_err(|e| format!("写 CSV 失败：{e}"))?;
    written.push(csv_path);

    let json_path = out_dir.join(format!("{base}.json"));
    let body = json!({
        "source": "GAME (V-Synth-Studio 原生移植)",
        "samplerate": algo::SAMPLE_RATE,
        "timestep": algo::TIMESTEP,
        /* 实际用的是哪条推理路径。⚠️ 用户选了 GPU 也可能走到这儿是 "CPU"
           （CUDA 建会话失败会整条退回，原因写在任务日志里），所以**必须记下来** ——
           否则「我明明开了 GPU」和「怎么还是这么慢」之间没有任何可查的东西。 */
        "backend": report.backend,
        "nSamples": report.n_samples,
        "slices": report.slices.iter().map(|(o, n)| json!({"offset": o, "samples": n})).collect::<Vec<_>>(),
        "steps": report.per_step,
        "seconds": {
            "encoder": report.encoder_seconds,
            "segmenter": report.segmenter_seconds,
            "estimator": report.estimator_seconds,
        },
        "notes": engine::note_rows(&report.notes).iter().map(|(o, e, p, m)| json!({
            "onset": o, "offset": e, "pitch": p, "midi": m,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写 JSON 失败：{e}"))?;
    written.push(json_path);

    Ok(written)
}

/// 从输入文件名取一个干净的输出主干。
///
/// 去掉路径、扩展名，再把文件名里不适合做文件名的字符换成 `_` —— 中文留着
/// （Windows 上完全合法，用户自己的歌名本来也多半是中文），只挡 `\ / : * ? " < > |`。
pub fn output_stem(input: &Path) -> String {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名".into());
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 0x20 {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "未命名".into()
    } else {
        cleaned.chars().take(80).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_stem_strips_extensions_and_blocks_path_separators() {
        assert_eq!(output_stem(Path::new(r"D:\歌\干声.wav")), "干声");
        assert_eq!(output_stem(Path::new("/tmp/a.mp3")), "a");
        /* 文件名里本来不该有的字符被换掉；中文与空格留着。
           ⚠️ **必须带盘符写**（`D:\...\`）：单独写 `x:y*z?.flac` 时它是个
           *盘符相对*路径，`Path::file_stem` 会把 `x:` 当盘符前缀剥掉，
           只剩 `y_z_`。这条断言曾经就是这么写错的（第一版测试自己的 bug，
           不是 `output_stem` 的）。用户给进来的永远是 `FilePicker` 挑的绝对路径。 */
        assert_eq!(output_stem(Path::new(r"D:\歌\x:y*z?.flac")), "x_y_z_");
        assert_eq!(output_stem(Path::new("人声 01.m4a")), "人声 01");
        // 全被过滤掉、或者压根没有名字，都退到占位名
        assert_eq!(output_stem(Path::new("...")), "未命名");
        /* ⚠️ 全是点的名字 `"..."` 被 `trim_matches('.')` 吃掉才走到占位名；
           但 `.wav` 这种**没有主干只有扩展名**的，`file_stem()` 给的就是 `"wav"`
           （Rust 视 `.wav` 为「无扩展名的点文件」，不是「扩展名为 wav」）——
           于是输出会是 `wav.mid`。难看但不炸，不值得为它加特判。 */
        assert_eq!(output_stem(Path::new(r"D:\歌\.wav")), "wav");
    }

    #[test]
    fn stem_is_capped_so_windows_never_complains() {
        let long = "啊".repeat(200);
        let got = output_stem(Path::new(&format!("{long}.wav")));
        assert_eq!(got.chars().count(), 80);
    }

    #[test]
    fn cancel_starts_clear_and_stays_set() {
        let c = Cancel::new();
        assert!(!c.stopped());
        c.stop();
        assert!(c.stopped());
    }

    /// `mini_onnx()` 的字节要是拼错了，`probe_cuda` 会给出**错误的结论** ——
    /// 一台真有 N 卡的机器会被报成「CUDA 建不出会话」，而用户只看到 GPU 那格锁着。
    /// 所以这里不依赖 onnxruntime，直接按 protobuf 线格式把它拆一遍。
    #[test]
    fn mini_onnx_is_a_well_formed_model() {
        /// 解析一个 varint，返回 `(值, 吃掉几个字节)`。
        fn take_varint(b: &[u8], i: usize) -> (u64, usize) {
            let (mut v, mut n) = (0u64, 0usize);
            loop {
                let byte = *b.get(i + n).expect("varint 越界");
                v |= ((byte & 0x7f) as u64) << (7 * n);
                n += 1;
                if byte & 0x80 == 0 {
                    return (v, n);
                }
            }
        }
        /// 把一个 message 的字段拆成 `(field_number, 内容)`，只处理 LEN 与 varint。
        fn fields(b: &[u8]) -> Vec<(u64, Vec<u8>)> {
            let mut out = Vec::new();
            let mut i = 0;
            while i < b.len() {
                let (tag, n) = take_varint(b, i);
                i += n;
                let (num, wire) = (tag >> 3, tag & 7);
                assert!(
                    wire == 0 || wire == 2,
                    "字段 {num} 的 wire type 是 {wire}（整块是 {}）",
                    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
                );
                if wire == 0 {
                    let (v, n) = take_varint(b, i);
                    i += n;
                    out.push((num, v.to_le_bytes().to_vec()));
                } else {
                    let (len, n) = take_varint(b, i);
                    i += n;
                    let end = i + len as usize;
                    assert!(end <= b.len(), "字段 {num} 声称的长度越界");
                    out.push((num, b[i..end].to_vec()));
                    i = end;
                }
            }
            out
        }
        fn str_of(f: &[u8]) -> &str {
            std::str::from_utf8(f).expect("不是 UTF-8")
        }
        /// `ValueInfoProto` → 名字。⚠️ **必须按字段号找，不能按下标猜** ——
        /// 这里真踩过：`type` 写成了字段 1，于是 `ValueInfoProto` 成了「两个 `name`、
        /// 没有 `type`」，而最早那版测试按 `fs[1]` 取值，**照样全绿**。
        fn value_info(f: &[u8], who: &str) -> String {
            let fs = fields(f);
            let name_field = fs.iter().find(|(n, _)| *n == 1).expect("没有 name 字段");
            assert_eq!(name_field.0, 1);
            let type_field = fs
                .iter()
                .find(|(n, _)| *n == 2)
                .unwrap_or_else(|| panic!("{who}: ValueInfoProto 没有 type 字段（只有 {:?}）", fs.iter().map(|(n, _)| *n).collect::<Vec<_>>()));
            let type_proto = fields(&type_field.1);
            assert_eq!(type_proto[0].0, 1, "{who}: TypeProto.tensor_type 必须是字段 1");
            let tensor = fields(&type_proto[0].1);
            assert_eq!(tensor[0].0, 1, "{who}: elem_type 必须是字段 1");
            assert_eq!(tensor[0].1, [1, 0, 0, 0, 0, 0, 0, 0], "{who}: elem_type 必须是 FLOAT(1)");
            assert_eq!(tensor[1].0, 2, "{who}: shape 必须是字段 2");
            let shape = fields(&tensor[1].1);
            assert_eq!(shape[0].0, 1, "{who}: shape.dim[0] 必须是字段 1");
            let dim = fields(&shape[0].1);
            assert_eq!(dim[0].0, 1, "{who}: dim_value 必须是字段 1");
            assert_eq!(dim[0].1, [1, 0, 0, 0, 0, 0, 0, 0], "{who}: dim_value 必须是 1（静态维度）");
            str_of(&name_field.1).to_owned()
        }

        let m = fields(&mini_onnx());
        assert_eq!(m[0].0, 1, "第一个字段必须是 ir_version");
        assert_eq!(m[0].1, [7, 0, 0, 0, 0, 0, 0, 0], "ir_version 必须是 7");
        assert_eq!(m[1].0, 7, "第二个字段必须是 graph");
        assert_eq!(m[2].0, 8, "第三个字段必须是 opset_import");

        let graph = fields(&m[1].1);
        assert_eq!(str_of(&graph[0].1), "cuda-probe");
        let node = fields(&graph[1].1);
        assert_eq!(node[0].0, 1, "node.input");
        assert_eq!(str_of(&node[0].1), "x");
        assert_eq!(node[1].0, 2, "node.output");
        assert_eq!(str_of(&node[1].1), "y");
        assert_eq!(node[2].0, 4, "node.op_type");
        assert_eq!(str_of(&node[2].1), "Identity");
        assert_eq!(value_info(&graph[2].1, "input"), "x", "graph.input");
        assert_eq!(value_info(&graph[3].1, "output"), "y", "graph.output");

        let opset = fields(&m[2].1);
        assert_eq!(opset[0].1, [13, 0, 0, 0, 0, 0, 0, 0], "opset 必须是 13");
    }

    /// 同一个进程里重复 `ort::init_from` 是安全的（它内部是 `OnceLock`），
    /// 而且我们只在这一个测试里加载。找不到就跳过（干净的 CI 上没有运行时）。
    #[test]
    fn a_real_onnxruntime_accepts_the_probe_graph() {
        let dll = Path::new(
            r"H:\工作站\app\data\svsep\runtime\Lib\site-packages\onnxruntime\capi\onnxruntime.dll",
        );
        if !dll.is_file() {
            eprintln!("跳过：这台机器上没有借用的 onnxruntime.dll（{}）", dll.display());
            return;
        }
        crate::game::engine::load_runtime(dll).expect("加载 ONNX Runtime");
        let s = ort::session::Session::builder()
            .expect("建 builder")
            .with_execution_providers([ort::ep::CPU::default().build()])
            .expect("注册 CPU")
            .commit_from_memory(&mini_onnx());
        assert!(s.is_ok(), "ONNX Runtime 不认这张探针图：{:?}", s.err());
    }

    #[test]
    fn status_reports_every_candidate_missing() {
        let dir = std::env::temp_dir().join("vss-midi-status-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let v = status(&dir, &dir);
        assert_eq!(v["models"]["ready"], json!(false));
        assert_eq!(v["models"]["missing"].as_array().unwrap().len(), 3);
        assert_eq!(v["runtime"]["ready"], json!(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 摆一套假模型（内容无所谓，`missing_models` 只看文件在不在）。
    fn fake_models(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for f in engine::MODEL_FILES {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
    }

    /// `models.origin` 三种取值，以及「绿色版两层同路径」这个坑。
    ///
    /// ⚠️ 这组断言是**给未来改的人看的**：`origin` 报的是「引擎在用哪个目录」，
    /// 不是「这份是谁放的」—— 绿色版下这两件事物理上不可分，任何想按目录状态
    /// 区分「下下来的 / 随包自带的」的改法都会在这里红。
    #[test]
    fn models_origin_distinguishes_portable_from_installed() {
        let base = std::env::temp_dir().join("vss-midi-origin-test");
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let writable = base.join("appdata");

        // ① 绿色版：`writable == root/app/data` ⇒ 两层是同一个绝对路径
        let portable = root.join("app").join("data");
        fake_models(&portable.join("game").join("models"));
        let v = status(&root, &portable);
        assert_eq!(v["models"]["ready"], json!(true));
        assert_eq!(
            v["models"]["origin"],
            json!("local"),
            "绿色版两层同路径 ⇒ local（没有第二层可回落，界面不该说「随包自带」）"
        );

        // ② 安装版 + 只有可写那一层有 ⇒ downloaded
        fake_models(&writable.join("game").join("models"));
        let v = status(&root, &writable);
        assert_eq!(v["models"]["origin"], json!("downloaded"));

        // ③ 安装版 + 可写层空、随包层齐 ⇒ bundled（界面要据此写清删除按钮的行为）
        let _ = std::fs::remove_dir_all(writable.join("game"));
        fake_models(&root.join("app").join("data").join("game").join("models"));
        let v = status(&root, &writable);
        assert_eq!(v["models"]["ready"], json!(true));
        assert_eq!(v["models"]["origin"], json!("bundled"));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 推理方式的读写与容错。
    ///
    /// ⚠️ 这组断言的用处是**挡住「顺手改个名字」**：设置文件写的是
    /// `<可写>/midi/midi_settings.json`，音轨分离那边是
    /// `<可写>/svsep/data/inference_settings.json`（Python 在管）。两边都叫
    /// 「推理方式」但**不是同一个文件、键名也不同**（我们 `device`、它 `mode`），
    /// 谁把这两个合并都会在这里红。
    #[test]
    fn device_settings_round_trip_and_fall_back_to_auto() {
        let base = std::env::temp_dir().join("vss-midi-device-test");
        let _ = std::fs::remove_dir_all(&base);
        let writable = base.join("appdata");

        // 没有文件 ⇒ 自动
        assert_eq!(read_device(&writable), Device::Auto);

        for dev in [Device::Gpu, Device::Cpu, Device::Auto] {
            write_device(&writable, dev).unwrap();
            assert_eq!(read_device(&writable), dev, "写进去再读出来必须一致");
        }

        // 键名是 `device`，跟音轨分离那边的 `mode` 不一样
        let raw = std::fs::read_to_string(settings_file(&writable)).unwrap();
        assert!(raw.contains("\"device\""), "实际写出来的是：{raw}");
        assert!(
            !raw.contains("inference_settings"),
            "别把设置写进音轨分离那个文件，实际：{raw}"
        );

        // 认不出的值 / 坏文件 ⇒ 自动（界面永远拿得到一个合法值）
        std::fs::write(settings_file(&writable), r#"{"device":"tpu"}"#).unwrap();
        assert_eq!(read_device(&writable), Device::Auto, "认不出的值当自动");
        std::fs::write(settings_file(&writable), "{ 这不是 JSON").unwrap();
        assert_eq!(read_device(&writable), Device::Auto, "文件坏了也当自动");

        // 解析函数本身：大小写与空格都吃掉，其余一律自动
        assert_eq!(Device::parse(" GPU "), Device::Gpu);
        assert_eq!(Device::parse("Cpu"), Device::Cpu);
        assert_eq!(Device::parse("cuda"), Device::Auto);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 没有 CUDA 的那台机器上，`device_status` 该说什么是**用户唯一看得到的东西**：
    /// 页面只负责把后端给的 `note` 原样显示，所以这段话术的触发条件必须钉住。
    ///
    /// ⚠️ `dll = None` 是**唯一能在这台 AMD 机器上稳定复现的分支**：真给一个 dll
    /// 路径会走 `preload_dylibs`（把 638 MB 的 cublasLt 真加载进测试进程）。
    /// 「能解锁」那一半这机器验不了，由 N 卡用户端反馈兜底。
    #[test]
    fn device_status_talks_to_the_user_when_cuda_is_missing() {
        let base = std::env::temp_dir().join("vss-midi-devstatus-test");
        let _ = std::fs::remove_dir_all(&base);
        let writable = base.join("appdata");

        // 没选 GPU ⇒ 不该有那句补救话术（页面此时也不该出现警示条）
        write_device(&writable, Device::Cpu).unwrap();
        let st = device_status(&writable, None);
        assert_eq!(st.device, Device::Cpu);
        assert!(!st.cuda_ok);
        assert!(st.note.is_empty(), "选了 CPU 还说「用不了 GPU」是错话：{}", st.note);
        assert!(st.cuda_detail.contains("先下运行库"), "实到：{}", st.cuda_detail);

        // 选了 GPU 但用不了 ⇒ 必须给话术，且带上后端探出来的原因
        write_device(&writable, Device::Gpu).unwrap();
        let st = device_status(&writable, None);
        assert_eq!(st.device, Device::Gpu);
        assert!(!st.cuda_ok);
        assert!(st.note.contains("用不了 GPU"), "实到：{}", st.note);
        assert!(st.note.contains(&st.cuda_detail), "话术里必须带上原因，实到：{}", st.note);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 「往上找 nvidia 目录」那个路标查找器 —— **找不到时必须回 `None`**。
    ///
    /// ⛔ 回 `None` 与回一个不存在的目录，后果完全不同：`probe_cuda` 拿到 `None`
    /// 会继续去问 ONNX Runtime（机器上可能有系统级 CUDA），而拿到一个空目录会
    /// 让 `preload_dylibs` 直接报错、把整条 GPU 路判死。
    ///
    /// 另有一条更隐蔽的：**那 12 个组件分在四个 `bin/` 里**（cudart / cublas /
    /// cufft / cudnn），所以每个组件都得能各自定位到自己的目录 —— 夹具把这四层
    /// 都搭出来，下面逐个数一遍。
    /// `runtime_dll` 的挑法：**带 CUDA provider 的那份优先**，别让用户下过的
    /// 官方 CPU 包把音轨分离里那份 GPU 版遮蔽掉（用户报的「N 卡点不亮 GPU」）。
    #[test]
    fn a_cuda_capable_runtime_wins_over_a_downloaded_cpu_pack() {
        // debug 构建里 `runtime_candidates` 认这个环境变量，先清掉免得干扰
        std::env::remove_var("VSS_MIDI_DLL");
        let base = std::env::temp_dir().join("vss-midi-pick-test");
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let writable = base.join("writable");

        // ① 用户自己下过的那份：官方 CPU 包，落在 `<可写>/midi/`（候选里的第一个）
        let cpu = data_dir(&writable).join(crate::tools::dll("onnxruntime"));
        std::fs::create_dir_all(cpu.parent().unwrap()).unwrap();
        std::fs::write(&cpu, b"x").unwrap();

        // ② 音轨分离运行时里那份：GPU 构建 —— 旁边躺着 CUDA provider
        let capi = crate::svsep::runtime_dir(&root)
            .join("runtime")
            .join("Lib")
            .join("site-packages")
            .join("onnxruntime")
            .join("capi");
        std::fs::create_dir_all(&capi).unwrap();
        let gpu = capi.join(crate::tools::dll("onnxruntime"));
        std::fs::write(&gpu, b"x").unwrap();
        let provider = capi.join(crate::tools::dll("onnxruntime_providers_cuda"));

        // 只有 CPU 包时，就用它（用户没装音轨分离，这就是他能有的最好的一份）
        assert_eq!(runtime_dll(&root, &writable).as_deref(), Some(cpu.as_path()));

        // 多一份 GPU 构建之后，必须改挑 GPU 那份 —— 遮蔽 bug 就在这一行
        std::fs::write(&provider, b"x").unwrap();
        assert_eq!(runtime_dll(&root, &writable).as_deref(), Some(gpu.as_path()));

        // 把 GPU 那份删掉，退回 CPU 包（别把「没有」当成「出错」）
        std::fs::remove_file(&gpu).unwrap();
        assert_eq!(runtime_dll(&root, &writable).as_deref(), Some(cpu.as_path()));
    }

    /// GitHub 的地址要能生成镜像，别的地址不许乱加前缀。
    #[test]
    fn github_urls_get_a_mirror_and_other_hosts_do_not() {
        assert_eq!(
            mirrored(RUNTIME_URL).as_deref(),
            Some(
                "https://ghfast.top/https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-win-x64-1.23.2.zip"
            )
        );
        assert!(mirrored("https://1856610041.cdn.123clouddisk.com/x.zip").is_none());
        assert!(mirrored("http://github.com/a/b.zip").is_none(), "只认 https");
        assert!(mirrored("not a url").is_none());

        // 顺序：官方在前，镜像是退路
        let urls = runtime_urls();
        assert_eq!(urls.first().map(String::as_str), Some(runtime_url().as_str()));
        assert!(urls.len() >= 1);
    }

    #[test]
    fn find_dylibs_dir_needs_a_real_hit() {
        let base = std::env::temp_dir().join("vss-midi-dylib-test");
        let _ = std::fs::remove_dir_all(&base);
        /* 仿造真实布局。⚠️ **层级必须与真的一比一**：
             `…/Lib/site-packages/nvidia/cublas/bin/cublas64_12.dll`   ← 组件（在 nvidia 下面）
             `…/Lib/site-packages/onnxruntime/onnxruntime.dll`         ← ORT 本体（与 nvidia 同级）
           夹具多一层或少一层都会让人去改没坏的生产代码（这里真发生过：夹具曾多搭了一层
           `onnxruntime/capi/`）。 */
        let bin = base
            .join("Lib")
            .join("site-packages")
            .join("nvidia")
            .join("cublas")
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let dll = bin.join("cublas64_12.dll");
        std::fs::write(&dll, b"x").unwrap();
        let cuda = base
            .join("Lib")
            .join("site-packages")
            .join("onnxruntime")
            .join("onnxruntime.dll");
        std::fs::create_dir_all(cuda.parent().unwrap()).unwrap();
        std::fs::write(&cuda, b"x").unwrap();

        // 从 ORT 本体那个 dll 往上找，应当摸到 cublas\bin，再返回它（我们的 preload 根）
        let hit = find_dylibs_dir(&cuda, "cublas64_12.dll").expect("应当能找到");
        assert_eq!(hit, bin, "返回的必须是**装 dll 的那个目录**，不是 nvidia 根");

        /* ⭐ 四个组件四个目录，各进各的 `bin/`。真实布局就是这样：
             cudart64_12.dll → nvidia/cuda_runtime/bin/
             cublas64_12.dll → nvidia/cublas/bin/
             cufft64_11.dll  → nvidia/cufft/bin/
             cudnn64_9.dll   → nvidia/cudnn/bin/
           只传 `nvidia/` 给 `preload_dylibs` 一个都加载不上，所以这里逐个钉子：
           谁把 `find_dylibs_dir` 改成只认某一个组件目录，这几条立刻红。 */
        for comp in ["cuda_runtime", "cublas", "cufft", "cudnn"] {
            std::fs::create_dir_all(bin.parent().unwrap().parent().unwrap().join(comp).join("bin"))
                .unwrap();
        }
        std::fs::write(
            bin.parent().unwrap().parent().unwrap().join("cuda_runtime").join("bin").join("cudart64_12.dll"),
            b"x",
        )
        .unwrap();
        std::fs::write(
            bin.parent().unwrap().parent().unwrap().join("cufft").join("bin").join("cufft64_11.dll"),
            b"x",
        )
        .unwrap();
        std::fs::write(
            bin.parent().unwrap().parent().unwrap().join("cudnn").join("bin").join("cudnn64_9.dll"),
            b"x",
        )
        .unwrap();
        let root = bin.parent().unwrap().parent().unwrap();
        assert_eq!(
            find_dylibs_dir(&cuda, "cudart64_12.dll"),
            Some(root.join("cuda_runtime").join("bin")),
            "cudart 在 cuda_runtime/bin，不能回 cublas 那个目录"
        );
        assert_eq!(
            find_dylibs_dir(&cuda, "cufft64_11.dll"),
            Some(root.join("cufft").join("bin")),
            "cufft 在 cufft/bin"
        );
        assert_eq!(
            find_dylibs_dir(&cuda, "cudnn64_9.dll"),
            Some(root.join("cudnn").join("bin")),
            "cudnn 在 cudnn/bin"
        );

        /* ⛔ 下面这条挡的是「把判据写成『哪一层叫 nvidia』」：从 ORT 本体往上走时
           候选是 `onnxruntime → site-packages → …`，而 `nvidia` 是 `site-packages`
           **下面**的一层，那样写条件恒假、永远找不到（这里真这么错过一次）。
           用一个 nvidia 树里不存在的名字，且它**只**在 ORT 自己那一层有 —— 若有人
           把查找范围放宽到「整个 site-packages」，这条会红。 */
        std::fs::write(cuda.parent().unwrap().join("onnxruntime_providers_shared.dll"), b"x").unwrap();
        assert_eq!(
            find_dylibs_dir(&cuda, "onnxruntime_providers_shared.dll"),
            None,
            "组件查找只在 nvidia/ 里做，别把 ORT 自己那一层也算进去"
        );

        // 找一个不存在的 dll ⇒ None（别返回一个看似合理的空目录）
        assert!(
            find_dylibs_dir(&cuda, "cublasLt64_12.dll").is_none(),
            "夹具里没有这个组件 ⇒ 必须是 None（别返回一个看似合理的空目录）"
        );

        // dll 在一个跟 nvidia 毫无关系的目录里 ⇒ None
        let lone = base.join("elsewhere").join("onnxruntime.dll");
        std::fs::create_dir_all(lone.parent().unwrap()).unwrap();
        std::fs::write(&lone, b"x").unwrap();
        assert!(find_dylibs_dir(&lone, "cublas64_12.dll").is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 点「删除依赖」必须**同时**清掉下下来的模型 —— 只清 `<可写>/midi/` 的话，
    /// 用户会看到「删了 0 个文件、模型还在、下载按钮再也不出现」。
    ///
    /// ⚠️ 第一版只清 `<可写>/midi/`（那儿只有 `.part` 与 `ort-unpack`），
    /// 于是用户点「删除依赖」后模型一个字节没少、下载按钮再也不出现 —— 真报过。
    #[test]
    fn delete_deps_removes_downloaded_models_but_not_bundled() {
        let base = std::env::temp_dir().join("vss-midi-delete-test");
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let writable = base.join("appdata");

        // 安装版：两层是两个目录，都能写
        let dl_dir = writable.join("game").join("models");
        let bundle_dir = root.join("app").join("data").join("game").join("models");
        fake_models(&dl_dir);
        fake_models(&bundle_dir);

        // 顺带放两个下载物，确认那一路也还在删
        let midi_dir = data_dir(&writable);
        std::fs::create_dir_all(&midi_dir).unwrap();
        std::fs::write(midi_dir.join("ort.part"), b"half").unwrap();

        let (files, bytes, note) = delete_deps(&root, &writable);
        assert_eq!(files, 4, "3 个模型 + 1 个 ort.part");
        assert_eq!(bytes, 3 + 4);
        assert!(!dl_dir.exists(), "可写那一层必须删掉");
        assert!(bundle_dir.join("encoder.onnx").is_file(), "随包那层一个字节都不能动");
        assert!(
            note.contains("随包自带"),
            "删完仍就绪时必须说清是随包那份在顶着，实际：{note}"
        );

        /* 绿色版：两层同路径，删完就是真没了 ⇒ note 必须是空的。
           ⚠️ 这里曾经写错成「引擎会接着用随包自带的那份模型」，是**错话** ——
           一个目录被删光了，引擎没有第二层可回落。 */
        let portable = base.join("portable").join("app").join("data");
        fake_models(&portable.join("game").join("models"));
        let (files, _bytes, note) = delete_deps(&base.join("portable"), &portable);
        assert_eq!(files, 3);
        assert_eq!(note, "", "绿色版删完是真删掉了，不能说「还在用随包那份」");

        let _ = std::fs::remove_dir_all(&base);
    }
}
