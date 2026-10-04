import type { ReactNode } from 'react'
import { Panel, PanelHead, Stat } from '@/components/Panel'
import { api } from '@/lib/api'

/**
 * 许可与出处 —— 每个功能页脚上一块，说明「这页的活是谁干的、按什么许可」。
 *
 * 为什么要有它：这个程序自己写的代码不多，真正的活全交给了第三方
 * （LibreSVIP 转格式、ffmpeg 转音频、yt-dlp 下载、audio-separator 分离、yt-dlp/GAME 扒谱…），
 * 而那些东西的许可（Apache-2.0 / GPL v3 / Unlicense / MIT）**要求随分发附上说明**。
 * 所以这里不是装饰：左边几格是事实（版本 + 许可 + 用法），底下一段是给用户看的解释。
 *
 * ⚠️ **同一批事实在设置页还有一份总表**（`pages/Settings.tsx` 的「源码许可」小节）。
 * 用户明确要求两处都留，所以改一处要**同步另一处**，别只改一边。
 *
 * 排版：上面一排 `.credit-stats`（和 `.stats-row` 同形但不需要「右边挂动作」那种对齐），
 * 下面一段正文。**别用 `<a>` 写在正文里** —— 在这个壳里点链接会把整个 WebView 带走，
 * 上游链接一律用 [`Upstream`](#)（它转手交给系统默认浏览器）。
 */
export function Credit({
  desc,
  tags,
  items,
  children,
}: {
  /** 面板说明，一句话讲清这一页把活交给了谁 */
  desc?: ReactNode
  /** 标题右边的小标签（`<Chip>`），用来标注模型作者、引擎名之类 */
  tags?: ReactNode
  /** 左排的事实格：`{ label: '代码', value: 'GPL v3', sub: 'FFmpeg 9.0.2' }` */
  items: { label: ReactNode; value: ReactNode; sub?: ReactNode }[]
  children: ReactNode
}) {
  return (
    <Panel>
      <PanelHead title="许可与出处" desc={desc} extra={tags} />
      {items.length > 0 && (
        <div className="credit-stats">
          {items.map((it, i) => (
            <Stat key={i} label={it.label} value={it.value} sub={it.sub} />
          ))}
        </div>
      )}
      <p className="hint">{children}</p>
    </Panel>
  )
}

/**
 * 正文里的上游链接。
 *
 * **不是 `<a href>`** —— 窗口加载的是 `http://127.0.0.1:17878`，点一个真链接会把这个
 * 本地页面整个换成上游网站，用户就回不来了（后退键在 WebView 里没有）。所以走
 * `/api/fs/open` 的 `{ url }` 分支：后端交给自己平台的默认处理方式
 * （Windows 上 `ShellExecute`、macOS 上 `open`），在**系统浏览器**里打开。
 *
 * ⚠️ 这条契约以前是坏的：`/api/fs/open` 有很长一段时间只读 `path`，所以所有传
 * `{url}` 的调用必然 400 ——「在浏览器打开」处处都点不动。现在两种参数都收
 * （`platform::looks_like_url` 判），别再改回去只认 `path`。
 *
 * 失败**必须让用户看见**：非 Windows 平台这个动作还没实现，后端会回一句人话，
 * 静默吞掉的话用户只会觉得「点了没反应」。
 */
export function Upstream({
  href,
  children,
  onToast,
}: {
  href: string
  children: ReactNode
  onToast?: (msg: string, tone?: 'ok' | 'err' | 'warn' | 'info') => void
}) {
  return (
    <button
      type="button"
      className="upstream"
      title={`在系统浏览器里打开：${href}`}
      onClick={() => {
        api.fsOpen({ url: href }).catch((e: unknown) => {
          const msg = e instanceof Error ? e.message : String(e)
          onToast?.(`打不开浏览器：${msg}`, 'err')
        })
      }}
    >
      {children}
    </button>
  )
}
