import { useCallback, useEffect, useRef, useState } from 'react'
import { api } from '@/lib/api'
import type { AppState, HealthInfo } from '@/lib/types'
import { Button } from '@/components/Button'
import { Field, TextInput } from '@/components/Field'
import { Chip, GlassPanel, Panel, PanelHead, Stat } from '@/components/Panel'
import { Upstream } from '@/components/Credit'
import { GlassSlider } from '@ttqtt/liquid-glass-react'
import { GLASS_LEVELS, useGlassLevel, type GlassLevel } from '@/lib/useGlass'
import { useNavLens } from '@/lib/useNavLens'
import type { ThemeMode } from '@/App'

/**
 * 设置。
 *
 * ## 「性能模式」这一项换掉了
 *
 * 旧版这里有个手写的「性能模式」开关，切 `<html data-perf>` 来全局关模糊。
 * 现在用库的 `transparency`（由 `App.tsx` 喂给 `GlassProvider`）——
 * 它同时做三件旧版没做的事：
 *
 *   1. 系统里开了「减少透明度」时**自动**生效，不用用户再点一次
 *   2. 关掉的是整个材质（模糊 + 半透明 + 折射），不只是 `backdrop-filter`
 *   3. 开关一开，所有玻璃面一起变，不会漏掉某个角落
 *
 * 语义也改了：不是「性能模式」（听起来像降级），而是**降低透明度**（这是无障碍需求）。
 */

const SECTIONS = [
  { id: 'appearance', label: '外观' },
  { id: 'paths', label: '路径' },
  { id: 'tools', label: '外部工具' },
  { id: 'about', label: '关于' },
] as const

type SectionId = (typeof SECTIONS)[number]['id']

export function Settings({
  state,
  theme,
  onThemeChange,
  onRefreshState,
  onNavigate,
  onToast,
}: {
  state: AppState | null
  theme: ThemeMode
  onThemeChange: (m: ThemeMode) => void
  onRefreshState: () => Promise<void>
  onNavigate: (id: string) => void
  onToast: (msg: string, tone?: string) => void
}) {
  const [section, setSection] = useState<SectionId>('appearance')
  /* 高亮块要量位置：和主侧栏同一套（见 lib/useNavLens.ts） */
  const navRef = useRef<HTMLElement>(null)
  const lensRef = useRef<HTMLSpanElement>(null)
  useNavLens(navRef, lensRef, section)
  const [cfg, setCfg] = useState<Record<string, unknown>>({})
  const [health, setHealth] = useState<HealthInfo | null>(null)

  const reload = useCallback(async () => {
    try {
      const s = await api.state()
      setCfg((s.config ?? {}) as Record<string, unknown>)
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e), 'err')
    }
  }, [onToast])

  useEffect(() => {
    void reload()
    api.health().then(setHealth).catch(() => {})
  }, [reload])

  const save = useCallback(
    async (patch: Record<string, unknown>, okMsg = '设置已保存') => {
      try {
        await api.saveConfig(patch)
        setCfg((c) => ({ ...c, ...patch }))
        onToast(okMsg, 'ok')
      } catch (e) {
        onToast(`保存失败：${e instanceof Error ? e.message : String(e)}`, 'err')
        throw e
      }
    },
    [onToast],
  )

  return (
    <div className="settings">
      {/* 这一条小节导航和**主侧栏是同一种东西**：一块玻璃 + 一个滑过去的高亮块。
          结构必须和 `App.tsx` 的主导航一致（`useNavLens` 靠这三个类名找目标）。 */}
      <GlassPanel
        className="settings-nav"
        /* 参数**和主侧栏逐项对齐**（large 玻璃 / 26 圆角 / 12 内边距）——
           `.nav-lens` 那个 14px 圆角就是按「26 − 12」算的同心情形，换数字就对不上了 */
        size="large"
        radius={26}
        padding={12}
      >
        <nav className="app-nav" aria-label="设置分节" ref={navRef}>
          <span className="lg-selection-lens nav-lens" ref={lensRef} aria-hidden="true" />
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              type="button"
              className="nav-row"
              aria-current={section === s.id ? 'page' : undefined}
              onClick={() => setSection(s.id)}
            >
              <span className="nav-row-label">{s.label}</span>
            </button>
          ))}
        </nav>
      </GlassPanel>

      <div className="stack-lg settings-body">
        {section === 'appearance' && (
          <Appearance theme={theme} onThemeChange={onThemeChange} />
        )}
        {section === 'paths' && (
          <Paths cfg={cfg} state={state} onSave={save} onToast={onToast} />
        )}
        {section === 'tools' && (
          <Tools state={state} onRefreshState={onRefreshState} onToast={onToast} />
        )}
        {section === 'about' && (
          <About
            state={state}
            health={health}
            onRefresh={reload}
            onNavigate={onNavigate}
            onToast={onToast}
          />
        )}
      </div>
    </div>
  )
}

