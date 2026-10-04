//! 配置 —— **用户偏好的唯一持久化载体**。
//!
//! 这一层取代了两样东西：
//!
//!   1. 旧前端的 `localStorage`（26 处：主题、玻璃等级、音频/视频/转换页参数、
//!      PV 歌词交接键……）。它有两个顽疾：**按 origin 隔离**（端口一变就"像新用户"）、
//!      以及**只有前端知道**（后端读不到，重启后要等前端灌回来）。
//!   2. 旧 HTTP 的 `GET/POST /api/config`。
//!
//! 现在只有一份真相：`config.json`（绿色版 `<根>/app/data/`，安装版
//! `%APPDATA%\com.qingmu.vocalworkstation\`）。前端启动时一次 `get_config` 拉全量，
//! 改动走 `set_config` 做局部更新。**端口、origin、WebView2 的存储目录都不再影响它。**
//!
//! ⚠️ Cookie 类的值**回显时打码**（`server::simple::mask_secrets`）：
//! 这份数据会进前端全局状态、也常被贴进截图或日志，而它就是账号登录态。
//! 前端把打码后的占位串原样提交回来时，`set_config` 会跳过它 —— 不会出现
//! 「真值被占位串覆盖」这种反向事故。

use serde_json::Value;

use super::Cmd;

/// 读全量配置。Cookie 已打码。
#[tauri::command]
pub async fn get_config(st: super::St<'_>) -> Cmd {
    let cfg = st.config_snapshot();
    Ok(serde_json::json!({ "config": super::config_file::mask_secrets(&cfg) }))
}

/// 局部更新配置（只传要改的键）。
///
/// 回的是**更新后的完整配置**（同样打码）—— 前端拿它直接替换本地那份，
/// 不用再补一次 `get_config`。
#[tauri::command]
pub async fn set_config(st: super::St<'_>, patch: Value) -> Cmd {
    set_config_inner(&st, patch)
}

/// 一次性搬迁：把旧前端留在 `localStorage` 里的设置交上来。
///
/// **为什么需要它**：老用户升级到这一版之后，origin 从
/// `http://127.0.0.1:17878` 变成了 `http://tauri.localhost` —— 那是**两个不同的
/// 存储区**，`localStorage` 里的东西按设计读不到。但 WebView2 的用户数据目录是同一个，
/// 所以旧 origin 下那些键**还在磁盘上**。做法很朴素：前端在**旧的 origin 下**
/// 读一次、交给这个命令、由后端并进 `config.json`。
///
/// ⚠️ 前端只在**第一次**跑新版时调它，并且传 `old` 里只有「config 里还没有的键」——
/// 合并规则是「**已有的不动**」，所以重复调用是幂等的，不会把用户新改的设置顶掉。
///
/// `old` 的形状：`{ "qingmu.theme": "dark", "qingmu.glassLevel": "3", … }`
/// （原样搬 localStorage 的键值对，**映射到配置键的规则写在 ipc/config.rs 的
/// `LEGACY_KEYS` 里** —— 那是一张数据表，不是逻辑）。
#[tauri::command]
pub async fn migrate_legacy_settings(st: super::St<'_>, old: Value) -> Cmd {
    let Some(src) = old.as_object() else {
        return Err("old 应该是一个对象".into());
    };
    if src.is_empty() {
        return Ok(serde_json::json!({ "migrated": 0 }));
    }

    let mut cfg = st.config_snapshot();
    let mut migrated = 0u32;

    for (ls_key, cfg_key) in LEGACY_KEYS {
        let Some(raw) = src.get(*ls_key).and_then(|v| v.as_str()) else {
            continue;
        };
        let val = legacy_value(raw);
        let (Some(map), Some(v)) = (cfg.as_object_mut(), Some(val)) else {
            continue;
        };
        /* **已有的不动** —— 用户升级后可能已经在新界面里改过设置了，
           不能被一份陈旧的 localStorage 覆盖回去。 */
        if map.get(*cfg_key).is_some() {
            continue;
        }
        map.insert((*cfg_key).to_string(), v);
        migrated += 1;
    }

    if migrated > 0 {
        super::config_file::save_config(&st.writable, &cfg).map_err(|e| {
            format!("迁移结果写不进配置文件：{e}")
        })?;
        if let Ok(mut guard) = st.config.lock() {
            *guard = cfg;
        }
        crate::log_line(&format!("已从旧前端的 localStorage 迁来 {migrated} 项设置"));
    }
    Ok(serde_json::json!({ "migrated": migrated }))
}

/// localStorage 键 → 配置键。
///
/// 保留 `fandiao.*` / `qingmu.*` 这些旧名字是**有意**的：它们本来就是老用户
/// 数据里的键名，改掉只会让迁移表更难对。配置键用短名（`theme` / `glassLevel` / …）。
const LEGACY_KEYS: &[(&str, &str)] = &[
    ("qingmu.theme", "theme"),
    ("qingmu.glassLevel", "glassLevel"),
    ("fandiao.audio.settings", "audio"),
    ("fandiao.video.settings", "video"),
    ("fandiao.convert.options", "convert"),
    ("qingmu.pv.lyrics", "pvPendingLyrics"),
];

/// localStorage 里存的是字符串，但有的本来就是 JSON（`{"options":{…}}`）。
///
/// 能解析成 JSON 就按 JSON 存（这样 `config.audio` 是个真对象、前端不用再 parse 一次），
/// 否则当普通字符串。**解析失败不是错误** —— 老数据里什么样都有。
fn legacy_value(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(v @ (Value::Object(_) | Value::Array(_))) => v,
        _ => Value::String(raw.to_string()),
    }
}

/* ══════════════════════════════════ 落盘的实现 ══════════════════════════════════ */

/// 局部更新配置并落盘，回**更新后**的完整配置（已打码）。
///
/// ⚠️ 脱敏字段（以 `Cookie` 结尾的键）收到占位串 `已设置` 时**跳过不写**：
/// 前端拿到的是打码后的值，原样提交回来是正常行为，不能把真值覆盖掉。
fn set_config_inner(st: &super::AppState, patch: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut cfg = st.config_snapshot();
    if let (Some(dst), Some(src)) = (cfg.as_object_mut(), patch.as_object()) {
        for (k, v) in src {
            if k.ends_with("Cookie") && v.as_str() == Some(super::config_file::MASKED) {
                continue;
            }
            dst.insert(k.clone(), v.clone());
        }
    }
    super::config_file::save_config(&st.writable, &cfg).map_err(|e| format!("保存配置失败：{e}"))?;
    if let Ok(mut guard) = st.config.lock() {
        *guard = cfg.clone();
    }
    // 回显也要打码：不然刚保存完 Cookie，明文就从响应里漏回前端了
    Ok(serde_json::json!({ "config": super::config_file::mask_secrets(&cfg) }))
}