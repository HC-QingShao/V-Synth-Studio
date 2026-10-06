import {useCallback, useSyncExternalStore} from 'react'
import {type GlassMaterial} from '@/components/Glass'
import {getConfig, saveConfig} from '@/lib/config'

/**
 * **玻璃等级 1~4** —— 一个滑块管住材质、透明度、内容面板三件事：
 *
 * | 等级 | 名字 | 材质 | 透明度 | 内容面板 |
 * |---|---|---|---|---|
 * | 1 | 关 | — | `opaque`（库画不透明底、不做模糊） | 轻量材质 |
 * | 2 | 毛玻璃 | `regular` | `system` | 轻量材质 |
 * | 3 | 液态玻璃 | `clear` + 折射 | `system` | 轻量材质（栏 / 侧栏 / 控件折射，面板仍轻量） |
 * | 4 | 全液态 | `clear` + 折射 | `system` | 玻璃面，也折射 |
 *
 * ⚠️ **3 级和 4 级的区别只有「内容面板算不算玻璃」。** 级别**单调递增**：往上走只会更玻璃。
 *
 * ⚠️ **必须用 `useSyncExternalStore`，不能用 `useState`。** 钩子被两个组件各调一次时，
 * `useState` 会给出两份独立 state —— 值改了、DOM 没变，看着就是「改了没有任何用」。
 *
 * ⚠️ 存的地方是 **`config.json` 的 `glassLevel`**：`set()` 写的是后端，不是浏览器存储。
 */
export type GlassLevel = 1 | 2 | 3 | 4

export const GLASS_LEVELS: { level: GlassLevel; label: string; desc: string }[] = [
    {
        level: 1,
        label: '关',
        desc: '不透明底色，不做模糊。文字对比最高，低端机、远控桌面选这档',
    },
    {level: 2, label: '毛玻璃', desc: '栏、侧栏、控件模糊提色；内容面板不变'},
    {
        level: 3,
        label: '液态玻璃',
        desc: '栏与控件改用折射，内容面板不变',
    },
    {
        level: 4,
        label: '全液态',
        desc: '连内容面板也折射，开销最大',
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
 * 从**配置**读玻璃等级（键 `glassLevel`）。
 *
 * ⚠️ **不要加档位映射**：读出的值直接按 1~4 用。任何把某一档往上顶的映射都会让那一档
 * 永远选不中（写 3、读出来是 4）。只有配置里那个键算数，别处送来的旧键由
 * `lib/config.ts` 的启动迁移处理。
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
    saveConfig({glassLevel: level})
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
    return {level, setLevel}
}

/* ── 下面两个是给不认识「等级」的地方用的派生值 ───────────────────── */

/** 材质（`components/Glass.tsx` 按它取折射参数） */
export function useMaterial() {
    return {material: levelMaterial(useGlassLevel().level)}
}

/** 内容面板要不要玻璃面（`components/Panel.tsx`） */
export function useGlobalGlass() {
    return {globalGlass: levelGlobalGlass(useGlassLevel().level)}
}