/* ══════════════════════════════════════════════════════════════ 外观 ══ */

function Appearance({
  theme,
  onThemeChange,
}: {
  theme: ThemeMode
  onThemeChange: (m: ThemeMode) => void
}) {
  const { level, setLevel } = useGlassLevel()

  /** 滑块给的是 number，收进 1~3（拖动/键盘理论上都给不出界外值，防御一下） */
  const clampLevel = (v: number): GlassLevel =>
    Math.min(GLASS_LEVELS.length, Math.max(1, Math.round(v))) as GlassLevel
  const THEMES: { id: ThemeMode; label: string; desc: string }[] = [
    { id: 'system', label: '跟随系统', desc: '系统切换配色时自动跟着换' },
    { id: 'light', label: '明亮', desc: '浅色底、细描边' },
    { id: 'dark', label: '黑暗', desc: '深色底，长时间看不刺眼' },
  ]

  return (
    <>
      {/*
        玻璃等级：一个滑块管住原来三件事 —— 材质（毛玻璃/液态）、
        「全局玻璃」（内容面板要不要玻璃面）、「降低透明度」（库的 opaque 策略）。
        用户看到的是三个互相影响的开关，合成分级之后语义才清楚：级别越高越「玻璃」，代价越大。
        滑块本身是**库的 `GlassSlider`**（真 `<input type=range>` 打底，键盘/读屏都能用）。
      */}
      <Panel>
        <PanelHead
          title="玻璃等级"
          desc="拖动滑块调整。级别越高越「玻璃」，开销也越大 —— 1 级最省、对比最高，4 级折射最全"
        />
        <div className="slider-row">
          <GlassSlider
            aria-label="玻璃等级"
            min={1}
            max={GLASS_LEVELS.length}
            step={1}
            marks
            value={level}
            onValueChange={(v) => setLevel(clampLevel(v))}
            formatValue={(v) => `${v} 级：${GLASS_LEVELS[v - 1]?.label ?? ''}`}
            minLabel="1"
            maxLabel={String(GLASS_LEVELS.length)}
          />
          <div className="slider-legend">
            {GLASS_LEVELS.map((l) => (
              <button
                key={l.level}
                type="button"
                className="slider-legend-item"
                aria-pressed={level === l.level}
                onClick={() => setLevel(l.level)}
              >
                <span className="slider-legend-label">
                  {l.level} 级 · {l.label}
                </span>
                <span className="slider-legend-desc">{l.desc}</span>
              </button>
            ))}
          </div>
        </div>
        <p className="hint">
          1 级把玻璃换成不透明底色；2 级只模糊提色、3 级开折射 —— 这两级**内容面板都是轻量材质**；
          4 级连内容面板也变成玻璃，画面里每个玻璃面都会多一层 SVG 位移贴图（开销最大）。
          等级只影响材质，背景图参数不变。
          系统里开了「减少透明度」时，模糊会自动失效 —— 那是库的无障碍策略，不受这里影响。
        </p>
      </Panel>

      <Panel>
        <PanelHead title="主题" desc="整套界面的配色，选完立刻生效" />
        <div className="choice-grid">
          {THEMES.map((t) => (
            <button
              key={t.id}
              type="button"
              className="choice"
              aria-pressed={theme === t.id}
              onClick={() => onThemeChange(t.id)}
            >
              <span className="choice-head">
                <span className="choice-label">{t.label}</span>
                {theme === t.id && <Chip tone="accent">已选</Chip>}
              </span>
              <span className="choice-desc">{t.desc}</span>
            </button>
          ))}
        </div>
      </Panel>
    </>
  )
}

