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
        crate::svsep::runtime_dir(root)
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
pub fn runtime_dll(root: &Path, writable: &Path) -> Option<PathBuf> {
    runtime_candidates(root, writable).into_iter().find(|p| p.is_file())
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

    crate::svsep::fetch_to_file(
        &runtime_url(),
        &zip,
        None,
        ctl,
        &|got, total, stage| on_progress(got, total, stage),
    )
    .await?;
    on_progress(0, None, crate::svsep::Stage::Extract);
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

    /// 删除依赖：**必须连 `<可写>/game/models` 一起删**。
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
