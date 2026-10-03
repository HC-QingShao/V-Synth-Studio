import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { GlassDialog, List, ListRow, ListSection, PathBar } from '@ttqtt/liquid-glass-react'
import {
  api,
  midiFileUrl,
  MIDI_LANGUAGES,
  type FsEntry,
  type MidiNote,
  type MidiStatus,
} from '@/lib/api'
import { Button } from '@/components/Button'
import { Chip, Finding, Panel, PanelHead, Stat } from '@/components/Panel'
import { Field, TextInput } from '@/components/Field'
import { DirectoryInput } from '@/components/DirPicker'
import { formatBytes } from '@/lib/format'
import { useJob } from '@/lib/useJob'
import type { PageProps } from './types'
import './Midi.css'

/**
 * 人声转 MIDI —— 把干声扒成音符（GAME 的原生 Rust 移植）。
 *
 * ## 它在工作流的哪一步
 *
 * 放在「音轨分离」**后面**：分离页把伴奏剥掉、拿到干声，这一页把干声变成 MIDI。
 * 所以页面上第一句话就该把这条路说清楚，而不是让用户猜输入该给什么。
 *
 * ## 和音轨分离页的形态差异（决定了这一页长什么样）
 *
 * 音轨分离背后是**一个 Python 子进程**：要起服务、要探端口、要健康检查，
 * 服务忙完自动关。这一页背后**就是这个进程本身** —— `crate::game::engine`
 * 直接算，ONNX Runtime 用 `ort` crate 动态加载。由此派生出几条：
 *
 *   * **没有「服务起没起」**，也没有启动按钮。`status.running` 装的是
 *     「正在跑的那次任务 id」（同时只允许一个），不是服务的生死。
 *   * **第一次用要下 364 MB 模型包**（官方 ONNX 导出，三个 `.onnx` + `config.json`；
 *     我们从自己的 CDN 托管同一份，见 `midi_transcribe.rs::MODEL_URL`）。
 *     权重是 **CC BY-NC-SA 4.0（非商业）**，不随包发 —— 界面上必须把许可写出来。
 *   * **动态库可能白捡**：装了音轨分离的话，它运行时里那份 `onnxruntime.dll`
 *     直接拿来用（`status.runtime.borrowed`）。有它就别让用户白下那个 78 MB 的 zip
 *     （里面真正要的只有 15 MB 的 dll）。
 *
 * ## ⚠️ 这一页最重要的一个数字：它很慢
 *
 * 本机纯 CPU（AMD RX 580，没有任何可用的 GPU 后端）实测**约 10 秒墙钟换 1 秒
 * 音频** —— 见 `app/desktop/src/game/engine.rs` 与 `midi_transcribe.rs` 的实测
 * 记录。3 分钟干声就是半小时。这个事实必须**在点按钮之前**说出来，不能让用户
 * 提交完才发现。耗时几乎与「去噪步数」成正比，所以步数是这一页最值得动的旋钮。
 *
 * DirectML 试过了、此路不通（三个图里有一条 Reshape 它处理不了，一推理就抛
 * `MLOperatorAuthorImpl.cpp(2597)`）。所以界面上**不提供 GPU 选项**，
 * 只提供线程数 —— 那个是真有效的（1 线程 2.66s / 4 线程 1.04s / 8 线程 3.30s 每步）。
 */

/* ══════════════════════════════════════════════════════════ 常量 ══ */

/** 轮询间隔（毫秒）。状态只是读几个文件大小，便宜。 */
const POLL_STATUS = 2000

/**
 * 实测吞吐：**约 10 秒墙钟 / 1 秒音频**（10.68 s 干声跑了 106 s）。
 *
 * 用它给用户一个量级，而不是精确承诺 —— 文案里写「按本机实测估」。
 * 机器不同差距很大（有 CUDA 会快一个数量级），所以这只是个提示，不参与判断。
 */
const SECONDS_PER_AUDIO_SECOND = 10

/** 去噪步数。8 是上游默认；耗时几乎与它线性相关，所以文案里要说清。 */
const STEP_CHOICES = [4, 8, 16, 32] as const

