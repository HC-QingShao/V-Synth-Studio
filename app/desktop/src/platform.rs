//! 平台相关：Windows 特有的是注册表、explorer、回收站；其余走标准库。
//!
//! 这一层是**唯一**放平台代码的地方 —— 移植 macOS 时只需要在这里加一个 cfg 分支，
//! 上层业务代码一行都不用改。（对照 docs/PLATFORM-PORT.md）

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

/* ══════════════════════════════════ 下载目录 ══════════════════════════════════ */

/// 系统的「下载」目录。
///
/// Windows 上 Explorer 会把用户改过的下载路径写进注册表，那是权威来源；
/// 读不到就退回 `%USERPROFILE%\Downloads`。macOS/Linux 直接 `$HOME/Downloads`。
///
/// **绝不返回空串。** 早先的写法是「目录不存在就跳过」——三条来源都不存在时返回 ""，
/// 结果程序没有输出目录，界面上是个空字段，用户完全不知道文件会存到哪。
/// 现在改成：拿到的路径就算还不存在也采用（下载目录本来就可能没建过），
/// 最后兜底到用户主目录，保证调用方永远有一个可用的绝对路径。
pub fn downloads_dir() -> String {
    // 1) 注册表（用户可能把下载目录改到别的盘）
    #[cfg(windows)]
    if let Some(p) = windows_downloads_from_registry() {
        if !p.trim().is_empty() {
            // 不存在就先建出来 —— 不建的话后面写文件会失败
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }

    // 2) 用户主目录下的 Downloads
    if let Some(home) = home_dir() {
        let d = home.join("Downloads");
        if d.is_dir() {
            return d.to_string_lossy().to_string();
        }
        // 3) 主目录本身总存在，用它兜底（下载目录建不出来时的最后退路）
        return home.to_string_lossy().to_string();
    }

    // 4) 连主目录都拿不到（极少见）：用临时目录，至少不是空串
    std::env::temp_dir().to_string_lossy().to_string()
}

pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(windows)]
fn windows_downloads_from_registry() -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ, REG_EXPAND_SZ,
        REG_SZ,
    };

    const SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders";
    // {374DE290-123F-4565-9164-39C4925E467B} 就是「下载」的 KNOWNFOLDERID
    const VALUE: &str = "{374DE290-123F-4565-9164-39C4925E467B}";

    let subkey = to_wide(SUBKEY);
    let value_name = to_wide(VALUE);

    unsafe {
        let mut hkey = std::mem::zeroed();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }

        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let mut ty = 0u32;
        let rc = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut len,
        );
        RegCloseKey(hkey);

        if rc != 0 || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
            return None;
        }

        let n = (len as usize / 2).saturating_sub(1);
        let raw = String::from_utf16_lossy(&buf[..n]);
        Some(expand_env(&raw))
    }
}

