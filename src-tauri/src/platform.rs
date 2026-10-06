//! 平台相关：Windows 特有的是 explorer、回收站；其余走标准库。
//!
//! 这一层是**唯一**放平台代码的地方 —— 移植 macOS 时只需要在这里加一个 cfg 分支，
//! 上层业务代码一行都不用改。
//!
//! ⚠️ 两样东西**不放这里**：
//!   * **系统文件/文件夹对话框** —— 走官方 `tauri-plugin-dialog`，见
//!     `ipc/fs.rs::pick_paths`。自己调 Win32（`GetOpenFileNameW` /
//!     `SHBrowseForFolderW`）还要处理 COM 初始化、owner 句柄、双 NUL 过滤器，
//!     全是不必要的成本。
//!   * **下载目录与文件系统根列表** —— 选目录一律走系统对话框
//!     （见 `components/DirPicker.tsx`），不需要给自画浏览器当数据源。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 平台名（`win32` / `darwin` / `linux`）。
///
/// ⚠️ 必须是这三个词，前端读它判断「哪些功能在哪些平台可用」——
/// `std::env::consts::OS` 给的是 `windows`，对不上。
pub fn node_platform_name() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/* ══════════════════════════════════ 路径规范化 ══════════════════════════════════ */

/// 把路径转成可以直接显示、写进 JSON 的字符串。
///
/// Windows 上 `fs::canonicalize` 返回的是 **verbatim 路径**，形如 `\\?\H:\foo`
/// （UNC 则是 `\\?\UNC\server\share`）。直接回给前端会很难看，而且前端拿它去
/// 拼新路径时前缀会跟着扩散。这里统一剥掉。
pub fn clean_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    s
}

/* ══════════════════════════════════ 打开 / 定位 / 删除 ══════════════════════════════════ */

/// 用系统默认程序打开（文件或目录）
pub fn open_path(target: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // explorer 对目录和文件都能处理；不要用 `cmd /c start`，那会弹黑框
        Command::new("explorer").arg(target).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(target).spawn()?;
        Ok(())
    }
}

/**
 * 用**系统默认浏览器**打开一个 URL。
 *
 * 为什么单独一个函数：`open_path` 走的是 `explorer <目标>`，喂 URL 时行为依赖
 * explorer 的 shell 委托，不可靠；这里用 Windows 官方的 URL 协议处理器。
 * 三端各自的写法与 `open_path` 平行。
 */
pub fn open_url(target: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // rundll32 url.dll,FileProtocolHandler 是 shell 打开 URL 的标准做法，
        // **不会弹黑框**（`cmd /c start` 会，见 open_path 的注释）。
        Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(target)
            .spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(target).spawn()?;
        Ok(())
    }
}

/// 在文件管理器里定位到该文件（选中它）
///
/// `select`（试着**选中**该文件而不是只打开它所在的目录）是 Windows / macOS 才有的能力：
/// explorer 认 `/select,`，Finder 认 `-R`。Linux 上 `xdg-open` **只能打开目录**，
/// 没有任何标准方式让文件管理器选中某个条目（不同桌面环境的 DBus 接口各不相同）。
/// 所以那一支显式丢掉 `select` 并注明原因 —— 别在这里假装能做到。
pub fn reveal_in_explorer(target: &str, select: bool) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let mut c = Command::new("explorer");
        if select {
            // /select, 后面**不能有空格**，这是 explorer 的怪癖
            c.arg(format!("/select,{target}"));
        } else {
            c.arg(target);
        }
        c.spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        if select {
            c.arg("-R");
        }
        c.arg(target).spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = select; // 见函数文档：xdg-open 没有「选中」这一说
        let dir = Path::new(target).parent().unwrap_or(Path::new("."));
        Command::new("xdg-open").arg(dir).spawn()?;
        Ok(())
    }
}

/// 移到回收站。做不到时返回错误，让调用方决定是否硬删。
pub fn move_to_trash(target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // 用 Shell.Application 的 Verb「删除」把文件送进回收站。
        // 不引额外的 crate —— PowerShell 一行就够，而且行为可控。
        let script = format!(
            "$p = '{}'; $item = (New-Object -ComObject Shell.Application).Namespace(0).ParseName($p); \
             if ($item) {{ $item.InvokeVerb('delete') }} else {{ throw '找不到项目' }}",
            target.to_string_lossy().replace('\'', "''")
        );
        let out = crate::ipc::tools::quiet_command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                String::from_utf8_lossy(&out.stderr).to_string(),
            ))
        }
    }
    #[cfg(not(windows))]
    {
        // macOS/Linux 没有统一的回收站 API，退回硬删（调用方会在界面上提示）
        if target.is_dir() {
            std::fs::remove_dir_all(target)
        } else {
            std::fs::remove_file(target)
        }
    }
}

/* ══════════════════════════════════ 可执行文件查找 ══════════════════════════════════ */

/// 在 PATH 和给定目录里找一个可执行文件
pub fn find_binary(name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = if cfg!(windows) && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };

    for dir in extra_dirs {
        let p = dir.join(&file);
        if p.is_file() {
            return Some(p);
        }
    }

    // PATH 里找
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(&file);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}