/* ══════════════════════════════════════════════════════════════ 路径 ══ */

function Paths({
  cfg,
  state,
  onSave,
  onToast,
}: {
  cfg: Record<string, unknown>
  state: AppState | null
  onSave: (patch: Record<string, unknown>, ok?: string) => Promise<void>
  onToast: (m: string, t?: string) => void
}) {
  const [out, setOut] = useState('')
  const [dl, setDl] = useState('')
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    setOut(String(cfg.outputDir ?? state?.paths?.outputDir ?? ''))
    setDl(String(cfg.downloadDir ?? state?.paths?.downloadDir ?? ''))
  }, [cfg.outputDir, cfg.downloadDir, state?.paths?.outputDir, state?.paths?.downloadDir])

  return (
    <Panel>
      <PanelHead title="默认目录" desc="只影响默认值，每次操作时还能单独改" />
      <div className="stack">
        <Field label="默认输出目录（转换结果）">
          <TextInput value={out} onChange={(e) => setOut(e.target.value)} />
        </Field>
        <Field label="默认下载目录（视频 / 音频）">
          <TextInput value={dl} onChange={(e) => setDl(e.target.value)} />
        </Field>
        <div className="btn-row">
          <Button
            variant="primary"
            loading={busy}
            onClick={async () => {
              setBusy(true)
              try {
                await onSave({ outputDir: out, downloadDir: dl }, '路径已保存')
              } catch {
                /* save 已经报过 toast */
              } finally {
                setBusy(false)
              }
            }}
          >
            保存路径设置
          </Button>
          <Button
            onClick={() =>
              api.fsReveal(out, false).catch((e: unknown) =>
                onToast(e instanceof Error ? e.message : String(e), 'err'),
              )
            }
          >
            打开输出目录
          </Button>
        </div>
        <p className="hint">程序根目录：{state?.paths?.root ?? '未读取到'}</p>
      </div>
    </Panel>
  )
}

/* ══════════════════════════════════════════════════════════ 外部工具 ══ */

function Tools({
  state,
  onRefreshState,
  onToast,
}: {
  state: AppState | null
  onRefreshState: () => Promise<void>
  onToast: (m: string, t?: string) => void
}) {
  const tools = state?.tools ?? {}
  const [busy, setBusy] = useState(false)
  const missing = ['ffmpeg', 'ytdlp'].filter((k) => !tools[k]?.available)
  const rows = [
    { key: 'ffmpeg', name: 'ffmpeg', desc: '音视频合并、导出 WAV/MP3、变调变速、响度标准化' },
    { key: 'ytdlp', name: 'yt-dlp', desc: 'YouTube 等上千站点的解析与下载（B 站走内置解析）' },
    { key: 'python', name: 'Python', desc: '可选：部分脚本与 yt-dlp 的模块模式会用到' },
  ]

  return (
    <Panel>
      <PanelHead
        title="外部工具"
        desc={`工具目录：${state?.paths?.toolsDir ?? '未读取到'}`}
        extra={
          <Button
            size="sm"
            loading={busy}
            onClick={async () => {
              setBusy(true)
              try {
                await api.detect(true)
                await onRefreshState()
                onToast('检测完成', 'ok')
              } catch (e) {
                onToast(e instanceof Error ? e.message : String(e), 'err')
              } finally {
                setBusy(false)
              }
            }}
          >
            重新检测
          </Button>
        }
      />
      <div className="tool-list">
        {rows.map((r) => {
          const info = tools[r.key]
          return (
            <div key={r.key} className="tool-row">
              <div className="tool-row-text">
                <span className="tool-name">{r.name}</span>
                <span className="tool-desc">{r.desc}</span>
              </div>
              <span className="tool-version" data-ok={info?.available ? 'true' : undefined}>
                {info?.available ? String(info.version ?? '已就绪').slice(0, 28) : '未找到'}
              </span>
              {info?.available && info.path && (
                <Button
                  size="sm"
                  onClick={() =>
                    api.fsReveal(info.path!, true).catch((e: unknown) =>
                      onToast(e instanceof Error ? e.message : String(e), 'err'),
                    )
                  }
                >
                  定位
                </Button>
              )}
            </div>
          )
        })}
      </div>
      <p className="hint">
        {missing.length
          ? `未检测到 ${missing.map((m) => (m === 'ytdlp' ? 'yt-dlp' : m)).join(' / ')}。它们随程序分发、不需要联网下载；若显示未找到，把 tools 目录重新解压到程序根目录即可。`
          : '三个外部工具都齐了，音频与下载功能完整可用。'}
      </p>
    </Panel>
  )
}