/** 输入的音频扩展名（后端 ffmpeg 能解的都能给，这里只是文件选择器的过滤） */
const AUDIO_EXTS = ['mp3', 'wav', 'flac', 'm4a', 'aac', 'ogg', 'opus', 'wma', 'aiff', 'ape']

const OUT_DIR_KEY = 'qingmu.midi.outDir'

/* ══════════════════════════════════════════════════════════ 小工具 ══ */

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

function baseName(p: string): string {
  const seg = p.replace(/[\\/]+$/, '').split(/[\\/]/)
  return seg[seg.length - 1] || p
}

function dirName(p: string): string {
  const i = Math.max(p.lastIndexOf('\\'), p.lastIndexOf('/'))
  return i > 0 ? p.slice(0, i) : ''
}

/** 秒 → `3 分 12 秒` / `45 秒`。估时用的，不追求精确。 */
function humanSecs(sec: number): string {
  const s = Math.max(0, Math.round(sec))
  if (s < 60) return `${s} 秒`
  const m = Math.floor(s / 60)
  const r = s % 60
  if (m < 60) return r > 0 ? `${m} 分 ${r} 秒` : `${m} 分`
  return `${Math.floor(m / 60)} 小时 ${m % 60} 分`
}

/** 半音号 → `A3` 这样的音名（69 → A4，和 MIDI 的记法一致） */
function midiName(n: number): string {
  const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
  return `${NAMES[((n % 12) + 12) % 12]}${Math.floor(n / 12) - 1}`
}

/* ══════════════════════════════════════════════════ 选音频文件 ══ */

/**
 * 挑一个音频文件。
 *
 * `components/DirPicker.tsx` 只选目录（后端 `files=0`），所以这里用同一套库组件
 * （`GlassDialog` + `PathBar` + `List`）再拼一个选文件的 —— 和 `pages/Audio.tsx`
 * 的 `MediaPicker`、`pages/Convert.tsx` 的 `FilePicker` 是同一个做法，
 * 样式复用 `index.css` 里那组 `.dir-*`。
 *
 * 为什么不复用 `Audio.tsx` 里那个：它在文件内部、没有 `export`，导出它会让两个
 * 页面之间多一条没必要的依赖；这段本身只有几十行。
 */
function AudioPicker({
  open,
  onOpenChange,
  onPick,
  initialDir,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onPick: (path: string) => void
  initialDir: string
}) {
  const [cwd, setCwd] = useState('')
  const [entries, setEntries] = useState<FsEntry[]>([])
  const [roots, setRoots] = useState<{ name: string; path: string }[]>([])
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const load = useCallback(async (path: string) => {
    setBusy(true)
    setErr(null)
    try {
      const data = await api.fsList(path, { files: true, exts: AUDIO_EXTS })
      setCwd(data.path)
      setEntries(data.entries)
    } catch (e) {
      setErr(errText(e))
    } finally {
      setBusy(false)
    }
  }, [])

  const started = useRef(false)
  useEffect(() => {
    if (!open) {
      started.current = false
      return
    }
    if (started.current) return
    started.current = true
    api
      .fsRoots()
      .then((d) => {
        setRoots(d.roots)
        void load(initialDir)
      })
      .catch((e: unknown) => setErr(errText(e)))
  }, [open, initialDir, load])

  const segments = cwd.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean)
  const dirs = entries.filter((e) => e.dir)
  const files = entries.filter((e) => !e.dir)

  return (
    <GlassDialog
      open={open}
      onOpenChange={onOpenChange}
      title="选择音频"
      description="点进子目录，然后点一个音频文件"
      className="dir-dialog"
    >
      <div className="dir-body">
        <PathBar
          aria-label="所在路径"
          items={[
            { key: 'root', label: '此电脑', onSelect: () => void load('') },
            ...segments.map((seg, i) => ({
              key: seg + i,
              label: seg,
              /* 最后一级不给 onSelect —— 当前项不是链接（和 DirPicker 同一条规矩） */
              onSelect:
                i === segments.length - 1
                  ? undefined
                  : () => void load(segments.slice(0, i + 1).join('\\')),
            })),
          ]}
        />

        {roots.length > 0 && (
          <List className="dir-roots">
            {roots.map((r) => (
              <ListRow
                key={r.path}
                label={r.name}
                secondaryLabel={r.path}
                onSelect={() => void load(r.path)}
              />
            ))}
          </List>
        )}

        <List>
          <ListSection header={busy ? '读取中…' : `${dirs.length} 个子目录`}>
            {dirs.map((e) => (
              <ListRow key={e.path} label={e.name} disclosure onSelect={() => void load(e.path)} />
            ))}
            {!busy && dirs.length === 0 && <ListRow label="（没有子目录）" disabled />}
          </ListSection>
          <ListSection header={`${files.length} 个音频文件`}>
            {files.map((e) => (
              <ListRow
                key={e.path}
                label={e.name}
                secondaryLabel={e.size ? formatBytes(e.size) : undefined}
                onSelect={() => {
                  onPick(e.path)
                  onOpenChange(false)
                }}
              />
            ))}
            {!busy && files.length === 0 && <ListRow label="（这个目录里没有音频文件）" disabled />}
          </ListSection>
        </List>

        {err && <p className="finding-text">{err}</p>}
        <p className="dir-note">当前：{cwd || '（未选择）'}</p>
      </div>

      <div className="dir-actions">
        <span className="spacer" />
        <Button size="sm" variant="ghost" onClick={() => onOpenChange(false)}>
          取消
        </Button>
      </div>
    </GlassDialog>
  )
}

