//! B 站扫码登录的三条路由。
//!
//! 这里**只做三件事**：收参数、调 `crate::bili` 的真实现、把 Cookie 写进配置。
//! 解析与网络全在 `crate::bili`（`qr_generate` / `qr_poll`），这一层不该有协议细节。
//!
//! ⚠️ **Cookie 绝不回传前端。** 登录态就是账号本身，而这个响应会进前端全局状态、
//! 也常被贴进截图或日志。所以：
//!   * `qr/poll` 成功时只回 `{code, message, loggedIn}` —— `QrPoll::cookie` 在这里
//!     就被吃掉了，前端的 `api.biliQrPoll` 也是照这个形状声明的；
//!   * 写盘用 `simple::save_config`，写进 `bilibiliCookie`（`media.rs` 读的就是这个键）；
//!   * `/api/config` 与 `/api/state` 那边还有一层 `mask_secrets` 把值换成「已设置」。
//!
//! 前端是 `components/ScanLogin.tsx`（`GlassDialog` + `qrcode.react`）。

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use super::{ok, ApiError, AppState};

/// 扫码第一步：申请二维码。
///
/// 回 `{url, qrcodeKey}` —— `url` 由前端画成二维码，`qrcodeKey` 是下一步轮询的令牌。
/// 只有 15 秒超时，所以**没有**必要求助 `spawn_blocking`：这是纯 IO 等待，不占 CPU。
pub async fn qr_generate() -> Result<Json<Value>, ApiError> {
    let (url, key) = crate::bili::qr_generate().await.map_err(ApiError::internal)?;
    Ok(Json(ok(json!({ "url": url, "qrcodeKey": key }))))
}

/// 扫码第二步：问一次「扫了没」。
///
/// ⚠️ **不能回 500**。除成功之外的每一种状态（还没扫 86101 / 扫了待确认 86090 /
/// 已失效 86038）都是**正常流程**，前端要拿 `code` 和 `message` 去更新界面。
/// 真出网络错（超时、DNS）才走 `ApiError`。
///
/// 成功时才 `loggedIn: true`，并把 Cookie 落盘。
pub async fn qr_poll(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let key = body
        .get("qrcodeKey")
        .and_then(|v| v.as_str())
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("缺少 qrcodeKey"))?;

    let r = crate::bili::qr_poll(key).await.map_err(ApiError::internal)?;

    /* 拿到 Cookie = 登录成功。写配置这一步失败要**当成错误报出去**：
       不报的话界面会显示「登录成功」，而实际上画质还是 480P —— 用户只会以为
       登录功能是坏的（和 `bili::qr_poll` 里「code 0 但没捞到 SESSDATA」同一个道理）。 */
    let logged_in = match &r.cookie {
        Some(cookie) => {
            let mut cfg = st.config_snapshot();
            if let Some(map) = cfg.as_object_mut() {
                map.insert("bilibiliCookie".into(), json!(cookie));
            } else {
                cfg = json!({ "bilibiliCookie": cookie });
            }
            super::simple::save_config(&st.writable, &cfg).map_err(|e| {
                ApiError::internal(format!("登录成功，但 Cookie 写不进配置文件：{e}"))
            })?;
            if let Ok(mut guard) = st.config.lock() {
                *guard = cfg;
            }
            true
        }
        None => false,
    };

    Ok(Json(ok(json!({
        "code": r.code,
        "message": r.message,
        "loggedIn": logged_in,
    }))))
}

/// 退出登录：把 `bilibiliCookie` 清空。
///
/// 只清本机配置里那一份 —— 不去调 B 站的退登接口（那会让**别的设备**上的
/// 登录态也失效，代价太大，而用户想要的只是「这台机器别再带着我的登录态」）。
///
/// `config_post` 那条路也能量清它，但设置页需要一个明确的「退出登录」动作，
/// 而且这里回的是 `{loggedOut}`，比「回整份配置」更像一个动作。
pub async fn logout(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let mut cfg = st.config_snapshot();
    let had = cfg
        .get("bilibiliCookie")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if let Some(map) = cfg.as_object_mut() {
        map.insert("bilibiliCookie".into(), json!(""));
    }
    super::simple::save_config(&st.writable, &cfg).map_err(ApiError::from)?;
    if let Ok(mut guard) = st.config.lock() {
        *guard = cfg;
    }
    Ok(Json(ok(json!({ "loggedOut": had }))))
}
