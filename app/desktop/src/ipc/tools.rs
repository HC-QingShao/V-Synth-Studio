//! 外部工具探测与启动。
//!
//! 旧 HTTP 有 `detect` / `install` / `launch` 三条，这里只留两条：
//!   * `install` **不做了** —— ffmpeg / yt-dlp 改为随包分发，那条命令必然报错。
//!     要告诉用户怎么办的话写在 `tools_detect` 的结果里（`available: false` 时界面
//!     自己会显示「从压缩包里把 tools 目录重新解压」）。一条「永远是错误」的命令
//!     留在 IPC 表里只会让人以为还有这条路。

use serde_json::{json, Value};

use super::Cmd;

/// 重新探测外部工具（**绕过缓存**）。
///
/// 这条命令的整个价值就是「绕过缓存」：用户把 `tools/` 目录补回来之后点「重新检测」，
/// 必须立刻反映真实情况，不能被那 60 秒的缓存骗过去。
/// 代价是它本来就要 2~7 秒（真的 spawn `yt-dlp --version` + 逐段扫 PATH），
/// 界面上要给转圈提示。
#[tauri::command]
pub async fn tools_detect(st: super::St<'_>) -> Cmd {
    let (editors, tools) = st.probe_cached(true);
    Ok(detect_all_from(&st.inner().root, editors, tools))
}

/// 把「探测结果」拼成界面要的完整响应。
///
/// 从 `server/tools.rs` 搬过来的（那一层整个删了）。三段：外部工具、编辑器、
/// 以及一句总体结论 —— 界面按 `summary` 决定功能开关是亮着还是置灰。
pub fn detect_all_from(root: &std::path::Path, editors: Vec<Value>, tools: Value) -> Value {
    let installed = editors
        .iter()
        .filter(|e| e.get("installed").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    let available = |k: &str| {
        tools
            .get(k)
            .and_then(|v| v.get("available"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    json!({
        "tools": tools,
        "editors": editors,
        // 格式表也顺手带上：界面「工程转换」那一页据此决定哪些格式可选
        "formats": crate::libresvip::list_formats(root),
        "summary": {
            "ffmpeg": available("ffmpeg"),
            "ytdlp": available("ytdlp"),
            "python": available("python"),
            "editorsInstalled": installed,
            "editorsTotal": editors.len(),
        },
    })
}

/// 启动一个外部程序（可选带一个文件参数）。
///
/// 调用方都已经有确切路径（设置页选的文件、音频页记下的编辑器路径）。
/// 工作目录设成程序自己所在目录 —— 不少编辑器要靠相对路径找自己的资源。
#[tauri::command]
pub async fn tools_launch(path: String, file: Option<String>) -> Cmd {
    if path.trim().is_empty() {
        return Err("缺少程序路径".into());
    }
    if !std::path::Path::new(&path).is_file() {
        return Err(format!("程序不存在或已被移动：{path}"));
    }
    let mut cmd = quiet_command(&path);
    if let Some(f) = file.filter(|s| !s.is_empty()) {
        cmd.arg(f);
    }
    let workdir = std::path::Path::new(&path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    cmd.current_dir(workdir)
        .spawn()
        .map_err(|e| format!("启动失败：{e}"))?;
    Ok(json!({ "launched": path }))
}

/// Windows 上起一个**不弹窗**的子进程。
///
/// 从 `server/simple.rs` 搬过来的。为什么不能直接用 `Command::new`：程序是
/// windows 子系统，但被它拉起来的控制台程序（编辑器、脚本宿主）**会亮一个黑框**，
/// 一闪而过看着像出错。
///
/// `CREATE_NO_WINDOW`（0x0800_0000）只在 Windows 存在，所以这里必须 cfg 分叉 ——
/// 别的平台没有这个概念，直接用普通 `Command`。
pub fn quiet_command(program: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}