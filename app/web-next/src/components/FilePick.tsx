import { useCallback, useEffect, useRef, useState } from 'react'
import type { DragEvent, ReactNode } from 'react'
import { api } from '@/lib/api'

/**
 * 选文件的**两条入口**，都回**本机路径**。
 *
 * 为什么非要回路径：后端（ffmpeg / LibreSVIP / Python 分离引擎）只认本机路径，
 * 浏览器里的 `File` 对象它看不见。所以两条路都要落到「一个路径字符串」上：
 *
 *   1. **系统「打开」对话框** —— `POST /api/fs/pick`（`platform::pick_files`）。
 *      这条**会一直挂着**直到用户点确定或取消，所以超时给到 10 分钟，取消回的是
 *      `files: []` 而**不是错误**。
 *   2. **把文件直接拖进窗口** —— 浏览器只给 `File`，所以先 `POST /api/fs/upload`
 *      把字节交上去，后端落进临时目录换回一个路径。
 *
 * ⚠️ **别再画自制的目录树 / 文件树**。以前这里挂着 `DirPicker`（自己用
 * `GlassDialog` + `PathBar` + `List` 画），六个页面各挂一份、长得还不一样，
 * 而系统对话框全都有 —— 还多出「此电脑」「网络位置」和用户自己的快捷方式。
 *
 * ⚠️ 落点（`dropProps`）挂在**页面根元素**上，不挂在提示条上：用户拖文件进来时
 * 眼睛看的是整页，落在任意位置都该算数。
 */

export type ToastTone = 'ok' | 'err' | 'warn' | 'info'

/** 拖放那套事件处理，直接展开到页面根元素上：`<div {...dropProps}>` */
export interface DropProps {
  onDragEnter?: (e: DragEvent<HTMLElement>) => void
  onDragOver?: (e: DragEvent<HTMLElement>) => void
  onDragLeave?: (e: DragEvent<HTMLElement>) => void
  onDrop?: (e: DragEvent<HTMLElement>) => void
}

/**
 * 拖放手势。**只管把 `File` 交出去**，不碰 `onPaths` —— 需要本机路径的用
 * `useFilePick`，只需要 `File` 本身的（音轨分离要直接 multipart 上传）用这个。
 *
 * ⚠️ `dragenter` / `dragleave` 是**冒泡且成对**的：鼠标在页面上从子元素滑到子元素
 * 会先 `leave` 再 `enter`。用布尔量记状态的话，滑过任意一层就会闪一下
 * （提示条一明一暗）。所以用一个**计数器**：进出配平，归零才算真的离开。
 */
export function useFileDrop(onFiles: (files: File[]) => void): {
  dropProps: DropProps
  dragging: boolean
} {
  const [dragging, setDragging] = useState(false)
  const depth = useRef(0)
  const cb = useRef(onFiles)
  cb.current = onFiles

  /** 拖到窗口外面松手时 `dragleave` 可能收不到 —— 补一个全局收尾，免得提示条一直亮着 */
  useEffect(() => {
    const end = () => {
      depth.current = 0
      setDragging(false)
    }
    window.addEventListener('dragend', end)
    window.addEventListener('drop', end)
    return () => {
      window.removeEventListener('dragend', end)
      window.removeEventListener('drop', end)
    }
  }, [])

  const onDragEnter = useCallback((e: DragEvent<HTMLElement>) => {
    if (!e.dataTransfer?.types.includes('Files')) return
    depth.current += 1
    setDragging(true)
  }, [])

  const onDragOver = useCallback((e: DragEvent<HTMLElement>) => {
    /* ⚠️ **必须 preventDefault**：不阻止默认行为的话浏览器会「打开这个文件」，
       WebView 会直接跳走，而且 `drop` 事件根本不会派发到我们头上。 */
    if (!e.dataTransfer?.types.includes('Files')) return
    e.preventDefault()
    e.dataTransfer.dropEffect = 'copy'
  }, [])

  const onDragLeave = useCallback((e: DragEvent<HTMLElement>) => {
    if (!e.dataTransfer?.types.includes('Files')) return
    depth.current = Math.max(0, depth.current - 1)
    if (depth.current === 0) setDragging(false)
  }, [])

  const onDrop = useCallback(
    (e: DragEvent<HTMLElement>) => {
      const files = Array.from(e.dataTransfer?.files ?? [])
      if (!files.length) return
      e.preventDefault()
      depth.current = 0
      setDragging(false)
      cb.current(files)
    },
    [],
  )

  return { dropProps: { onDragEnter, onDragOver, onDragLeave, onDrop }, dragging }
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
  /** 弹系统「打开」对话框。**不抛异常**：失败已经 toast 出去了，调用方 `void pick()` 即可 */
  pick: () => Promise<void>
  dropProps: DropProps
  dragging: boolean
  /** 正在忙（对话框挂着，或拖进来的文件正在上传）。用来给按钮上转圈 */
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
    const { exts, label, title, dir, multi, onPaths, onToast } = o.current
    setBusy(true)
    try {
      /* ⚠️ `exts` 一律传数组（可以是空的）。后端只在参数**缺失**时才不筛，
         传空数组和不传是两回事，别为了「好看」把它省掉。 */
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

  /** 拖进来的文件：过滤 → 逐个上传换路径 → 一次性交给 `onPaths` */
  const handleFiles = useCallback(async (files: File[]) => {
    const { exts, onPaths, onToast } = o.current
    const ok = files.filter((f) => allowed(f.name, exts ?? []))
    const skipped = files.length - ok.length
    if (!ok.length) {
      onToast?.(`只认这些格式：${(exts ?? []).map((x) => '.' + x).join(' / ')}`, 'warn')
      return
    }
    setBusy(true)
    try {
      const paths: string[] = []
      for (const f of ok) {
        const up = await api.fsUpload(f)
        paths.push(up.path)
      }
      onPaths(paths)
      if (skipped) {
        onToast?.(`有 ${skipped} 个文件格式不对，已经跳过`, 'warn')
      }
    } catch (e) {
      onToast?.(e instanceof Error ? e.message : String(e), 'err')
    } finally {
      if (alive.current) setBusy(false)
    }
  }, [])

  const { dropProps, dragging } = useFileDrop((files) => void handleFiles(files))

  return { pick, dropProps, dragging, busy }
}

/**
 * 「也可以拖进来」那一句话。**不是落点** —— 落点是页面根元素（`dropProps`），
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