/* ══════════════════════════════════════════════════════════════ 关于 ══ */

/** 「关于作者」那一栏的邮箱。点按钮就复制这一串。 */
const AUTHOR_MAIL = '1813616607@qq.com'

/**
 * 感谢名单。
 *
 * ⚠️ 顺序**不代表排名** —— 面板上那行「排名无先后顺序」就是为这条写的，改名单时别按
 * 「贡献大小」重排。ID 一律**照抄对方自己写的样子**（大小写、空格、假名、emoji 都别动）。
 */
const SPONSORS = [
  '坎伊bbb',
  'Desire Control',
  '浅唱教主',
  '老钱',
  '我是len的帽子',
  '入さん',
  'ゆりかごから墓場まで',
  '小偷人机在线逃跑',
  'WuJinGY',
  'venom',
  'LYT',
  '唠蟹公主',
  '火橘子',
  '筱箖',
  'nxy',
  '让我们一起陷入狂赌之渊吧',
  '星街彗星',
  '神野冬花',
]

/**
 * 源码许可总表（关于页）。
 *
 * 事实以 `docs/THIRD-PARTY-NOTICES.md` 为准 —— 拿不准就先查那一份，别凭印象写许可名。
 * ⚠️ 这张表和**各功能页自己那块 `Credit` 是同一批事实**：用户要求两边都留
 * （功能旁边写一份、关于页再汇总一份），所以改一处就得同步另一处。
 */