/// 展开 `%USERPROFILE%` 这类环境变量
pub fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '%' {
            if let Some(end) = bytes[i + 1..].iter().position(|&c| c == '%') {
                let name: String = bytes[i + 1..i + 1 + end].iter().collect();
                if let Ok(v) = std::env::var(&name) {
                    out.push_str(&v);
                    i += end + 2;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 平台名，用 Node 的命名习惯（`win32` / `darwin` / `linux`）。
///
/// 前端读这个字段来判断「哪些功能在哪些平台可用」，所以要跟 Node 版一致 ——
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

/* ══════════════════════════════════ 文件系统根 ══════════════════════════════════ */

/// 文件选择器的根节点列表，形状对齐 Node 版：
/// `[{name, path, type, parent?}]`
pub fn fs_roots() -> Vec<Value> {
    let mut roots = Vec::new();

    #[cfg(windows)]
    {
        for letter in b'C'..=b'Z' {
            let p = format!("{}:\\", letter as char);
            if Path::new(&p).exists() {
                roots.push(json!({
                    "name": format!("{}:", letter as char),
                    "path": p,
                    "type": "drive",
                }));
            }
        }
    }
    #[cfg(not(windows))]
    {
        roots.push(json!({ "name": "/", "path": "/", "type": "drive" }));
        if let Some(h) = home_dir() {
            roots.push(json!({ "name": "主目录", "path": h.to_string_lossy(), "type": "user" }));
        }
    }

    if let Some(home) = home_dir() {
        for (label, sub) in [
            ("桌面", "Desktop"),
            ("下载", "Downloads"),
            ("文档", "Documents"),
            ("音乐", "Music"),
            ("视频", "Videos"),
        ] {
            let p = home.join(sub);
            if p.exists() {
                let mut item = json!({
                    "name": label,
                    "path": p.to_string_lossy(),
                    "type": "user",
                });
                if label == "桌面" {
                    item["parent"] = json!(home.to_string_lossy());
                }
                roots.push(item);
            }
        }
        // 「用户目录」在 Node 版里排在桌面之后
        roots.push(json!({
            "name": "用户目录",
            "path": home.to_string_lossy(),
            "type": "user",
        }));
    }

    roots
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

/// 看着像 URL 吗（这几个前缀交给系统默认程序，**不做文件存在性检查**）
pub fn looks_like_url(s: &str) -> bool {
    let t = s.trim().to_ascii_lowercase();
    ["http://", "https://", "ftp://", "mailto:"].iter().any(|p| t.starts_with(p))
}
#[cfg(test)]
mod url_tests {
    use super::looks_like_url;

    /// 这个判定决定 `fs_open` 走「开浏览器」还是「开文件」，判错就会把
    /// 一个 URL 当成文件去 `explorer`（或者反过来），所以单独立一条测试。
    #[test]
    fn detects_urls_case_insensitively() {
        assert!(looks_like_url("https://example.com/a"));
        assert!(looks_like_url("HTTPS://EXAMPLE.COM"));
        assert!(looks_like_url("  http://127.0.0.1:17878/api/state  "));
        assert!(looks_like_url("mailto:someone@example.com"));
        assert!(!looks_like_url(""));
        assert!(!looks_like_url("C:\\Music\\a.wav"));
        assert!(!looks_like_url("https:/missing-slash"));
        assert!(!looks_like_url("file:///C:/tmp")); // file:// 有意不认，交给路径分支
    }
}
/// 在文件管理器里定位到该文件（选中它）
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
        let out = crate::server::quiet_command("powershell")
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

/* ══════════════════════════════ 系统文件对话框 ══════════════════════════════════ */

/// 系统「打开」对话框要的几个参数。
pub struct PickOptions {
    /// 过滤器那一行的名字，例如「音频文件」。给了才加这一行。
    pub label: Option<String>,
    /// 过滤器里的扩展名（不带点、小写）。空 = 只有「所有文件」。
    pub exts: Vec<String>,
    pub title: Option<String>,
    /// 打开时停在哪个目录。不传 = 系统自己的记忆。
    pub dir: Option<String>,
    /// 允许多选
    pub multi: bool,
}

/// 对话框的 owner 句柄：**只认本进程的前台窗口**，被别的程序抢了焦点就当没有 owner。
///
/// 对话框要弹在**我们自己窗口**前面。`GetForegroundWindow()` 未必是我们 ——
/// 请求来自页面，发请求的那一刻用户人在窗口里，但被别的程序抢了焦点就会弹到
/// 别人后面去。所以拿到句柄先核 PID：不是本进程就退回无 owner
/// （宁可非模态，也别把对话框挂到别人的窗口上）。
#[cfg(windows)]
fn owner_hwnd() -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    unsafe {
        let h = GetForegroundWindow();
        if h.is_null() {
            return std::ptr::null_mut();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid == std::process::id() {
            h
        } else {
            std::ptr::null_mut()
        }
    }
}

/// 弹一次系统「打开」对话框，回选中的**绝对路径**（取消 → 空数组）。
///
/// 为什么值得走这一步：网页的 `<input type="file">` **拿不到本机路径**，
/// 只给一个 `File` 对象，于是文件必须先整个上传到后端才能用。而这几个页面
/// （音频处理、人声转 MIDI、导入歌词）后面全是「吃路径」的活 —— ffmpeg、
/// 试听（`/api/fs/raw`）、试转 MIDI 都直接读本机文件。走系统对话框等于把
/// 路径直接要过来：不复制字节、不占内存，还能继续「在资源管理器里定位」。
///
/// 拖入那条路没有路径可用，由 `server::simple::fs_upload` 落盘换回一个路径，
/// 两条入口最后都汇成「一串绝对路径」。
///
/// ⚠️ **`OFN_NOCHANGEDIR` 不能省**：进程 cwd 是 `main.rs::resolve_paths()`
/// 的兜底依据（资源目录找不到时按 cwd 往上找），而这套对话框的默认行为是
/// 把调用进程的 cwd 改成用户最后逛到的目录 —— 那之后所有相对路径就全歪了。
///
/// 调用方必须放进 `spawn_blocking`：它一直阻塞到用户点下确定。
#[cfg(windows)]
pub fn pick_files(opts: &PickOptions) -> Result<Vec<String>, String> {
    use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER,
        OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let hwnd = owner_hwnd();

    /*
     * COM：这套对话框不初始化 COM 也能跑，但 shell 那几个扩展点（左侧导航、
     * 快速访问、右键菜单）在没起的线程上会静默失灵 —— 表现就是「对话框能开，
     * 但左边那一栏是空的」。`spawn_blocking` 给的是干净线程，正常回 S_OK；
     * 万一线程已经是 MTA（`RPC_E_CHANGED_MODE`，负数）就**别**配对去
     * `CoUninitialize` —— 那会拆掉别人的 COM。
     */
    // `COINIT_APARTMENTTHREADED` 在 windows-sys 里是 `i32`，而 `CoInitializeEx`
    // 要 `u32`（同一个标志，两套头文件写法），所以这里得转一下。
    let com_ok =
        unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0;

    // 过滤器是**双 NUL 结尾**的一串：`标题\0模式\0…\0\0`。
    let mut filter: Vec<u16> = Vec::new();
    if let Some(label) = opts.label.as_deref().filter(|l| !l.is_empty()) {
        filter.extend(label.encode_utf16());
        filter.push(0);
        let pats: Vec<String> = opts.exts.iter().map(|e| format!("*.{e}")).collect();
        filter.extend(
            if pats.is_empty() { "*.*".to_string() } else { pats.join(";") }.encode_utf16(),
        );
        filter.push(0);
    }
    // 「所有文件」永远留一项：扩展名猜错时（有的 .lrc 被存成 .txt）也得能选到，
    // 否则用户面对的是一个「文件明明在、却点不动」的对话框。
    filter.extend("所有文件".encode_utf16());
    filter.push(0);
    filter.extend("*.*".encode_utf16());
    filter.push(0);
    filter.push(0);

    let title = to_wide(opts.title.as_deref().unwrap_or("选择文件"));
    let initial = opts.dir.as_deref().filter(|d| !d.is_empty()).map(to_wide);

    /*
     * 缓冲区给 32K 个 UTF-16 码元（64 KB）。多选时这里装的是
     * `目录\0文件1\0文件2\0…\0\0`，**几千个文件就可能撑爆** —— Win32 的规矩是
     * 装不下就截断并且返回成功，所以我们拿到的总是「能装下的那部分」，
     * 不会报错。32K 对「拖一批歌进来」这个量级绰绰有余。
     */
    let mut buf = vec![0u16; 32768];

    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = hwnd;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.nFilterIndex = 1;
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    if let Some(dir) = initial.as_ref() {
        ofn.lpstrInitialDir = dir.as_ptr();
    }
    let mut flags = OFN_EXPLORER
        | OFN_FILEMUSTEXIST
        | OFN_PATHMUSTEXIST
        | OFN_HIDEREADONLY
        | OFN_NOCHANGEDIR;
    if opts.multi {
        flags |= OFN_ALLOWMULTISELECT;
    }
    ofn.Flags = flags;

    let picked = unsafe { GetOpenFileNameW(&mut ofn) };
    if com_ok {
        unsafe { CoUninitialize() };
    }

    if picked == 0 {
        /*
         * 返回 0 有两种意思：用户点了取消，或者真出错了。区分它们的唯一办法是
         * `CommDlgExtendedError()` —— 取消时它回 0。混在一起的话，「对话框打不开」
         * 在界面上会表现成「点了没反应」，那种 bug 没法查。
         */
        let code = unsafe { CommDlgExtendedError() };
        if code != 0 {
            return Err(format!("系统文件对话框出错（错误码 {code:#x}）"));
        }
        return Ok(Vec::new());
    }

    // 缓冲区里的第一个字符串：多选时是**目录**，只选一个时是**整条路径**
    // （Win32 就这么定的，不是我们能选的）。
    let first_end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    let first = String::from_utf16_lossy(&buf[..first_end]);

    let mut names: Vec<String> = Vec::new();
    let mut i = first_end + 1;
    while i < buf.len() && buf[i] != 0 {
        let len = buf[i..].iter().position(|&c| c == 0).unwrap_or(0);
        names.push(String::from_utf16_lossy(&buf[i..i + len]));
        i += len + 1;
    }

    if names.is_empty() {
        return Ok(vec![clean_path(Path::new(&first))]);
    }
    let dir = PathBuf::from(&first);
    Ok(names.iter().map(|n| clean_path(&dir.join(n))).collect())
}

/// 非 Windows：还没有系统对话框那一层。让用户手填路径，别假装能用。
#[cfg(not(windows))]
pub fn pick_files(_opts: &PickOptions) -> Result<Vec<String>, String> {
    Err("这个平台还没有系统文件对话框，请直接把路径填进输入框".into())
}

/// 弹一次系统「选择文件夹」对话框，回选中的**目录**（取消 → 空数组）。
///
/// 目录选择以前是**自己画**的（`components/DirPicker.tsx`：面包屑 + 自己列目录 +
/// 新建文件夹 + 就选这里），六个页面各挂一份。系统本来就有这个对话框，还白送
/// 「新建文件夹」「此电脑」「网络位置」和用户自己收藏的快捷方式 —— 用户对自己的
/// 文件管理器比对我们那个列表熟得多。
///
/// 用的是 `SHBrowseForFolderW`（shell32 的老 API，但**确实是系统对话框**）。
/// 没走 `IFileOpenDialog`：那套 COM 接口在 windows-sys 里要自己搬 vtable 调用，
/// 换来的只是新一点的皮；这里真正要解决的是「别自己画一个文件管理器」。
///
/// ⚠️ `BIF_NEWDIALOGSTYLE` 是最要紧的一个 flag：**没有它就没有「新建文件夹」**，
/// 拿到的是又小又旧、不能改大小的那个框。
///
/// 调用方必须放进 `spawn_blocking`：它一直阻塞到用户点下确定。
#[cfg(windows)]
pub fn pick_folder(opts: &PickOptions) -> Result<Vec<String>, String> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows_sys::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BFFM_INITIALIZED, BFFM_SETSELECTIONW, BIF_EDITBOX,
        BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;

    /// `SHBrowseForFolderW` 的回调。`BIF_NEWDIALOGSTYLE` 下它必须是个**真函数**
    /// （新式对话框也是从老接口长出来的），我们只借 `BFFM_INITIALIZED` 那一刻
    /// 把初始目录塞进去，其余一概不管、照常返回 0。
    unsafe extern "system" fn browse_cb(hwnd: HWND, msg: u32, _lp: LPARAM, data: LPARAM) -> i32 {
        if msg == BFFM_INITIALIZED && data != 0 {
            // `wParam = TRUE` 表示「lParam 是一个字符串指针」（FALSE 才是 pidl）
            SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, data);
        }
        0
    }

    let com_ok = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0;

    let title = to_wide(opts.title.as_deref().unwrap_or("选择文件夹"));
    /*
     * 初始目录：宽串的指针要当 `lParam` 交给回调，所以**必须活到对话框结束** ——
     * 绑在函数作用域的变量上，不能是临时值（回调是在对话框里面跑的）。
     */
    let initial = opts.dir.as_deref().filter(|d| !d.is_empty()).map(to_wide);

    // pszDisplayName：系统往里写当前选中项的名字，得给它一块真缓冲区
    let mut shown = vec![0u16; 260];
    let mut bi: BROWSEINFOW = unsafe { std::mem::zeroed() };
    bi.hwndOwner = owner_hwnd();
    bi.pszDisplayName = shown.as_mut_ptr();
    bi.lpszTitle = title.as_ptr();
    bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_EDITBOX;
    if let Some(dir) = initial.as_ref() {
        bi.lpfn = Some(browse_cb);
        bi.lParam = dir.as_ptr() as LPARAM;
    }

    let pidl = unsafe { SHBrowseForFolderW(&bi) };
    let picked = if pidl.is_null() {
        // NULL = 用户取消。这个接口**没有**扩展错误码可查，取消和失败长得一样，
        // 所以一律当「什么都没选」，别把它变成界面上的报错。
        None
    } else {
        let mut buf = vec![0u16; 32768];
        let ok = unsafe { SHGetPathFromIDListW(pidl, buf.as_mut_ptr()) };
        // pidl 是 shell 分配的，必须还回去
        unsafe { CoTaskMemFree(pidl as *const core::ffi::c_void) };
        if ok == 0 {
            // 选中了拿不到文件系统路径的虚拟项（「网络」「此电脑」本身）
            None
        } else {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end]))
        }
    };
    if com_ok {
        unsafe { CoUninitialize() };
    }

    match picked {
        Some(p) if !p.is_empty() => Ok(vec![clean_path(Path::new(&p))]),
        _ => Ok(Vec::new()),
    }
}

/// 非 Windows：同 `pick_files`，让用户手填。
#[cfg(not(windows))]
pub fn pick_folder(_opts: &PickOptions) -> Result<Vec<String>, String> {
    Err("这个平台还没有系统文件夹对话框，请直接把路径填进输入框".into())
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
