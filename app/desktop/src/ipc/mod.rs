//! IPC 命令层 —— **前端唯一能碰到后端的入口**。
//!
//! 这里取代了以前那个同进程的 axum HTTP 服务（9 个文件 / 5,610 行 / 56 条路由 +
//! 静态文件伺服）。前端加载 Tauri 资源协议之后页面就在 Tauri 里，调后端直接
//! `invoke('命令名')`，于是「HTTP 那一层」整体没有了：端口、路由表、JSON 信封、
//! CORS/Range 中间件、静态伺服、SSE、上传、body limit —— 一个都不需要。
//!
//! **搬迁已完成**（2026-10-04）：`src/server/` 与 `axum` / `tower-http` 两个依赖
//! 都已删除，全仓库 `crate::server::` 引用 0 处。下面这条铁律现在是真的。
//!
//! ## 两条铁律
//!
//! 1. **命令只做搬运**：解参数 → 调领域函数（`libresvip.rs` / `lyrics.rs` / `bili.rs` /
//!    `ytdlp.rs` / `audio.rs` / `svsep.rs` / `midi_transcribe.rs` / `platform.rs`）→ 回值。
//!    **任何业务逻辑都不许写在这一层。**
//!    ⚠️ 这条以前带个「搬迁期可以借 server/」的例外，**那个例外已经作废** ——
//!    `server/` 不存在了，这里就是唯一实现。**别在这里重新实现一遍业务**，
//!    也**别为了「顺手起个 HTTP 端点」把 `axum` 加回来**：前端加载的是 Tauri 的
//!    资源协议，页面里根本没有 HTTP 服务器的位置。
//!
//! 2. **命令名全局唯一**：`tauri::generate_handler!` 里的名字不按模块作用域，
//!    两个模块各有一个 `status` 就会撞名。所以带前缀：`svsep_*` / `midi_*` /
//!    `lyrics_*` / `video_*` / `audio_*` / `bili_*` / `convert_*` / `pv_*`。
//!    ⚠️ 还有一个更隐蔽的后果：`#[tauri::command]` 会为命令名生成同名包装项，
//!    所以**别的模块别按那个名字去调同名逻辑函数** —— 会解析到宏生成的那个，
//!    报出「expected `State<Arc<AppState>>`, found `&Arc<AppState>`」这种看不懂的错。
//!    真踩过（`jobs::cancel_job` 同时是命令 + 逻辑函数），解法是在调用方写个私有助手。
//!
//! ## 注册点只有一个
//!
//! `main.rs` 的 `generate_handler!` 是唯一清单（当前 **70 条**）。加命令要**两处一起改**：
//! 这里写函数、`main.rs` 登记。漏登记的表现是「前端调用报 command not found」，不报编译错。
//!
//! ## 与「响应形状」有关的约定
//!
//! 不再有 `{ok:true,...}` 信封 —— IPC 用 `Result<T, E>` 表达成败。出错时返回
//! `Err(String)`，前端 `invoke` 把它抛成 `Error`，`lib/ipc.ts` 的 `call()` 统一成人话。
//! 成功时返回的 `Value` **仍然是 `server/` 时代那个 JSON 对象**（只是因为 `ok` 信封
//! 已经由 `Result` 承担，里面不再有那个键）—— 这样第 3 步换前端时形状是对得上的。

pub mod bili;
pub mod config;
pub mod config_file;
pub mod convert;
pub mod fs;
pub mod jobs;
pub mod lyrics;
pub mod media;
pub mod midi;
pub mod pv;
pub mod state;
pub mod svsep;
pub mod tools;

use std::sync::Arc;

pub use crate::ipc::state::AppState;

/// IPC 命令的返回类型。
///
/// 成功给 `serde_json::Value`（Tauri 直接序列化给前端），失败给一句**给用户看的人话**。
/// 刻意不用 `ApiError` —— 那是 HTTP 层的类型，带着状态码，而 IPC 没有状态码这个概念。
pub type Cmd = Result<serde_json::Value, String>;


/// 命令体里常见的「拿状态」：`State<Arc<AppState>>`。
///
/// Tauri 的 `State<T>` 要求 `T: Send + Sync + 'static`，`Arc<AppState>` 满足；
/// 状态是在 `main.rs` 的 setup 里 `app.manage()` 进来的。
pub type St<'a> = tauri::State<'a, Arc<AppState>>;

/// 让 `Arc` 的导入在搬迁期不报未使用。
#[allow(dead_code)]
type _KeepArc = Arc<()>;