const LICENSES: { feature: string; upstream: string; href?: string; license: string; how: string }[] = [
  {
    feature: '工程转换',
    upstream: 'LibreSVIP 2.9.0',
    href: 'https://github.com/SoulMelody/LibreSVIP',
    license: 'Apache-2.0',
    how: '独立进程调用 libresvip-cli.exe，不链接也不修改它的代码',
  },
  {
    feature: '视频解析下载',
    upstream: 'yt-dlp 2026.08.19',
    href: 'https://github.com/yt-dlp/yt-dlp',
    license: 'Unlicense',
    how: '独立进程，读它的 JSON 输出（Unlicense 等于公有领域，无附加义务）',
  },
  {
    feature: '视频解析（扫码登录）',
    upstream: 'qrcode.react 4.2.0',
    href: 'https://github.com/zpao/qrcode.react',
    license: 'ISC',
    how: '把登录二维码画成 SVG（纯前端渲染，不联网）；Cookie 由 Rust 直接写进本机配置，不经过前端',
  },
  {
    feature: '音频处理',
    upstream: 'FFmpeg 9.0.2（gyan.dev essentials）',
    href: 'https://ffmpeg.org/',
    license: 'GPL v3',
    how: '独立进程跑转格式 / 变调变速 / 裁剪 / 响度 / 抽音轨；分发时附 GPL v3 全文并给出源码地址',
  },
  {
    feature: '视频合流转码',
    upstream: 'FFmpeg 9.0.2（gyan.dev essentials）',
    href: 'https://ffmpeg.org/',
    license: 'GPL v3',
    how: '同上，能「-c copy」就不重编码',
  },
  {
    feature: '音轨分离',
    upstream: 'python-audio-separator 0.39.1',
    href: 'https://github.com/nomadkaraoke/python-audio-separator',
    license: 'MIT',
    how: '独立进程；运行时与模型都不随包分发，第一次用要先下好依赖',
  },
  {
    feature: '音轨分离（模型）',
    upstream: 'UVR 系列 BS-RoFormer / MDX，@Anjok07 训练',
    license: '随模型自带说明',
    how: '同上，第一次用按需下载',
  },
  {
    feature: '人声转 MIDI（算法）',
    upstream: 'openvpi/GAME',
    href: 'https://github.com/openvpi/GAME',
    license: 'MIT',
    how: '按它的算法在 Rust 里重写，没有链接或拷贝它的代码',
  },
  {
    feature: '人声转 MIDI（权重）',
    upstream: 'GAME-1.0.3-large-onnx',
    license: 'CC BY-NC-SA 4.0',
    how: '⚠️ 非商业 —— 不随包分发、由界面按需下载；带着它就不能用于商业用途',
  },
  {
    feature: '人声转 MIDI（推理运行时）',
    upstream: 'ONNX Runtime 1.23.2（ort crate）',
    href: 'https://github.com/microsoft/onnxruntime',
    license: 'MIT',
    how: '运行时才加载 onnxruntime.dll（优先借音轨分离那份），不静态链接、不随包分发',
  },
  {
    feature: '歌词处理',
    upstream: '163MusicLyrics',
    href: 'https://github.com/jitwxs/163MusicLyrics',
    license: 'Apache-2.0',
    how: '时间戳多写法解析 / LRC 转 SRT 收尾 / 译文对齐等按它移植，移植处都有行内注释；取歌词的 HTTP 是本程序自研',
  },
  {
    feature: '文字 PV（编辑器）',
    upstream: 'JIZURA v0.9.0 · © 2026 hakoniwa',
    href: 'https://github.com/852wa/JIZURA',
    license: 'MIT',
    how: 'iframe 同源嵌入作者发布的单文件构建产物，界面与功能未改；唯一的改动是把字体来源从 Google Fonts 换成本机文件',
  },
  {
    feature: '文字 PV（随包字体）',
    upstream: 'Google Fonts 12 个家族',
    license: 'SIL OFL 1.1',
    how: '原样随包分发 woff2 子集，未修改字形（OFL 的保留字体名称条款照旧适用）',
  },
  {
    feature: '界面素材',
    upstream: '@ttqtt/liquid-glass-react',
    href: 'https://github.com/Tsdsj/liquid-glass-react',
    license: 'MIT',
    how: '玻璃材质、配色、字号、间距、圆角与动效；按 Apple 设计语言做的独立组件库，不是 Apple 官方产品，也不含 Apple 的字体或图标素材',
  },
  {
    feature: '汉字读音',
    upstream: 'pinyin-data',
    license: 'MIT',
    how: 'app/data/pinyin.json 的读音数据来源，运行时只读这个 JSON',
  },
]

