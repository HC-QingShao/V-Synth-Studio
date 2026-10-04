import { useCallback, useSyncExternalStore } from 'react'
import { type GlassMaterial } from '@/components/Glass'
import { getConfig, saveConfig } from '@/lib/config'

/**
 * **玻璃等级 1~4** —— 一个滑块管住原来三个开关：
 *
 * | 等级 | 名字 | 材质 | 透明度 | 内容面板 |
 * |---|---|---|---|---|
 * | 1 | 关 | — | `opaque`（库画不透明底、不做模糊） | 轻量材质 |
 * | 2 | 毛玻璃 | `regular` | `system` | 轻量材质 |
 * | 3 | 液态玻璃 | `clear` + 折射 | `system` | 轻量材质（← 用户说的「一半液态玻璃」） |
 * | 4 | 全液态 | `clear` + 折射 | `system` | 玻璃面，也折射 |
 *
 * ⚠️ **3 级和 4 级的区别只有「内容面板算不算玻璃」。** 上一版只有 3 档，
 * 把「栏 / 侧栏 / 控件折射、面板仍是轻量材质」那一档挤掉了 —— 用户报
 * 「之前有个一半液态玻璃的效果没了」。级别**单调递增**：往上走只会更玻璃，代价更大。
 *
 * ⚠️ **必须用 `useSyncExternalStore`，不能用 `useState`。**
 * 踩过：钩子里用 `useState`、再被两个组件各调一次，两个组件各拿一份独立 state ——
 * 值改了、DOM 没变，看着就是「改了没有任何用」。
 *
 * ⚠️ 存的地方是 **`config.json` 的 `glassLevel`**（旧键 `qingmu.glassLevel`，
 * 由 `lib/config.ts` 的迁移搬过来）。所以 `set()` 写的是后端，不是浏览器存储。
 */
export type GlassLevel = 1 | 2 | 3 | 4

export const GLASS_LEVELS: { level: GlassLevel; label: string; desc: string }[] = [
  {
    level: 1,
    label: '关',
    desc: '不透明底色，不做模糊。文字对比最高，低端机、远控桌面选这档',
  },
  { level: 2, label: '毛玻璃', desc: '栏、侧栏、控件模糊提色；内容面板仍是轻量材质' },
  {
    level: 3,
    label: '液态玻璃',
    desc: '上面那些改成折射（边缘把背后折弯）；内容面板还是轻量材质',
  },
  {
    level: 4,
    label: '全液态',
    desc: '连内容面板也折射 —— 每个玻璃面都会多一层 SVG 位移贴图，开销最大',
  },
]

/** 等级 → 库的材质参数（前两级毛玻璃，后两级液态） */
export function levelMaterial(level: GlassLevel): GlassMaterial {
  return level >= 3 ? 'liquid' : 'frosted'
}

/** 等级 → 库的透明度策略（1 级走 `opaque`：库会关掉模糊、改画不透明底） */
export function levelTransparency(level: GlassLevel): 'opaque' | 'system' {
  return level === 1 ? 'opaque' : 'system'
}

/** 等级 → 内容区的面板要不要玻璃面（**只有最高的 4 级要**） */
function levelGlobalGlass(level: GlassLevel): boolean {
  return level >= 4
}

const DEFAULT_LEVEL: GlassLevel = 2
const listeners = new Set<() => void>()
let current: GlassLevel | null = null

/**
 * 从**配置**读玻璃等级（键 `glassLevel`，旧键是 `qingmu.glassLevel`）。
 *
 * ⚠️ **不要给 3 档时代的旧值做映射。** 第一版把老 3 级（液态 + 面板玻璃）映射成新的
 * 4 级，结果每次读都把新选的 3 级顶成 4 级 —— 用户根本选不中 3 级（实测：写 3、读出来是 4）。
 * 3 档只存在过一个 commit，直接按新语义读就行。
 *
 * （老用户那两个更早的键 `qingmu.globalGlass` + `qingmu.glass` 由启动时那次
 * `migrate_legacy_settings` 处理，见 `lib/config.ts` —— 这里不再认识它们。）
 */
function readLevel(): GlassLevel {
  const raw = Number(getConfig().glassLevel)
  if (raw === 1 || raw === 2 || raw === 3 || raw === 4) return raw as GlassLevel
  return DEFAULT_LEVEL
}

function get(): GlassLevel {
  if (current === null) current = readLevel()
  return current
}

/** 配置换了（启动时那一次 `get_config` 落定，或别处改了）就重读一遍。 */
export function syncGlassLevel() {
  const next = readLevel()
  if (current === next) return
  current = next
  for (const l of listeners) l()
}

function set(level: GlassLevel) {
  if (current === level) return
  current = level
  saveConfig({ glassLevel: level })
  for (const l of listeners) l()
}

function subscribe(cb: () => void) {
  listeners.add(cb)
  return () => {
    listeners.delete(cb)
  }
}

export function useGlassLevel() {
  const level = useSyncExternalStore(subscribe, get, () => 2 as GlassLevel)
  const setLevel = useCallback((l: GlassLevel) => set(l), [])
  return { level, setLevel }
}

/* ── 下面两个是给不认识「等级」的地方用的派生值 ───────────────────── */

/** 材质（`components/Glass.tsx` 按它取折射参数） */
export function useMaterial() {
  return { material: levelMaterial(useGlassLevel().level) }
}

/** 内容面板要不要玻璃面（`components/Panel.tsx`） */
export function useGlobalGlass() {
  return { globalGlass: levelGlobalGlass(useGlassLevel().level) }
}
