import type {ReactNode} from 'react'
import {useCallback, useEffect, useRef, useState} from 'react'
import {api} from '@/lib/api'
import {baseName} from '@/lib/format'
import {subscribeDrop} from '@/lib/ipc'

/**
 * 选文件的**两条入口**，都回**本机路径**。
 *
 * 为什么非要回路径：后端（ffmpeg / LibreSVIP / Python 分离引擎）只认本机路径，
 * 浏览器里的 `File` 对象它看不见。所以两条路都要落到「一个路径字符串」上：
 *
 *   1. **系统对话框** —— `pick_paths`（官方 `tauri-plugin-dialog`，rfd 打底）。
 *      取消回的是空数组而**不是错误**。
 *   2. **把文件直接拖进窗口** —— `getCurrentWebview().onDragDropEvent()` 的载荷里
 *      **直接就有磁盘上的真路径**（`event.payload.paths`）。
 *
 * ⚠️ **页面里不要再写 HTML5 的 `onDrop` / `DataTransfer`。** Tauri 默认把拖放截走
 * 改发成 Window 事件（`lib.rs` 里那条「不要关掉拖放拦截」的注释说的就是这件事），
 * 所以页面上的 HTML5 拖放事件**根本不会触发**。
 *
 * ⚠️ **别再画自制的目录树 / 文件树**：系统对话框提供「此电脑」「网络位置」与用户
 * 自己的快捷方式，自制的那套覆盖不到。
 */

export type ToastTone = 'ok' | 'err' | 'warn' | 'info'

/**
 * 拖放那套东西，展开到页面根元素上：`<div {...dropProps}>`。
 *
 * ⚠️ 现在**是空对象**（拖放由 Tauri 的窗口事件接管，DOM 上没有任何监听器要挂）。
 * 保留这个形状只是为了让六个页面的 `<div {...dropProps}>` 一个字都不用改 ——
 * 展开一个空对象是无害的。`dragging` 才是要用的那个值。
 */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type
export interface DropProps {
}

/**
 * 监听「文件被拖进窗口」。`onPaths` 拿到的是**真路径**。
 *
 * ⚠️ 订阅是**窗口级**的，所以每个挂着的钩子都会收到同一次拖放的路径 ——
 * 一页只挂一个（`useFilePick` 内部挂的也是它）。
 */
export function useNativeDrop(onPaths: (paths: string[]) => void): {
    dropProps: DropProps
    dragging: boolean
} {
    const [dragging, setDragging] = useState(false)
    const cb = useRef(onPaths)
    cb.current = onPaths

    useEffect(() => {
        let alive = true
        let un: (() => void) | null = null
        void subscribeDrop({
            onActive: (a) => {
                if (alive) setDragging(a)
            },
            onDrop: (paths) => cb.current(paths),
        })
            .then((fn) => {
                if (alive) un = fn
                else fn()
            })
            .catch((e: unknown) => {
                /* 拿不到 webview（理论上不会）时不该把整页拖垮：拖放这条路没了，对话框还在 */
                console.warn('[FilePick] 拖放订阅失败：', e)
            })
        return () => {
            alive = false
            setDragging(false)
            un?.()
        }
    }, [])

    return {dropProps: {}, dragging}
}

export interface FilePickOptions {
    /** 允许的扩展名，**不带点**：`['mp3', 'wav']`。空数组 = 不限制 */
    exts?: string[]
    /** 对话框里的文件类型名，给用户看的：`'音频 / 视频文件'` */
    label?: string
    /** 系统对话框的标题 */
    title?: string
    /** 对话框的起始目录 */
    dir?: string
    /** 允许多选（工程转换一次挑几个是常事） */
    multi?: boolean
    /** 选中的代价：拿到本机路径后干什么 */
    onPaths: (paths: string[]) => void
    onToast?: (msg: string, tone?: ToastTone) => void
}

/** `name` 不在允许表里就丢（没给 `exts` 时全收）。大小写不敏感、不要点。 */
function allowed(name: string, exts: string[]): boolean {
    if (!exts.length) return true
    const dot = name.lastIndexOf('.')
    if (dot < 0) return false
    return exts.some((x) => x.replace(/^\./, '').toLowerCase() === name.slice(dot + 1).toLowerCase())
}

export function useFilePick(opts: FilePickOptions): {
    /** 弹系统对话框。**不抛异常**：失败已经 toast 出去了，调用方 `void pick()` 即可 */
    pick: () => Promise<void>
    dropProps: DropProps
    dragging: boolean
    /** 正在忙（对话框挂着）。给按钮上转圈用 */
    busy: boolean
} {
    const [busy, setBusy] = useState(false)
    /* 这些是「每次调用都可能变」的值（起始目录跟着上次选的文件走），用 ref 拿最新的，
       免得 hook 的返回函数每次渲染都换身份、把页面里的 `useEffect` 依赖搅乱。 */
    const o = useRef(opts)
    o.current = opts

    /* 卸载后别再 setState（用户可能等不及对话框就切页了） */
    const alive = useRef(true)
    useEffect(() => {
        alive.current = true
        return () => {
            alive.current = false
        }
    }, [])

    const pick = useCallback(async () => {
        const {exts, label, title, dir, multi, onPaths, onToast} = o.current
        setBusy(true)
        try {
            const r = await api.fsPick({
                exts: exts?.length ? exts : undefined,
                label,
                title,
                dir: dir || undefined,
                multi: !!multi,
            })
            /* 取消不是错误：回的是空数组。这里**不 toast** —— 用户自己按的取消，
               弹一句「你取消了」只会让人以为出了事。 */
            if (r.files.length) onPaths(r.files)
        } catch (e) {
            onToast?.(e instanceof Error ? e.message : String(e), 'err')
        } finally {
            if (alive.current) setBusy(false)
        }
    }, [])

    /** 拖进来的文件：**已经是真路径**，过滤一下格式就能直接交给 `onPaths` */
    const handlePaths = useCallback((paths: string[]) => {
        const {exts, onPaths, onToast} = o.current
        const ok = paths.filter((p) => allowed(baseName(p), exts ?? []))
        const skipped = paths.length - ok.length
        if (!ok.length) {
            onToast?.(`只认这些格式：${(exts ?? []).map((x) => '.' + x).join(' / ')}`, 'warn')
            return
        }
        onPaths(ok)
        if (skipped) onToast?.(`有 ${skipped} 个文件格式不对，已经跳过`, 'warn')
    }, [])

    const {dropProps, dragging} = useNativeDrop(handlePaths)

    return {pick, dropProps, dragging, busy}
}

/**
 * 「也可以拖进来」那一句话。**不是落点** —— 落点是整个窗口（Tauri 的拖放事件），
 * 这条只是告诉用户「可以拖」，拖到窗口上时亮成强调色。
 */
export function DropHint({
                             dragging,
                             busy,
                             text = '文件也可以直接拖进这个窗口',
                             icon,
                             className = '',
                         }: {
    dragging: boolean
    busy?: boolean
    text?: ReactNode
    icon?: ReactNode
    /** 页面给它加尺寸 / 定位用的类（`pv-drop` 那种在工具条里的一行） */
    className?: string
}) {
    return (
        <div className={`drop-hint ${className}`.trim()} data-over={dragging ? 'true' : 'false'}>
            {icon}
            <span>{busy ? '正在读取拖进来的文件…' : text}</span>
        </div>
    )
}
