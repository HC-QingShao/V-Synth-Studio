import {GlassSurface, type GlassSurfaceOptions} from '@ttqtt/liquid-glass-react'
import * as React from 'react'
import {createContext, type ReactNode, useContext} from 'react'

/**
 * 玻璃材质 —— **两种，都来自库，没有一行是手写的**。
 *
 * 材质由 `@ttqtt/liquid-glass-react` 的 `GlassSurface` 提供：**折射**、**一条发丝边**、
 * **干净的内部**三样。手写的 blur + saturate + inset 高光只能做出模糊的半透明方块。
 *
 * | 材质 | 库的取值 | 观感 |
 * |---|---|---|
 * | 毛玻璃 `frosted` | `material="regular"`、`refraction={0}` | 模糊 + 提色 + 发丝边，不折射 |
 * | 液态玻璃 `liquid` | `material="clear"`、`refraction={32}` | 边缘把背后的内容折弯 |
 *
 * ⚠️ **只有 Chromium 认那条 SVG 位移滤镜**（WebView2 就是 Chromium，没问题；
 * macOS 的 WKWebView / Firefox 拿不到）。库对此有内置降级：它把**真实的 `blur()` 放在
 * CSS 链的最前面**，滤镜只负责位移 —— 所以哪家引擎丢掉了 `url()`，剩下的仍然是磨砂，
 * 而不是一块透明的洞。不需要自己写 `@supports` 兜底。
 *
 * ⚠️ 折射**开销约三倍**，库默认是关的，所以它只在选了「液态玻璃」时才开。
 */

export type GlassMaterial = 'frosted' | 'liquid'

/**
 * 材质 → 库的参数的映射。这是「两种材质」的唯一定义处。
 *
 * ⚠️ **默认是毛玻璃，不是液态玻璃。** 设计系统第 3 节把两者的用处分得很清：
 * `regular`（毛玻璃）是默认，「栏、侧边栏、菜单、文字较多的表面」都用它；
 * `clear`（液态玻璃）**只用于媒体内容之上、且上层内容本身明亮醒目**的场合。
 *
 * **不要默认给 `clear`**（明亮模式，`tone=light`）：库会叠一层
 * `.lg-tint = rgba(0,0,0,.35)` 的 35% 黑压，而 `clear/small` 的模糊只有 1.5px
 * （开折射后再减半，0.75px）—— 顶栏和侧栏会变成两块**纯灰板**。
 *
 * ⚠️ 玻璃等级 1~4（`lib/useGlass.ts`）是界面上的选择，材质由那一档派生 ——
 * 这里不自己读存储（那个键已经并进 `config.json` 的 `glassLevel`）。
 */
export function materialOptions(m: GlassMaterial): GlassSurfaceOptions {
    return m === 'frosted'
        ? {material: 'regular', refraction: 0}
        : {material: 'clear', refraction: 32}
}

/* ══════════════════════════════════════════════════════════════════════════ */

interface GlassLayerProps extends Omit<GlassSurfaceOptions, 'material'> {
    children: React.ReactNode
    /** 挂到**玻璃面本身**上的类。尺寸、外边距、定位放这里。 */
    className?: string
    /** 挂到**内容层**（`.lg-content`）上的类。要撑满高度时用 `h-full`。 */
    contentClassName?: string
    /** 圆角。库的刻度：14 / 20 / 26 / 34，或 `'pill'`。 */
    radius?: number | 'pill'
    /** 内容内边距。默认 18px（库的 `.lg-surface` 值）。 */
    padding?: number
    style?: React.CSSProperties
    material: GlassMaterial
}

/**
 * 一块玻璃面。**浮起来的那一层**才用它 —— 栏、工具栏、侧栏、浮层、提示。
 *
 * ⚠️ 库自己的注释写得很直接：
 *
 * > This belongs to the navigation / control layer — bars, groups, overlays.
 * > **It is not a card**: content-layer containers use `Card`, `List` or `MaterialView`,
 * > which do not sample the backdrop at all.
 *
 * 也就是**别再往卡片上套玻璃**：满屏半透明 = 没有东西真的浮起来。
 */
export function GlassLayer({
                               children,
                               className,
                               contentClassName,
                               radius,
                               padding,
                               style,
                               material,
                               ...rest
                           }: GlassLayerProps) {
    /**
     * 内容层的类名靠 **context 传下去**，由每个 `GlassPanel` 用它包一层真盒子。
     *
     * ⚠️ **不要改回「用 DOM 找子元素」那套**，两条路都不成立：
     *
     *   1. `firstElementChild` —— 只给**第一个**子元素加类，后面的拿不到
     *      `display:flex`，会换行成两行、玻璃跟着撑高。
     *   2. `display: contents` 的包装当锚点 —— **它不生成盒子**（`offsetWidth === 0`），
     *      写在子元素上的 `display:flex` 全废，`innerW=0`、内容塌掉。
     *
     * 库生成的 `.lg-content` 我们拿不到 ref，React 的子元素也没法批量加类 ——
     * 用一个 context 让它自己包，是最不容易出错的做法（一个真 div，看得见摸得着）。
     */
    return (
        <GlassSurface
            {...materialOptions(material)}
            {...rest}
            radius={radius}
            className={className}
            style={{...style, ...(padding !== undefined ? {padding} : null)}}
        >
            <ContentClass.Provider value={contentClassName ?? ''}>{children}</ContentClass.Provider>
        </GlassSurface>
    )
}

const ContentClass = createContext('')

/**
 * 玻璃面里的内容层。**自己**包一层 div，类名从 `GlassLayer` 传下来。
 *
 * 直接放内容进 `GlassLayer` 也行（会落到库的 `.lg-content` 里），但只要需要
 * flex / 滚动 / 撑满，就用这个 —— 它会拿到 `contentClassName`。
 */
export function GlassContent({
                                 children,
                                 className,
                                 fill = false,
                             }: {
    children: ReactNode
    className?: string
    /** 撑满玻璃面高度（侧栏那种要自己滚的用得上） */
    fill?: boolean
}) {
    const cls = useContext(ContentClass)
    return (
        <div
            className={`${cls} ${fill ? 'glass-fill' : ''} ${className ?? ''}`.trim()}
        >
            {children}
        </div>
    )
}