function About({
  state,
  health,
  onRefresh,
  onNavigate,
  onToast,
}: {
  state: AppState | null
  health: HealthInfo | null
  onRefresh: () => Promise<void>
  onNavigate: (id: string) => void
  onToast: (m: string, t?: string) => void
}) {
  const toolsReady = ['ffmpeg', 'ytdlp'].filter((k) => state?.tools?.[k]?.available).length

  /* 「关于作者」那三个按钮：两个外链走系统默认浏览器（和 `Upstream` 同一条路，
     比指望 WebView 处理 `target="_blank"` 稳）；邮箱按钮复制到剪贴板 ——
     界面跑在 http://127.0.0.1 上，是安全上下文，剪贴板接口能用；万一被拒就提示手动抄。 */
  const openInBrowser = (url: string) => {
    api.fsOpen({ url }).catch((err: unknown) => {
      console.warn('打不开系统浏览器：', err)
      onToast('打不开系统浏览器', 'err')
    })
  }
  const copyMail = async () => {
    try {
      await navigator.clipboard.writeText(AUTHOR_MAIL)
      onToast('邮箱已复制', 'ok')
    } catch {
      onToast('复制不了，手动抄一下吧', 'err')
    }
  }
  return (
    <>
      <Panel>
        <PanelHead title="关于 V-Synth-Studio" />
        <div className="stats-row">
          <Stat label="程序版本" value={health?.version ?? state?.version ?? '—'} />
          <Stat label="运行环境" value={health?.node ?? state?.platform ?? '—'} />
          <Stat
            label="进程"
            value={health?.pid ? `PID ${health.pid}` : '—'}
            sub={health?.uptimeSec ? `已运行 ${Math.floor(health.uptimeSec / 60)} 分钟` : undefined}
          />
          <Stat
            label="外部工具"
            value={`${toolsReady} / 2`}
            sub={state?.tools?.python?.available ? '含 Python' : '无 Python'}
          />
        </div>
        <p className="hint">程序根目录：{state?.paths?.root ?? '未读取到'}</p>
        <p className="hint">
          项目地址：
          <Upstream href="https://github.com/QingMu39-Gao/V-Synth-Studio">
            github.com/QingMu39-Gao/V-Synth-Studio
          </Upstream>
        </p>
        <div className="btn-row">
          <Button
            onClick={async () => {
              await onRefresh()
              onToast('已重新读取', 'ok')
            }}
          >
            重新读取配置
          </Button>
          <Button onClick={() => onNavigate('dashboard')}>回到总览</Button>
        </div>
      </Panel>

      {/* 关于作者 → 作者的话 → 感谢名单 → 源码许可总表，顺序是用户定的：
          人名在前、致谢在后，许可垫底。别把许可挪回前面。 */}
      <Panel>
        <PanelHead
          title="关于作者"
          desc="初次见面的人初次见面，好久不见的人好久不见 —— 一个热爱 Vocaloid 的普通人"
        />
        <div className="author-name">叫我清沐就好</div>
        <div className="btn-row">
          <Button
            size="sm"
            icon="bilibili"
            onClick={() => openInBrowser('https://b23.tv/qfAgBjQ')}
          >
            哔哩哔哩
          </Button>
          <Button
            size="sm"
            icon="douyin"
            onClick={() =>
              openInBrowser(
                'https://www.douyin.com/user/MS4wLjABAAAAC4OEMmA9ito6EUZwSHNw2pZQ7e5pqEPH3EJhDEfs3jqiT4EwydjMLiD2QMVrZYy0?from_tab_name=main',
              )
            }
          >
            抖音
          </Button>
          <Button size="sm" icon="mail" onClick={copyMail}>
            {AUTHOR_MAIL}
          </Button>
        </div>
        <p className="hint">
          前两个按钮在系统默认浏览器里打开（和资源库、上游链接走同一条路）；邮箱按钮点一下复制到剪贴板。
        </p>
      </Panel>

      <Panel>
        <PanelHead title="作者的话" />
        <p className="muted">
          这个项目由 DeepSeek、Claude 等智能体辅助开发。最初只是清沐想集结各种方便的功能，便于虚拟歌姬调教罢了。
          作者也只是个学生，很感谢大家的支持呀 —— 这个项目一半的资金都是大家赞助的！真的很谢谢大家！
        </p>
      </Panel>

      <Panel>
        <PanelHead title="感谢名单" desc="赞助者 —— 排名无先后顺序" />
        <div className="chips">
          {SPONSORS.map((n) => (
            <Chip key={n}>{n}</Chip>
          ))}
        </div>
        <p className="hint">以及一些无法展示 id 的用户和测试者们，同样谢谢你们。</p>
      </Panel>

      {/* 全量许可清单：所有用到第三方开源项目的功能在这儿各占一行。
          各功能页自己那块 `Credit` 仍然保留（用户要求两边都留），改一处要同步另一处。 */}
      <Panel>
        <PanelHead title="源码许可总表" desc="所有用到第三方开源项目的功能，按功能逐项列出" />
        <div className="lic-list">
          {LICENSES.map((l) => (
            <div className="lic-row" key={l.feature}>
              <div className="lic-feature">{l.feature}</div>
              <div>
                <div className="lic-head">
                  <span className="lic-upstream">
                    {l.href ? <Upstream href={l.href}>{l.upstream}</Upstream> : l.upstream}
                  </span>
                  <Chip>{l.license}</Chip>
                </div>
                <p className="hint">{l.how}</p>
              </div>
            </div>
          ))}
        </div>
        <p className="hint">
          本程序自身是 Rust 写的：Tauri 2（Apache-2.0 / MIT）与各 crate 静态链接进 exe，界面跑在系统自带的
          WebView2 Runtime 上；许可全文与逐项说明随仓库的 docs/THIRD-PARTY-NOTICES.md 一起分发。
          早期格式转换用过 UtaFormatix3，相关代码已全部删除，不再需要署名。
        </p>
      </Panel>
    </>
  )
}