/* ══════════════════════════════════════════════════ 钢琴卷帘 ══ */

/**
 * 结果预览 —— 一个极简钢琴卷帘。
 *
 * 后端只回**前 200 个音符**（`app/desktop/src/server/midi.rs` 的 `result.preview`）：
 * 一首歌几千个音符没必要塞进每 2 秒一次的轮询回包里，要全的都在 `.json` 里。
 * 所以图上写清「预览前 N 个」—— 不然用户会以为整首歌只有这么点音符。
 *
 * 音高按**实际出现的范围**铺满，而不是固定 0..127：干声通常只在两个八度里，
 * 固定量程会把它压成一条线。
 */
function PianoRoll({ notes }: { notes: MidiNote[] }) {
  const box = useMemo(() => {
    if (notes.length === 0) return null
    let lo = Infinity
    let hi = -Infinity
    let end = 0
    for (const n of notes) {
      if (n.midi < lo) lo = n.midi
      if (n.midi > hi) hi = n.midi
      if (n.offset > end) end = n.offset
    }
    // 上下各留 2 个半音的余量，免得最高的音贴着顶边
    lo -= 2
    hi += 2
    return { lo, span: Math.max(1, hi - lo), end: Math.max(0.001, end) }
  }, [notes])

  if (!box) return null
  const W = 1000
  const H = 160
  const rowH = H / box.span

  return (
    <svg
      className="midi-roll"
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
      role="img"
      aria-label={`钢琴卷帘预览，${notes.length} 个音符`}
    >
      {notes.map((n, i) => {
        const x = (n.onset / box.end) * W
        const w = Math.max(1.5, ((n.offset - n.onset) / box.end) * W)
        const y = H - (n.midi - box.lo + 0.5) * rowH
        return (
          <rect
            key={i}
            x={x}
            y={y - rowH * 0.45}
            width={w}
            height={Math.max(2, rowH * 0.9)}
            rx={1.5}
          />
        )
      })}
    </svg>
  )
}

/* ══════════════════════════════════════════════════════════ 页面 ══ */

