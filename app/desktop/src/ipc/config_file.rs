//! 配置文件的读写 —— **用户偏好的唯一真相**。
//!
//! 从 `server/simple.rs` 搬过来的（那一层要整个删掉）。搬的是**领域逻辑**：
//! 默认值表、路径、老目录迁移、落盘。原来它们跟 HTTP 处理器混在一个文件里，
//! 现在按用途分开 —— 这一份不认识 axum，也不认识 IPC。
//!
//! 两个形态共用一套代码，差别只在 `writable` 指向哪：
//!   * **绿色版**：`<根>/app/data/config.json`（整个目录可以拷着走）
//!   * **安装版**：`%APPDATA%\com.qingmu.vocalworkstation\config.json`
//!     （Program Files 只读，写那儿要管理员权限）
//! 判据是「能不能写」，见 `main.rs::resolve_paths`。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// 对外显示的版本号。
///
/// Cargo 的 `version` 必须是合法 semver（`1.3.0`），但那个字符串给人看太啰嗦。
/// 界面上要的是 `1.3.1beta` 这种写法，所以单独列一个常量 —— **改版本号时五处都要改**：
/// 这里、`Cargo.toml`、`tauri.conf.json`、`app/web-next/package.json`，
/// 以及两条锁文件记录（跑 `cargo update -p v-synth-studio --precise <版本>` 与
/// `npm install --package-lock-only` 让它们自己跟上，别手改）。
///
/// ⚠️ **这里可以带 `beta` 后缀，那四处不行**：Cargo / npm / MSI 的 ProductVersion 都只吃
/// 合法 semver（`1.3.1`），`1.3.1beta` 不是合法 semver。所以「带 beta」这件事分两层：
/// 界面上是 `APP_VERSION`（自由字符串，想怎么写就怎么写），
/// 安装包文件名由 CI 在打包后改名补上 beta（见 `.github/workflows/build-msi.yml`）。
pub const APP_VERSION: &str = "1.3.1beta";

/// 作者标识。出现在「关于」里，也散落在源码注释中作为出处水印。
pub const AUTHOR_TAG: &str = "QingMu39";

/// 「关于」里那行运行环境，例如 `Windows (x86_64)`。
pub fn platform_desc() -> String {
    format!(
        "{} ({})",
        crate::platform::node_platform_name(),
        std::env::consts::ARCH
    )
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 配置默认值。
///
/// ⚠️ **只有这里列出来的键会被 `load_config` 认**（它按默认值的键逐个取）。
/// 也就是说：**删掉一个键 = 老用户配置里的残留会在下次 `save_config` 整份回写时被清掉**
/// —— 这是有意的（`qqCookie` 当年就是这么退役的）。反过来，加键一定要在这里加，
/// 不然前端存进去的值下次启动就没了。
pub fn default_config() -> Value {
    json!({
        "theme": "system",
        "glassLevel": 2,
        "outputDir": "",
        "downloadDir": "",
        "bilibiliCookie": "",
        "neteaseCookie": "",
        "proxy": "",
        /* ── 下面这几项后端不读，但**必须在这儿列出来** ──────────────────────
         *
         * 它们原来是前端 `localStorage` 里的页面设置（2026-10-04 搬进来）。
         * 为什么不列不行：`load_config` 只认**默认值里有的键**（见上面那条注释），
         * 所以没列出来的键就算 `set_config` 写进了盘，下次启动也会被过滤掉 ——
         * 表现是「设置改了、重启就没了」，而且不报任何错。
         *
         * 也就是说：**前端每多存一项设置，这里就要多一个键。** */
        "audio": {},
        "video": {},
        "convert": {},
        // 歌词页 → 文字 PV 页的交接（原 `qingmu.pv.lyrics` / `qingmu.pv.sent`）
        "pvPendingLyrics": "",
        "pvSentLyrics": "",
        // 人声转 MIDI 的输出目录（原 `qingmu.midi.outDir`）
        "midiOutDir": "",
        /* 音轨分离运行时的落点（空 = 由后端按「程序目录能不能写」自己定）。
           为什么要给用户选：那一坨 4.7 GB / 解压后 7.4 GB，装在 C 盘紧张的人身上是灾难；
           而安装版默认落在 Program Files 下**根本写不进去**（见 `svsep::runtime_base`）。 */
        "svsepRuntimeDir": "",
        /* 显卡加速（DirectML）：`auto`（默认，非 N 卡就开）/ `on` / `off`；
           `svsepDmlSix` = 六轨（RoFormer）也走 DirectML —— 上游怕爆显存写死了 False，
           我们替它打开这条路，界面必须提示「建议显存 ≥ 8 GB」。 */
        "svsepDml": "auto",
        "svsepDmlSix": false,
    })
}

fn config_path(writable: &Path) -> PathBuf {
    writable.join("config.json")
}

/// 读配置。
///
/// 读不到 / 解析失败都回默认值（**不是错误**：装完第一次跑就没有这个文件）。
/// 但解析失败要记一行日志 —— 用户手改坏了 JSON 时，那句「已回落默认值」是唯一的线索。
pub fn load_config(writable: &Path) -> Value {
    let mut base = default_config();
    match std::fs::read_to_string(config_path(writable)) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(saved) => {
                if let (Some(dst), Some(src)) = (base.as_object_mut(), saved.as_object()) {
                    for (k, v) in src {
                        /* 只认默认值里有的键（见 `default_config` 的注释） */
                        if dst.contains_key(k) {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            Err(e) => crate::log_line(&format!(
                "配置读取失败，已回落默认值：{} —— {e}。请检查这个文件是不是合法 JSON。",
                config_path(writable).display()
            )),
        },
        Err(_) => { /* 首次运行：没有配置文件是正常的 */ }
    }
    base
}

/// 落盘。父目录不存在会先建出来（首次运行安装版就是这样）。
pub fn save_config(writable: &Path, cfg: &Value) -> std::io::Result<()> {
    let p = config_path(writable);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(cfg).unwrap_or_default())
}

/// Cookie 类配置回显时的占位串。前端认它：看到「已设置」就只当有值，不会把它提交回来。
pub const MASKED: &str = "已设置";

/// 把 Cookie 的值换成「已设置」再回给前端。
///
/// 为什么：`get_config` 与 `get_state` 的结果会进前端全局状态，也常被贴进截图或日志，
/// 而 Cookie 就是账号登录态 —— 回显真值等于把账号摊开。前端把占位串原样提交回来时
/// `ipc::config::set_config` 会跳过它，所以不会出现「真值被占位串覆盖」这种反向事故。
pub fn mask_secrets(cfg: &Value) -> Value {
    let mut out = cfg.clone();
    if let Some(map) = out.as_object_mut() {
        for (k, v) in map.iter_mut() {
            if k.ends_with("Cookie") && v.as_str().is_some_and(|s| !s.is_empty()) {
                *v = json!(MASKED);
            }
        }
    }
    out
}
