//! 外部工具探测与启动。
//!
//! 只有探测与启动两条，**没有 `install`** —— ffmpeg / yt-dlp 随包分发，
//! 一条「永远是错误」的命令留在 IPC 表里只会让人以为还有这条路。
//! 「缺了怎么办」写在 `tools_detect` 的结果里（`available: false` 时界面
//! 自己会显示「从压缩包里把 tools 目录重新解压」）。

use serde_json::{Value, json};

use super::Cmd;

/// **全部外部产物的状态一览。**
///
/// 这是「这台机器上到底缺什么」的**总账**：工具探测、分离状态、扒谱状态、拼音表
/// 各自回答一部分，形状还各不相同，而这里一次遍历 `artifact::ARTIFACTS` 就有 ——
/// 因为它就是唯一的真相。
///
/// 前端暂时没有页面用它（各页面继续用自己那条更详细的命令），
/// 这条是给**排障**用的：用户报「某个功能不能用」时，让他把这条的输出贴过来，
/// 缺什么、在哪个目录、齐不齐，一眼可见。
#[tauri::command]
pub async fn artifacts_status(st: super::St<'_>) -> Cmd {
    let ctx = crate::artifact::Ctx::new(&st.inner().root, &st.inner().writable);
    let items: Vec<Value> = crate::artifact::all()
        .iter()
        .map(|a| crate::artifact::report(&ctx, a.id))
        .collect();
    let missing: Vec<&str> = crate::artifact::all()
        .iter()
        .filter(|a| !crate::artifact::ready(&ctx, a))
        .map(|a| a.label)
        .collect();
    Ok(json!({
        "root": st.inner().root.to_string_lossy(),
        "writable": st.inner().writable.to_string_lossy(),
        "readyCount": items.iter().filter(|v| v["ready"] == json!(true)).count(),
        "totalCount": items.len(),
        // 「缺什么」按界面上的名字直说 —— 排障时先看这一行
        "missing": missing,
        "items": items,
    }))
}

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
/// 三段：外部工具、编辑器、以及一句总体结论 —— 界面按 `summary` 决定功能开关
/// 是亮着还是置灰。
pub fn detect_all_from(root: &std::path::Path, editors: Vec<Value>, tools: Value) -> Value {
    let installed = editors
        .iter()
        .filter(|e| {
            e.get("installed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
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
/// 为什么不能直接用 `Command::new`：程序是 windows 子系统，但被它拉起来的控制台
/// 程序（编辑器、脚本宿主）**会亮一个黑框**，一闪而过看着像出错。
///
/// `CREATE_NO_WINDOW`（0x0800_0000）只在 Windows 存在，所以这里必须 cfg 分叉 ——
/// 别的平台没有这个概念，直接用普通 `Command`。
/// ⚠️ 函数体按 cfg **整个**分叉（而不是「建好 Command、再条件加 flag」）：
/// 后者在非 Windows 上会留下一个多余的 `mut`，于是每个非 Windows 构建都吃一条
/// `unused_mut` 警告 —— 而警告一直挂在那里，真出问题时就被淹没了。
pub fn quiet_command(program: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = std::process::Command::new(program);
        c.creation_flags(CREATE_NO_WINDOW);
        c
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new(program)
    }
}
