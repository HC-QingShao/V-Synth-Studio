//! 配置 —— **用户偏好的唯一持久化载体**。
//!
//! 只有一份真相：`config.json`（绿色版 `<根>/data/`，安装版
//! `%APPDATA%\com.qingmu.vocalworkstation\`）。前端启动时一次 `get_config` 拉全量，
//! 改动走 `set_config` 做局部更新。**端口、origin、WebView2 的存储目录都不影响它。**
//!
//! ⚠️ Cookie 类的值**回显时打码**（`config_file::mask_secrets`）：
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

/// 把这些设置并进 `config.json`。`old` 的形状：`{ "qingmu.theme": "dark", … }`
/// （原样搬 localStorage 的键值对，**映射到配置键的规则写在 `LEGACY_KEYS` 里**
/// —— 那是一张数据表，不是逻辑）。
///
/// **为什么需要它**：`localStorage` **按 origin 隔离**，而 WebView2 的用户数据目录
/// 是同一个 —— 旧 origin 下那些键**还在磁盘上**，但要由前端在**旧 origin 下**读一次
/// 交上来。合并规则是「**已有的不动**」，所以调用幂等，不会把用户新改的设置顶掉。
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
        /* **已有的不动** —— 用户可能已经在新界面里改过设置了，
        不能被一份陈旧的 localStorage 覆盖回去。 */
        if map.get(*cfg_key).is_some() {
            continue;
        }
        map.insert((*cfg_key).to_string(), v);
        migrated += 1;
    }

    if migrated > 0 {
        super::config_file::save_config(&st.writable, &cfg)
            .map_err(|e| format!("迁移结果写不进配置文件：{e}"))?;
        if let Ok(mut guard) = st.config.lock() {
            *guard = cfg;
        }
        crate::log_line(&format!("已从旧前端的 localStorage 迁来 {migrated} 项设置"));
    }
    Ok(serde_json::json!({ "migrated": migrated }))
}

/// localStorage 键 → 配置键。
///
/// ⚠️ **左侧必须是用户数据里真实存在的键名**（`fandiao.*` / `qingmu.*`），
/// 改掉只会让这张表对不上；右侧配置键用短名（`theme` / `glassLevel` / …）。
const LEGACY_KEYS: &[(&str, &str)] = &[
    ("qingmu.theme", "theme"),
    ("qingmu.glassLevel", "glassLevel"),
    ("fandiao.audio.settings", "audio"),
    ("fandiao.video.settings", "video"),
    ("fandiao.convert.options", "convert"),
    /* 页面把这条写成 `fandiao.convert.settings`（两种写法并存）。两条都收下：
    哪条在就搬哪条，两条都在时后一条赢。 */
    ("fandiao.convert.settings", "convert"),
    ("qingmu.pv.lyrics", "pvPendingLyrics"),
    ("qingmu.pv.sent", "pvSentLyrics"),
    ("qingmu.midi.outDir", "midiOutDir"),
];

/// localStorage 里存的是字符串，但有的本来就是 JSON（`{"options":{…}}`）。
///
/// 能解析成 JSON 就按 JSON 存（这样 `config.audio` 是个真对象、前端不用再 parse 一次），
/// 否则当普通字符串。**解析失败不是错误**。
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
fn set_config_inner(st: &super::AppState, patch: Value) -> Result<Value, String> {
    let mut cfg = st.config_snapshot();
    if let (Some(dst), Some(src)) = (cfg.as_object_mut(), patch.as_object()) {
        for (k, v) in src {
            if k.ends_with("Cookie") && v.as_str() == Some(super::config_file::MASKED) {
                continue;
            }
            dst.insert(k.clone(), v.clone());
        }
    }
    super::config_file::save_config(&st.writable, &cfg)
        .map_err(|e| format!("保存配置失败：{e}"))?;
    if let Ok(mut guard) = st.config.lock() {
        *guard = cfg.clone();
    }
    // 回显也要打码：不然刚保存完 Cookie，明文就从响应里漏回前端了
    Ok(serde_json::json!({ "config": super::config_file::mask_secrets(&cfg) }))
}
