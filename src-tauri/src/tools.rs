//! 本机编辑器与外部工具探测
//!
//! 平台相关的路径与扫描规则都收在这一层，移植 macOS 时只改这里 ——
//! 这正是把平台代码收在一处的意义。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// 编辑器/工具候选定义
///
/// ⚠️ 现在 `candidates()` 是空的，所以这个结构体在**编译产物里没有构造点**。
/// 保留它是因为 `detect_candidate` / `scan_for_exe` 都按它写好了，将来往
/// `candidates()` 里加一条就能用。字段在主程序路径上被认为是 dead code，
/// 但单测会构造它，所以别删。
#[allow(dead_code)]
struct Candidate {
    id: &'static str,
    name: &'static str,
    vendor: &'static str,
    category: &'static str,
    formats: &'static [&'static str],
    color: &'static str,
    /// 直接命中：这些路径存在就算找到
    paths: Vec<PathBuf>,
    /// 兜底：在这些目录里按文件名找（深度受限）
    scan_dirs: Vec<PathBuf>,
    /// 扫描时匹配的可执行文件名（小写，精确匹配）
    exe_names: &'static [&'static str],
    scan_depth: usize,
}

/// 需要探测的外部程序。**现在一个都没有**。
///
/// 这张表是空的，但**不要因为「空函数很怪」就把它删掉** —— `detect_editors`
/// 与 `/api/tools/detect` 的 `editors` / `installedCount` 都还挂在这条链上，
/// 前端 `state.editors` 也还在读。将来要有新的外部程序要探测，往这里加。
///
/// 探测范围只限「本机装了什么」：离线人声分离已内嵌（见 `crate::svsep`），
/// 不需要外部程序，编辑器也不靠它启动。
fn candidates() -> Vec<Candidate> {
    Vec::new()
}
/// 探测本机装了哪些编辑器
pub fn detect_editors() -> Vec<Value> {
    candidates()
        .into_iter()
        .map(|c| {
            let hit = detect_candidate(&c);
            json!({
                "id": c.id,
                "name": c.name,
                "vendor": c.vendor,
                "category": c.category,
                "formats": c.formats,
                "color": c.color,
                "installed": hit.is_some(),
                "path": hit.as_ref().map(|(p, _)| p.to_string_lossy().to_string()),
                "how": hit.as_ref().map(|(_, how)| *how),
            })
        })
        .collect()
}

/// 返回 (路径, 命中方式)
fn detect_candidate(c: &Candidate) -> Option<(PathBuf, &'static str)> {
    for p in &c.paths {
        if p.is_file() {
            return Some((p.clone(), "已知路径"));
        }
    }
    for dir in &c.scan_dirs {
        if let Some(p) = scan_for_exe(dir, c.exe_names, c.scan_depth) {
            return Some((p, "目录扫描"));
        }
    }
    None
}

/// 在目录里按文件名找可执行文件（深度受限，命中即返回）
fn scan_for_exe(dir: &Path, names: &[&str], max_depth: usize) -> Option<PathBuf> {
    fn walk(dir: &Path, names: &[&str], depth: usize, max: usize) -> Option<PathBuf> {
        if depth > max {
            return None;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return None;
        };
        let mut subdirs = Vec::new();
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                subdirs.push(e.path());
            } else if ft.is_file() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                if names.contains(&name.as_str()) {
                    return Some(e.path());
                }
            }
        }
        for d in subdirs {
            if let Some(p) = walk(&d, names, depth + 1, max) {
                return Some(p);
            }
        }
        None
    }
    walk(dir, names, 0, max_depth)
}

/* ══════════════════════════════════ 外部工具 ══════════════════════════════════ */

/// Howard Hinnant 的 civil_from_days（公历换算，不引 chrono）
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 某个随包工具的位置与版本，形状给 `/api/state` 用。
///
/// ⚠️ 候选位置与 PATH 兜底都只在表的 `places` / `path_names` 里声明**一次** ——
/// `audio::find_ffmpeg` / `ytdlp::find_ytdlp` 与这里共用同一套判据，别在任何一处
/// 另写一份。这个函数只负责「找到没、在哪、版本多少、来源叫什么」。
fn probe_tool(root: &Path, id: &str, version_args: &[&str], kind: VersionStyle) -> Value {
    let ctx = crate::artifact::Ctx::root_only(root);
    let a = crate::artifact::get(id).expect("probe_tool 的 id 必须在表里");
    match crate::artifact::locate(&ctx, a) {
        Some(l) => {
            let path = crate::artifact::entry_path(&l.dir, &a.need[0]);
            let version = match kind {
                VersionStyle::Plain => trim_version(&path, version_args),
                VersionStyle::Ffmpeg => ffmpeg_version(&path),
            };
            json!({
                "available": true,
                "path": path.to_string_lossy(),
                "source": l.source,
                "version": version,
            })
        }
        None => json!({ "available": false, "path": null, "source": "", "version": null }),
    }
}

/// 版本号怎么从输出里取。
///
/// ⚠️ 这不是多余的抽象：`ffmpeg -version` 第一行是
/// `ffmpeg version 7.1 Copyright (c) ...`，整行塞进界面很难看；而
/// `yt-dlp --version` 的输出只有版本号一行。两支都写成 `trim` 会退化 ——
/// ffmpeg 那边会把整行版权信息显示出来。
enum VersionStyle {
    /// 输出即版本号（trim 一下就用）
    Plain,
    /// 从第一行里取第 3 个词（`ffmpeg version <X>` 的 X）
    Ffmpeg,
}

/// 外部工具探测（ffmpeg / yt-dlp / python）
///
/// python 不走表：它是**系统级**依赖（PATH 里那个），不随包分发，
/// 也没有「装到 tools/ 下」这种形态。
pub fn detect_tools(root: &Path) -> Value {
    let ffmpeg = probe_tool(root, "ffmpeg", &["-version"], VersionStyle::Ffmpeg);
    let ytdlp = probe_tool(root, "ytdlp", &["--version"], VersionStyle::Plain);

    let python = match crate::platform::find_binary("python", &[]) {
        Some(p) => json!({
            "available": true,
            "path": "python",
            "version": trim_version(&p, &["--version"]),
        }),
        None => json!({ "available": false, "path": null, "version": null }),
    };

    json!({ "ffmpeg": ffmpeg, "python": python, "ytdlp": ytdlp })
}

pub fn exe(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// 动态库的文件名（`onnxruntime` → `onnxruntime.dll`）。
///
/// 和 `exe` 分开写而不是共用一个函数：动态库在 macOS 上是 `.dylib`、Linux 上是
/// `.so`，命名规则和可执行文件不同（`lib` 前缀、后缀位置都不一样）。现在只有
/// ONNX Runtime 用得上，等真要跨平台了那两个分支就是这么补。
pub fn dll(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{base}.dylib")
    } else {
        format!("lib{base}.so")
    }
}

fn run_capture(bin: &Path, args: &[&str]) -> Option<String> {
    let out = crate::ipc::tools::quiet_command(&bin.to_string_lossy())
        .args(args)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Some(if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    })
}

fn ffmpeg_version(bin: &Path) -> Option<String> {
    let text = run_capture(bin, &["-version"])?;
    text.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(2))
        .map(|s| s.to_string())
}

fn trim_version(bin: &Path, args: &[&str]) -> Option<String> {
    run_capture(bin, args).map(|s| s.trim().to_string())
}
