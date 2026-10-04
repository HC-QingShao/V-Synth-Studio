/**
 * **配置** —— `config.json` 在前端的门面。
 *
 * 2026-10-04 起前端不再用**浏览器存储**存任何设置：用户的偏好只有一份真相，
 * 就是后端那个 `config.json`（绿色版 `<根>/app/data/config.json`，
 * 安装版 `%APPDATA%\com.qingmu.vocalworkstation\config.json`）。
 * 这么做解决了浏览器存储的两个顽疾：**按 origin 隔离**（端口一改就像换了新用户）
 * 和**只有前端知道**（后端读不到，重启后要等前端灌回来）。
 *
 * ⚠️ **本文件是整份前端里唯一还碰浏览器存储的地方** —— 而且只碰一次：启动时把旧键
 * 捞出来交给后端的 `migrate_legacy_settings`（见下面 `readLegacy()`）。
 * 别在别处再写 `localStorage.setItem`，那条路已经整体退役了。
 *
 * ## 时序
 *
 * `main.tsx` 在挂载 React **之前** `await ensureConfig()` —— `get_config` 只读一个
 * 几百字节的文件，比首屏渲染快得多，等它一下换来的是「第一帧就是用户选的主题」。
 * 真出错也不拦着界面出来（回落到默认值，由 `onConfigError` 报一句）。
 *
 * ## 写回
 *
 * 改一项就 `saveConfig(patch)`：内存里立刻生效（订阅者马上重渲染），落盘走
 * 250ms 防抖 —— 滑块那种连续改动不该每一帧都写一次文件。后端的 `set_config`
 * 本身也是**局部更新**（只传要改的键），并且回的是更新后的完整配置。
 */

import { useCallback, useSyncExternalStore } from 'react'
import { call } from './ipc'

/** `config.json` 的形状。后端 `default_config()` 里有的键就是这些（加了新键要两边一起加）。 */
export interface AppConfig {
  theme?: 'system' | 'light' | 'dark'
  glassLevel?: number
  outputDir?: string
  downloadDir?: string
  bilibiliCookie?: string
  neteaseCookie?: string
  proxy?: string
  /* ── 页面自己那份设置（旧前端把它们放在浏览器存储里，键名见 `readLegacy`）── */
  audio?: Record<string, unknown>
  video?: Record<string, unknown>
  convert?: Record<string, unknown>
  /** 歌词页 → 文字 PV 页的交接（原来的 `qingmu.pv.lyrics`） */
  pvPendingLyrics?: string
  /** 上面那份已经填过一次了，别重复填（原来的 `qingmu.pv.sent`） */
  pvSentLyrics?: string
  /** 人声转 MIDI 的输出目录（原来的 `qingmu.midi.outDir`） */
  midiOutDir?: string
  [k: string]: unknown
}

/* ══════════════════════════════════════ 模块级状态 ══════════════════════════════════════ */

let snapshot: AppConfig = {}
const listeners = new Set<() => void>()
let booted: Promise<AppConfig> | null = null
let errorHandler: ((msg: string) => void) | null = null

function emit() {
  for (const l of listeners) l()
}

function subscribe(cb: () => void) {
  listeners.add(cb)
  return () => {
    listeners.delete(cb)
  }
}

/** 同步读当前配置（不认识 React 的地方用，比如 `useGlass` 的存储层）。 */
export function getConfig(): AppConfig {
  return snapshot
}

/** 界面上报错的地方（App 挂载时接上 toast）。没接就只进控制台。 */
export function onConfigError(fn: ((msg: string) => void) | null) {
  errorHandler = fn
}

function fail(msg: string) {
  console.error('[config]', msg)
  errorHandler?.(msg)
}

/* ══════════════════════════════════════ 读 ══════════════════════════════════════ */

interface ConfigReply {
  config: AppConfig
}

/**
 * 旧前端的**浏览器存储**键 —— 整份前端里唯一读它们的地方（迁移用）。
 *
 * ⚠️ **只负责把它们捞出来，映射规则在后端**（`ipc/config.rs` 的 `LEGACY_KEYS`）
 * —— 那张表是唯一的映射处，这里再抄一遍就会两边不同步。
 *
 * ⚠️ **老用户读到的是空的**：那些键在旧 origin（`http://127.0.0.1:17878`）下，
 * 而现在的窗口是 `http://tauri.localhost`，两个 origin 的存储区不互通。
 * 这个函数真正能捞到东西的场合是「跑过中间那版、键写在 tauri.localhost 下」——
 * 那正是升级路径上的每一个人。捞到就交给后端并进去，捞不到就什么都不做。
 */
