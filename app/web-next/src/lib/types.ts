/**
 * 后端数据结构 —— **手写**，照 `ipc/state.rs::get_state` 那个 `json!({...})` 的真实形状。
 *
 * ⚠️ **不引 `ts-rs` 之类的构建期类型生成**（用户已拍板）：那几个类型就这么点，
 * 生成器带来的构建负担比省下的手写更大。改后端回包时**两处一起改**。
 *
 * ⚠️ 已经没有那条 health 路由了（`HealthInfo` 随之删掉）：`pid` / `uptimeSec` 是
 * 「跑在一个 HTTP 端口上」才需要的信息，IPC 下前端与 Rust 同进程同生命周期 ——
 * 版本号、运行环境、安装形态都在 `get_state` 里。
 */

/** 一种工程格式（40 种） */
export interface FormatInfo {
  id: string
  name: string
  exts: string[]
  /** 分组名，中文，直接当标题用 */
  group: string
  /** 模块是否就绪；false 时界面要置灰 */
  available: boolean
  canRead: boolean
  canWrite: boolean
  /** 仅 available 时有 */
  fidelity?: { notes?: string }
  /** 仅 !available 时有，说明为什么没就绪 */
  reason?: string
}

/** 外部工具检测结果（ffmpeg / yt-dlp / python） */
export interface ToolInfo {
  available: boolean
  path?: string
  version?: string
  /** '程序目录' / 'PATH' 等，界面上说明来源 */
  source?: string
  kind?: string
}

interface ToolsMap {
  ffmpeg?: ToolInfo
  ytdlp?: ToolInfo
  python?: ToolInfo
  [k: string]: ToolInfo | undefined
}

export interface AppState {
  formats: FormatInfo[]
  /** 编辑器探测的候选表（`tools.rs` 的候选已经清空，正常是空数组） */
  editors: unknown[]
  tools: ToolsMap
  transformOps: unknown[]
  audioFormats: Record<string, unknown>
  config: Record<string, unknown>
  paths: { root?: string; outputDir?: string; downloadDir?: string; toolsDir?: string }
  pinyin: Record<string, unknown>
  /** `win32` / `darwin` 这种（Node 命名，跨平台代码沿用了） */
  platform: string
  /** 给人看的一行，如 `Windows (x86_64)` */
  platformDesc?: string
  version: string
  /** 作者署名 */
  author?: string
  /** 安装版（Program Files）还是绿色版（解压即用） */
  installed?: boolean
}

/**
 * 后端任务（`ipc/jobs.rs`）。
 *
 * 形状照建任务时那个 `json!({...})`：
 * `{ id, type, title, status, percent, message, logs, createdAt }`，失败时多一个 `error`。
 * **`percent` 是 0~100**（不是 0~1）—— `GlassProgress` 要 `value`/`total`，直接给 100 当 total。
 */
export interface Job {
  id: string
  type?: string
  title?: string
  status: 'running' | 'done' | 'error' | 'canceled'
  percent?: number
  message?: string
  logs?: string[]
  error?: string
  createdAt?: number
  [k: string]: unknown
}
