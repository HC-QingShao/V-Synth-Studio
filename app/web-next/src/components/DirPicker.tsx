import { useCallback, useState } from 'react'
import { Button } from '@/components/Button'
import { api } from '@/lib/api'

/**
 * 目录输入框 + 「选择」按钮 —— 表单里最常见的一格。
 *
 * （文件名 `DirPicker.tsx` 是历史遗留：它以前还导出一个自己画的目录树组件，
 * 2026-10 那次「文件选择器一律走系统」把它删了，只留这一格。名字没改是为了
 * 不惊动六个页面的 import。）
 *
 * 「选择」拉的是**系统「选择文件夹」对话框**（`pick_paths` 带 `mode: "folder"`）。
 *
 * 以前这里挂着的是 `DirPicker`：自己用 `GlassDialog` + `PathBar` + `List` 画了一个
 * 目录树（靠已退役的列目录路由、自己实现「新建文件夹」）。六个页面各挂一份，
 * 长得还都不一样，而它做的事系统对话框全都有，还多出「此电脑」「网络位置」和
 * 用户自己收藏的快捷方式。**别再把它画回来。**
 *
 * 输入框本身留着：路径是可以手打、粘贴的，选只是其中一条路；「清空」也在。
 * 非 Windows 上系统对话框还没实现，后端会回一句人话（`Err("…请直接把路径填进输入框")`），
 * 我们照原样 toast 出来 —— 输入框还在，用户不至于卡死。
 */
export function DirectoryInput({
  value,
  onChange,
  placeholder = '留空则用设置里的默认目录',
  title = '选择目录',
  onToast,
}: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  /** 系统对话框的标题。给「保存到哪里」这类场景用得上 */
  title?: string
  onToast?: (msg: string, tone?: 'ok' | 'err' | 'warn' | 'info') => void
}) {
  /** 对话框开着的时候按钮变「等待选择…」—— 那是原生模态框，页面这会儿点不动 */
  const [picking, setPicking] = useState(false)

  const pickFolder = useCallback(async () => {
    setPicking(true)
    try {
      const r = await api.fsPick({ folder: true, title, dir: value || undefined })
      if (r.files.length) onChange(r.files[0])
    } catch (e) {
      onToast?.(e instanceof Error ? e.message : String(e), 'err')
    } finally {
      setPicking(false)
    }
  }, [onChange, onToast, title, value])

  return (
    <div className="input-group">
      <input
        className="input"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
      <Button size="sm" disabled={picking} onClick={() => void pickFolder()}>
        {picking ? '等待选择…' : '选择'}
      </Button>
      {value && (
        <Button size="sm" variant="ghost" onClick={() => onChange('')}>
          清空
        </Button>
      )}
    </div>
  )
}