function readLegacy(): Record<string, string> {
  const out: Record<string, string> = {}
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i)
      if (!k || !(k.startsWith('qingmu.') || k.startsWith('fandiao.'))) continue
      const v = localStorage.getItem(k)
      if (v) out[k] = v
    }
  } catch {
    /* 隐私模式：当没有旧数据 */
  }
  return out
}

/**
 * 启动时那一次：拉配置 → 搬旧设置 → 再拉一次。
 *
 * `migrate_legacy_settings` 的合并规则是「**配置里已有的键优先**」，所以它**幂等**，
 * 多调几次无害 —— 前端不必（也无法）记「搬没搬过」的标记，那个标记本身就得存下来，
 * 正是我们刚删掉的东西。
 */
async function boot(): Promise<AppConfig> {
  let cfg: AppConfig
  try {
    cfg = (await call<ConfigReply>('get_config')).config ?? {}
  } catch (e) {
    fail(`读取配置失败：${e instanceof Error ? e.message : String(e)}`)
    snapshot = {}
    emit()
    return snapshot
  }

  const old = readLegacy()
  if (Object.keys(old).length) {
    try {
      await call('migrate_legacy_settings', { old })
      cfg = (await call<ConfigReply>('get_config')).config ?? cfg
    } catch (e) {
      /* 搬不动不是致命的：用户在新界面里重设一次就行，别拦着启动 */
      console.warn('[config] 旧设置迁移失败：', e)
    }
    /* ⚠️ `theme` / `glassLevel` 这两个**轮不到后端那张表**：它们在
       `default_config()` 里本来就有默认值，而迁移的规则是「已有的键优先」——
       `cfg.get(key).is_some()` 恒为真，于是永远跳过。所以这里补一刀：
       **只有还停在默认值上**才把旧值提上来（用户已经在新界面改过就不动他）。 */
    const patch: Record<string, unknown> = {}
    if (cfg.theme === 'system' && old['qingmu.theme']) {
      const t = old['qingmu.theme']
      if (t === 'light' || t === 'dark' || t === 'system') patch.theme = t
    }
    if ((cfg.glassLevel ?? 2) === 2 && old['qingmu.glassLevel']) {
      const n = Number(old['qingmu.glassLevel'])
      if (n >= 1 && n <= 4) patch.glassLevel = Math.round(n)
    }
    if (Object.keys(patch).length) {
      try {
        cfg = (await call<ConfigReply>('set_config', { patch })).config ?? cfg
      } catch (e) {
        console.warn('[config] 旧主题 / 玻璃等级没能提上来：', e)
      }
    }
  }

  snapshot = cfg
  emit()
  return snapshot
}

/** 启动时调一次（`main.tsx`）。重复调用拿到的是同一个 Promise。 */
export function ensureConfig(): Promise<AppConfig> {
  if (!booted) booted = boot()
  return booted
}

/* ══════════════════════════════════════ 写 ══════════════════════════════════════ */

let pending: Record<string, unknown> = {}
let timer: number | null = null

async function flush() {
  timer = null
  const patch = pending
  pending = {}
  if (!Object.keys(patch).length) return
  try {
    const r = await call<ConfigReply>('set_config', { patch })
    /* 后端回的是**打码后**的完整配置，用它替掉本地那份 —— 这样 Cookie 类的键
       在内存里也不会留明文。（打码的值原样提交回来时后端会跳过，不会反向覆盖。） */
    if (r?.config) {
      snapshot = r.config
      emit()
    }
  } catch (e) {
    fail(`保存设置失败：${e instanceof Error ? e.message : String(e)}`)
  }
}

/**
 * 存几项设置。**立刻**在内存里生效（订阅者下一帧就重渲染），落盘防抖 250ms。
 */
export function saveConfig(patch: Record<string, unknown>): void {
  snapshot = { ...snapshot, ...patch }
  emit()
  pending = { ...pending, ...patch }
  if (timer !== null) window.clearTimeout(timer)
  timer = window.setTimeout(() => void flush(), 250)
}

/* ══════════════════════════════════════ React ══════════════════════════════════════ */

/** 整份配置（会跟着 `saveConfig` 重渲染）。 */
export function useConfig(): AppConfig {
  return useSyncExternalStore(subscribe, getConfig, getConfig)
}

/**
 * 取一项设置并拿到它的 setter —— 取代原来满页的「读一项、写一项」。
 *
 * ⚠️ `fallback` 传**模块级常量**（尤其对象/数组），别在渲染里现造一个字面量：
 * 配置里暂时没有这个键时，每次渲染都会得到一个新引用。
 */
export function useConfigValue<T>(key: string, fallback: T): [T, (v: T) => void] {
  const cfg = useConfig()
  const value = (cfg[key] as T | undefined) ?? fallback
  const set = useCallback((v: T) => saveConfig({ [key]: v }), [key])
  return [value, set]
}