export function Midi({ onToast, onNavigate }: PageProps) {
  const [st, setSt] = useState<MidiStatus | null>(null)
  const [statusErr, setStatusErr] = useState<string | null>(null)

  const [input, setInput] = useState('')
  const [outDir, setOutDir] = useState(() => {
    try {
      return localStorage.getItem(OUT_DIR_KEY) ?? ''
    } catch {
      return ''
    }
  })
  const [steps, setSteps] = useState(8)
  const [language, setLanguage] = useState(4)
  const [threads, setThreads] = useState(4)

  const [picker, setPicker] = useState(false)
  const [busy, setBusy] = useState(false)
  /** 输入文件的时长（秒）。拿 `audioProbe` 探；探不到就是 0，那时不给估时。 */
  const [duration, setDuration] = useState(0)

  const { job, start, stop } = useJob()
  /** 正在跑的那次任务 id（提交后记下，用来调 cancel 与拼结果下载地址） */
  const [taskId, setTaskId] = useState<string | null>(null)

  /* ── 轮询状态 ─────────────────────────────────────────── */
  const refresh = useCallback(async () => {
    try {
      const v = await api.midiStatus()
      setSt(v)
      setStatusErr(null)
    } catch (e) {
      setStatusErr(errText(e))
    }
  }, [])

  useEffect(() => {
    void refresh()
    const t = window.setInterval(() => void refresh(), POLL_STATUS)
    return () => window.clearInterval(t)
  }, [refresh])

  /* 离开页面时收干净订阅（`useJob` 内部也做了，这里显式一点） */
  useEffect(() => () => stop(), [stop])

  const rememberOutDir = (v: string) => {
    setOutDir(v)
    try {
      localStorage.setItem(OUT_DIR_KEY, v)
    } catch {
      /* 隐私模式 */
    }
  }

  /* ── 探测输入时长（只用来估时）─────────────────────────── */
  useEffect(() => {
    const path = input.trim()
    if (!path) {
      setDuration(0)
      return
    }
    let alive = true
    api
      .audioProbe(path)
      .then((r) => {
        if (!alive) return
        const d = Number(r.info?.durationSec ?? 0)
        setDuration(Number.isFinite(d) && d > 0 ? d : 0)
      })
      .catch(() => {
        /* 探不到就没有估时 —— 不该因此报错，提交时后端还会再探一次 */
        if (alive) setDuration(0)
      })
    return () => {
      alive = false
    }
  }, [input])

  const ready = !!st?.models.ready && !!st?.runtime.ready
  const running = !!job && job.status === 'running'
  const dl = st?.download

  /* 估时：实测吞吐 × 时长 × 步数比例。只是量级，不是承诺。 */
  const estimate = useMemo(
    () => (duration > 0 ? (duration * SECONDS_PER_AUDIO_SECOND * steps) / 8 : 0),
    [duration, steps],
  )

  /* ── 动作 ─────────────────────────────────────────────── */

  const doDownloadModels = async () => {
    setBusy(true)
    try {
      await api.midiDownloadModels()
      onToast(`开始下载模型（${st ? formatBytes(st.models.zipBytes) : '整个模型包'}）`, 'info')
      void refresh()
    } catch (e) {
      onToast(errText(e), 'err')
    } finally {
      setBusy(false)
    }
  }

  const doDownloadRuntime = async () => {
    setBusy(true)
    try {
      await api.midiDownloadRuntime()
      onToast('开始下载 ONNX Runtime（约 78 MB）', 'info')
      void refresh()
    } catch (e) {
      onToast(errText(e), 'err')
    } finally {
      setBusy(false)
    }
  }

  const doStopDownload = async () => {
    setBusy(true)
    try {
      await api.midiStopDownload()
      onToast('已请求停止下载', 'info')
      void refresh()
    } catch (e) {
      onToast(errText(e), 'err')
    } finally {
      setBusy(false)
    }
  }

  const doDeleteDeps = async () => {
    // 兜底只在「状态还没拉回来」时用得上，所以不写死体积（写死会过期）。
    const mb = st ? formatBytes(st.models.zipBytes) : '整个模型包'
    /* ⚠️ 这份提示只为安装版那种「引擎在用随包那份、删除删不到它」的情况准备。
       `local`（绿色版两层同一路径）**不能**说这句 —— 那儿删掉就真没了、按钮会回来。
       判据是 `origin`，别按路径尾巴猜（三种情况的路径都以 `\game\models` 结尾）。 */
    const warn =
      st?.models.origin === 'bundled'
        ? '\n\n注意：现在引擎用的是「随包自带」那份模型，这个按钮只清下载物、\n不会删它，所以删完状态还是「就绪」。'
        : ''
    if (!window.confirm(`删掉下好的模型与动态库？下次要用得重新下 ${mb}。${warn}`)) return
    setBusy(true)
    try {
      const r = await api.midiDeleteDeps()
      // ⚠️ 必须把后端的 `note` 一起显示：**安装版**下模型分两层，「删掉下下来
      // 那份」之后引擎可能还靠随包只读那份顶着、状态仍是「就绪」，下载按钮就
      // 不会出现 —— 不说清原因，用户看到的就是「点了删除没反应」。
      // （绿色版两层同路径，这句 `note` 是空串，那时删掉就是真没了。）
      onToast(`已删掉 ${r.files} 个文件（${formatBytes(r.bytes)}）。${r.note}`, 'ok')
      void refresh()
    } catch (e) {
      onToast(errText(e), 'err')
    } finally {
      setBusy(false)
    }
  }

  const doTranscribe = async () => {
    const path = input.trim()
    if (!path) {
      onToast('先选一个音频文件', 'warn')
      return
    }
    setBusy(true)
    try {
      const r = await api.midiTranscribe({
        input: path,
        outDir: outDir.trim() || undefined,
        steps,
        language,
        threads,
      })
      setTaskId(r.jobId)
      start(r.jobId, {
        onDone: () => {
          onToast('扒谱完成', 'ok')
          void refresh()
        },
        onError: (err) => {
          onToast(err.message, 'err')
          void refresh()
        },
        onCancel: () => {
          onToast('已取消', 'info')
          void refresh()
        },
      })
      onToast('已提交，跑的时候界面照常能用', 'info')
    } catch (e) {
      onToast(errText(e), 'err')
    } finally {
      setBusy(false)
    }
  }

  const doCancel = async () => {
    if (!taskId) return
    try {
      await api.midiCancel(taskId)
      onToast('已请求取消（当前这一段跑完才停）', 'info')
    } catch (e) {
      onToast(errText(e), 'err')
    }
  }

  /* ── 结果 ─────────────────────────────────────────────── */
  const result = (job?.result ?? null) as
    | {
        dir?: string
        files?: string[]
        notes?: number
        seconds?: { encoder?: number; segmenter?: number; estimator?: number }
        preview?: MidiNote[]
      }
    | null

  const dlPct = dl && dl.total > 0 ? Math.min(100, (dl.done / dl.total) * 100) : 0

  /* ── 渲染 ─────────────────────────────────────────────── */
  return (
    <div className="page-body midi-layout">
      {/* ══════════════ 左栏：素材 + 参数 ══════════════ */}
      <div className="midi-col">
        <Panel>
          <PanelHead
            title="干声素材"
            desc="把「音轨分离」拆出来的人声给这里；直接给整首歌也行，但伴奏会干扰判音高"
            extra={input ? <Chip tone="ok">已选</Chip> : <Chip>未选</Chip>}
          />
          <input
            type="text"
            className="input"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="D:\歌\干声.wav"
            spellCheck={false}
          />
          <div className="btn-row">
            <Button icon="folder" onClick={() => setPicker(true)}>
              选音频文件
            </Button>
            {input && (
              <Button variant="ghost" icon="x" onClick={() => setInput('')}>
                清空
              </Button>
            )}
          </div>
          {input ? (
            <p className="hint">
              {baseName(input)}
              {duration > 0 ? ` · ${humanSecs(duration)}` : ''}
            </p>
          ) : (
            <p className="hint">
              还没做过分离？
              <button type="button" className="midi-link" onClick={() => onNavigate('svsep')}>
                先去音轨分离页
              </button>
              拆出干声。
            </p>
          )}
        </Panel>

        <Panel>
          <PanelHead title="参数" desc="只有「去噪步数」值得反复试，其余照默认就行" />
          <Field
            label="去噪步数"
            hint={`上游默认 8。耗时几乎与它成正比 —— 32 步大约是 8 步的四倍。${
              estimate > 0 ? `按当前设置估约 ${humanSecs(estimate)}。` : ''
            }`}
          >
            <div className="midi-steps">
              {STEP_CHOICES.map((s) => (
                <button
                  key={s}
                  type="button"
                  className="midi-step"
                  data-on={steps === s ? 'true' : undefined}
                  onClick={() => setSteps(s)}
                >
                  {s}
                </button>
              ))}
            </div>
          </Field>

          <Field label="语言" hint="告诉它唱的是哪种语言，能提高音符边界的准确度">
            <select
              className="input"
              value={language}
              onChange={(e) => setLanguage(Number(e.target.value))}
            >
              {MIDI_LANGUAGES.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.label}
                </option>
              ))}
            </select>
          </Field>

          <Field
            label="线程数"
            hint="本机实测 4 最快（每步：1 线程 2.66s / 4 线程 1.04s / 8 线程 3.30s）—— 图窄，线程开多了反而被超订拖慢"
          >
            <TextInput
              type="number"
              min={1}
              max={32}
              value={threads}
              onChange={(e) => setThreads(Number(e.target.value) || 4)}
            />
          </Field>
        </Panel>

        <Panel>
          <PanelHead title="输出" desc="留空就写到音频同目录下的 midi 文件夹" />
          <DirectoryInput
            value={outDir}
            onChange={rememberOutDir}
            placeholder="留空 = 音频同目录\midi"
          />
          <div className="btn-row">
            <Button
              variant="ghost"
              icon="folder"
              disabled={!result?.dir}
              onClick={() => {
                if (!result?.dir) return
                void api.midiOpenOutput(result.dir).catch((e) => onToast(errText(e), 'err'))
              }}
            >
              打开输出目录
            </Button>
          </div>
        </Panel>
      </div>

      {/* ══════════════ 右栏：状态 + 结果 ══════════════ */}
      <div className="midi-col">
        <Panel>
          <PanelHead
            title="扒谱"
            desc="提交后会在后台跑，进度看下面；界面照常能用"
            extra={
              running ? (
                <Chip tone="accent">运行中</Chip>
              ) : ready ? (
                <Chip tone="ok">可以开始</Chip>
              ) : (
                <Chip tone="warn">缺依赖</Chip>
              )
            }
          />

          {statusErr && (
            <Finding level="warn" title="读不到状态">
              {statusErr}
            </Finding>
          )}

          {!ready && st && (
            <Finding level="warn" title="第一次用要先下模型（不随包发）">
              三个 ONNX 图要下 <strong>{formatBytes(st.models.zipBytes)}</strong>
              （解开后 {formatBytes(st.models.extractBytes)}）。权重许可是{' '}
              <strong>{st.license}</strong>
              ，所以要自己下、不随安装包分发 —— 点下面的按钮从官方 release 取。
              {st.models.partBytes > 0 && (
                <>
                  {' '}
                  ⚠️ 盘上还留着上次没下完的 {formatBytes(st.models.partBytes)}，它
                  <strong>不支持续传</strong>，再点一次是从头下。
                </>
              )}
            </Finding>
          )}

          <div className="midi-stats">
            <Stat
              label="模型"
              value={st ? (st.models.ready ? '就绪' : `缺 ${st.models.missing.length} 个`) : '…'}
              sub={st?.models.ready ? st.models.dir : st?.models.missing.join('、')}
            />
            <Stat
              label="ONNX Runtime"
              value={st ? (st.runtime.ready ? '就绪' : '缺') : '…'}
              sub={st?.runtime.borrowed ? '复用音轨分离的运行时' : st?.runtime.dll ?? '没找到'}
            />
            <Stat
              label="耗时"
              value={estimate > 0 ? `约 ${humanSecs(estimate)}` : '—'}
              sub={duration > 0 ? `${humanSecs(duration)} 素材 · 纯 CPU` : '选了文件后估算'}
            />
          </div>

          {!ready && (
            <div className="btn-row">
              <Button
                variant="primary"
                icon="download"
                disabled={busy || !!dl?.active}
                onClick={() => void doDownloadModels()}
              >
                {st?.models.ready
                  ? '模型已就绪'
                  : `下载模型（${st ? formatBytes(st.models.zipBytes) : '整个模型包'}）`}
              </Button>
              {!st?.runtime.ready && (
                <Button
                  icon="download"
                  disabled={busy || !!dl?.active}
                  onClick={() => void doDownloadRuntime()}
                >
                  {`下载运行库（${st ? formatBytes(st.runtime.zipBytes) : '78 MB'}）`}
                </Button>
              )}
            </div>
          )}

          {dl?.active && (
            <div className="midi-progress">
              <div className="midi-progress-head">
                {/* 下载与解压共用一条进度条，标签必须说清是哪一段：解压的分母跟
                    整包字节数差不多大，只写「正在下载」就成了「下到 100% 又归零
                    重爬」，看着像下完又重下了一遍 */}
                <span>
                  {dl.stage === 'extract'
                    ? '正在解压模型…'
                    : `正在下载${dl.kind === 'runtime' ? '运行库' : '模型'}…`}
                </span>
                <span className="midi-dim">
                  {formatBytes(dl.done)}
                  {dl.total > 0 ? ` / ${formatBytes(dl.total)}` : ''}
                </span>
              </div>
              <div
                className="midi-bar"
                role="progressbar"
                aria-valuenow={dl.total > 0 ? dlPct : undefined}
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <span className="midi-bar-fill" style={{ width: `${dlPct}%` }} />
              </div>
              <div className="btn-row">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => void doStopDownload()}
                >
                  停止下载
                </Button>
              </div>
              <p className="hint">
                这个包<strong>不支持续传</strong>（停下就是重来），所以这里只有「停止」没有「暂停」。
              </p>
            </div>
          )}

          {dl?.error && (
            <Finding level="warn" title="下载失败">
              {dl.error}
            </Finding>
          )}

          <div className="btn-row">
            <Button
              variant="primary"
              icon="play"
              disabled={busy || !ready || running || !input.trim()}
              onClick={() => void doTranscribe()}
            >
              {running ? '正在扒谱…' : '开始扒谱'}
            </Button>
            {running && (
              <Button variant="danger" icon="x" onClick={() => void doCancel()}>
                取消
              </Button>
            )}
          </div>

          {estimate > 0 && !running && (
            <p className="hint">
              ⚠️ 本机纯 CPU，实测<strong>约 10 秒换 1 秒音频</strong> —— 3 分钟干声就是半小时左右。
              嫌慢就把「去噪步数」调小。
            </p>
          )}
        </Panel>

        {(running || job) && (
          <Panel>
            <PanelHead
              title="进度"
              desc={job?.title}
              extra={
                job?.status === 'done' ? (
                  <Chip tone="ok">完成</Chip>
                ) : job?.status === 'canceled' ? (
                  <Chip tone="warn">已取消</Chip>
                ) : job?.status === 'error' ? (
                  <Chip tone="err">失败</Chip>
                ) : (
                  <Chip tone="accent">运行中</Chip>
                )
              }
            />
            <div className="midi-progress">
              <div className="midi-progress-head">
                <span>{job?.message ?? '正在准备…'}</span>
                <span className="midi-dim">{Math.round(job?.percent ?? 0)}%</span>
              </div>
              <div
                className="midi-bar"
                role="progressbar"
                aria-valuenow={job?.percent ?? 0}
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <span
                  className="midi-bar-fill"
                  style={{ width: `${Math.min(100, job?.percent ?? 0)}%` }}
                />
              </div>
            </div>
            {job?.error && (
              <Finding level="warn" title="失败">
                {job.error}
              </Finding>
            )}
            {(job?.logs?.length ?? 0) > 0 && (
              <div className="midi-logs">
                {(job?.logs ?? []).slice(-6).map((l, i) => (
                  <p key={i}>{l}</p>
                ))}
              </div>
            )}
          </Panel>
        )}

        {result?.preview && result.preview.length > 0 && (
          <Panel>
            <PanelHead
              title="结果"
              desc={`${result.notes} 个音符 · 预览前 ${result.preview.length} 个`}
              extra={<Chip tone="ok">已写出</Chip>}
            />
            <div className="midi-roll-box">
              <PianoRoll notes={result.preview} />
              <div className="midi-roll-axis">
                <span>{result.preview[0]?.onset.toFixed(2)}s</span>
                <span>{result.preview[result.preview.length - 1]?.offset.toFixed(2)}s</span>
              </div>
            </div>
            <div className="midi-notes">
              <div className="midi-notes-head">
                <span>起点</span>
                <span>时长</span>
                <span>音高</span>
              </div>
              {result.preview.slice(0, 40).map((n, i) => (
                <div className="midi-note-row" key={i}>
                  <span>{n.onset.toFixed(2)}s</span>
                  <span>{(n.offset - n.onset).toFixed(2)}s</span>
                  <span>
                    {midiName(n.midi)}
                    <span className="midi-dim"> ({n.midi})</span>
                  </span>
                </div>
              ))}
              {result.preview.length > 40 && (
                <p className="hint">还有 {result.preview.length - 40} 个在结果文件里。</p>
              )}
            </div>
            {taskId && (result.files?.length ?? 0) > 0 && (
              <div className="btn-row">
                {(result.files ?? []).map((f) => (
                  <a key={f} className="btn btn-sm" href={midiFileUrl(taskId, f)} download={f}>
                    {f}
                  </a>
                ))}
              </div>
            )}
            {result.seconds && (
              <p className="hint">
                特征 {result.seconds.encoder?.toFixed(1)}s · 去噪{' '}
                {result.seconds.segmenter?.toFixed(1)}s · 判音高{' '}
                {result.seconds.estimator?.toFixed(1)}s
              </p>
            )}
          </Panel>
        )}

        <Panel>
          <PanelHead title="许可与出处" desc="模型与代码是两套许可，界面上必须写清" />
          <div className="midi-stats">
            <Stat label="代码" value="MIT" sub="openvpi/GAME" />
            <Stat label="权重" value="CC BY-NC-SA 4.0" sub="非商业 —— 不随包分发" />
          </div>
          <p className="hint">
            模型是官方 ONNX release（
            <a href={st?.source ?? 'https://github.com/openvpi/GAME'} target="_blank" rel="noreferrer">
              {st?.source ?? 'github.com/openvpi/GAME'}
            </a>
            ）。推理、解码、切片与 MIDI 写出全部在本进程里用 Rust 实现，没有 Python 子进程。
          </p>
          {st && (
            <>
              {/* ⚠️ 这一条只在**安装版**那种「两层是两个不同目录、引擎在用随包那份」
                  的情况下才该出现：那时点「删掉下好的依赖」删不到引擎正在用的模型，
                  状态还是「就绪」、下载按钮不会回来 —— 不说清用户只会以为按钮坏了。
                  ⛔ 判据只能是后端回的 `models.origin`。别在前端按 `dir` 的尾巴猜：
                  三种情况（downloaded / bundled / local）的路径都以 `\game\models` 结尾。
                  `local` = 绿色版两层同一路径，没有第二层可回落，所以那边不用说话。 */}
              {st.models.ready && st.models.origin === 'bundled' && (
                <p className="hint">
                  引擎现在用的是「随包自带」那一层 <code>{st.models.dir}</code>，不是下载来的。
                  「删掉下好的依赖」只清可写目录里的下载物，
                  <strong>不动随包的安装内容</strong>，所以删完状态还是「就绪」、
                  下载按钮也不会出现（要试下载流程得手工把那一层挪走）。
                </p>
              )}
              <div className="btn-row">
                <Button
                  size="sm"
                  variant="ghost"
                  icon="trash"
                  disabled={busy || !!dl?.active}
                  onClick={() => void doDeleteDeps()}
                >
                  删掉下好的依赖
                </Button>
                <Button size="sm" variant="ghost" icon="refresh" onClick={() => void refresh()}>
                  刷新状态
                </Button>
              </div>
            </>
          )}
        </Panel>
      </div>

      <AudioPicker
        open={picker}
        onOpenChange={setPicker}
        initialDir={input ? dirName(input) : ''}
        onPick={(p) => {
          setInput(p)
          onToast(`已选：${baseName(p)}`, 'ok')
        }}
      />
    </div>
  )
}

export default Midi
