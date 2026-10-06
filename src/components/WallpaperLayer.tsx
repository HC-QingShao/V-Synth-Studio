/**
 * 背景壁纸层 —— 界面最底下那一层，`body::before`（静态背景图）的替代品。
 *
 * ⚠️ **它必须不透明**：玻璃材质靠 `backdrop-filter` 采样「身后有什么」，这一层一透明，
 * 整个界面的玻璃会**静默失效**。所以底色先铺一个 `--lg-bg-grouped`，再往上放壁纸。
 *
 * ⚠️ **场景壁纸跑在 sandbox iframe 里**（`allow-scripts`，**不给** `allow-same-origin`）：
 * 场景里的 SceneScript 是别人的代码，webwallgl 的脚本沙箱只是 JS 语义隔离、不是安全
 * 边界。不透明源之后它连父窗口都拿不到，自然也碰不到 `window.__TAURI_INTERNALS__`。
 *
 * ⚠️ **video / image 不走 webwallgl**：它的媒体路径要 `texImage2D(视频帧)`，而壁纸文件
 * 走 asset 协议（跨源）→ 跨源视频会污染 WebGL 纹理、直接抛安全错。原生 `<video>` 没有
 * 这个问题，观感一样（视频壁纸本来就是一段视频铺满）。
 *
 * ⚠️ **渲染失败要说话**：这个库还年轻，场景里用到的特性（自定义着色器、模型、脚本）
 * 不是都支持。失败时**退到预览图并说明原因** —— 静默退回的话，用户只知道「壁纸没动」。
 */

import {useEffect, useRef, useState} from 'react'

import {api} from '@/lib/api'
import {getConfig, onConfigChange} from '@/lib/config'
import {fileUrl} from '@/lib/ipc'
import {loadPkg, planWallpaper, useWeScan, type WallpaperPlan} from '@/lib/wallpaper'

import './WallpaperLayer.css'

/** 宿主页（`public/vendor/wallpaper/index.html`）。 */
const HOST_PAGE = '/vendor/wallpaper/index.html'

export function WallpaperLayer() {
    const {scan} = useWeScan()
    const [cfg, setCfg] = useState(() => getConfig())
    const [error, setError] = useState<string | null>(null)
    const [noticeClosed, setNoticeClosed] = useState(false)
    const frameRef = useRef<HTMLIFrameElement>(null)
    const videoRef = useRef<HTMLVideoElement>(null)

    /* 配置一变就重算（设置页点一下立刻生效）。订阅是模块级的、这个组件只挂一次，
       所以不退订。 */
    useEffect(() => {
        onConfigChange(() => setCfg(getConfig()))
    }, [])

    const plan: WallpaperPlan = planWallpaper(String(cfg['wallpaper'] ?? ''), scan)
    const paused = Boolean(cfg['wallpaperPaused'])
    /** 只跟「哪一张」有关：换壁纸才重挂，暂停/主题变化不重挂。 */
    const planKey = plan.kind === 'none' ? '' : `${plan.kind}:${plan.item.id}`

    const fail = (reason: string) => {
        setError(reason)
        setNoticeClosed(false)
        console.warn('[wallpaper]', reason)
    }

    /* `html[data-wallpaper]` 一在，`body::before` 的静态图就被关掉（见 css）——
       否则两张图叠着。退回静态图时把标记摘掉。 */
    useEffect(() => {
        const root = document.documentElement
        setError(null)
        if (!planKey) delete root.dataset.wallpaper
        else root.dataset.wallpaper = 'on'
        return () => {
            delete root.dataset.wallpaper
        }
    }, [planKey])

    /* 媒体类要先把这个目录放行给 asset 协议，否则 `<video>` 静默 403（控制台里才有）。 */
    useEffect(() => {
        if (plan.kind !== 'video' && plan.kind !== 'image') return
        void api.allowPath(plan.item.dir).catch(() => undefined)
    }, [planKey])

    /* 场景：等宿主页报 `boot`（它把 webwallgl 加载完）再把字节递进去。
       ⚠️ 字节走 `transfer` 转移所有权（几十 MB 的包别复制）。 */
    useEffect(() => {
        if (plan.kind !== 'scene') return
        let alive = true
        const pkgPath = plan.pkg
        const key = plan.item.id
        const onMsg = async (ev: MessageEvent) => {
            const frame = frameRef.current
            if (!frame || ev.source !== frame.contentWindow) return
            const d = (ev.data ?? {}) as {type?: string; message?: string}
            if (d.type === 'boot') {
                try {
                    const pkg = await loadPkg(pkgPath)
                    if (!alive) return
                    /* 30fps：这是背景层，上面还压着玻璃。上游实测帧率上限是省 CPU 最有效
                       的那一档（场景壁纸稳态 -25~38%），而 30fps 的观感对壁纸足够。 */
                    frame.contentWindow?.postMessage({type: 'mount', pkg, key, fps: 30}, '*', [pkg])
                } catch (e) {
                    fail(`场景包没喂进去：${e instanceof Error ? e.message : String(e)}`)
                }
                return
            }
            if (d.type === 'error') fail(`场景渲染失败：${d.message || '未知原因'}`)
            if (d.type === 'diagnostic') console.warn('[wallpaper] 诊断：', d.message)
        }
        window.addEventListener('message', onMsg)
        return () => {
            alive = false
            window.removeEventListener('message', onMsg)
            frameRef.current?.contentWindow?.postMessage({type: 'destroy'}, '*')
        }
    }, [plan.kind === 'scene' ? plan.pkg : ''])

    /* 暂停/恢复：设置里的开关、或者窗口被切到后台（省电，也免得玻璃每帧重算）。 */
    useEffect(() => {
        const apply = () => {
            const stop = paused || document.hidden
            if (plan.kind === 'scene') {
                frameRef.current?.contentWindow?.postMessage({type: stop ? 'pause' : 'resume'}, '*')
            } else {
                const v = videoRef.current
                if (!v) return
                if (stop) v.pause()
                else void v.play().catch(() => undefined)
            }
        }
        apply()
        document.addEventListener('visibilitychange', apply)
        return () => document.removeEventListener('visibilitychange', apply)
    }, [paused, planKey])

    if (plan.kind === 'none') return null

    const failed = error !== null
    const preview = plan.item.preview

    return (
        <>
            <div className="wallpaper" aria-hidden="true">
                {failed ? (
                    /* 失败了就显示这张壁纸的预览图（WE 每张都带 preview.gif/jpg）。
                       连预览都没有时什么都不画 —— 底下就是静态背景图那层。 */
                    preview ? <img className="wallpaper-media" src={fileUrl(preview)} alt=""/> : null
                ) : (
                    <>
                        {plan.kind === 'scene' && (
                            /* `sandbox` 只给 allow-scripts：不透明源，碰不到宿主的 IPC。
                               `key` 跟着场景包走 —— 换壁纸时整页重载，省掉「在旧场景上
                               原地换包」那条路。 */
                            <iframe
                                key={plan.pkg}
                                ref={frameRef}
                                className="wallpaper-frame"
                                src={HOST_PAGE}
                                sandbox="allow-scripts"
                                title=""
                            />
                        )}
                        {plan.kind === 'video' && (
                            <video
                                key={plan.src}
                                ref={videoRef}
                                className="wallpaper-media"
                                src={fileUrl(plan.src)}
                                autoPlay
                                muted
                                loop
                                playsInline
                                /* ⚠️ 视频解不了是**常态**，别让它变成一块黑：`canPlayType`
                                   会说 HEVC「probably」，真放起来却是 `MediaError 4`
                                   （本机那张 STUDY WITH MIKU 就是 HEVC，没有系统的
                                   HEVC 扩展就播不了）。 */
                                onError={(e) => {
                                    const err = (e.target as HTMLVideoElement).error
                                    fail(`视频解不了（MediaError ${err?.code ?? '?'}）：这个编码 WebView2 放不出来`)
                                }}
                            />
                        )}
                        {plan.kind === 'image' && (
                            <img key={plan.src} className="wallpaper-media" src={fileUrl(plan.src)} alt=""/>
                        )}
                    </>
                )}
            </div>
            {failed && !noticeClosed && error && (
                <WallpaperNotice title={plan.item.title} reason={error} onClose={() => setNoticeClosed(true)}/>
            )}
        </>
    )
}

/**
 * 渲染失败时的说明条。
 *
 * ⚠️ 它是 `.wallpaper` 的**兄弟**而不是子节点：那一层是 `pointer-events: none`，
 * 放进去的话「知道了」点不动。所以这里单独 fixed 定位、自己开 `pointer-events`。
 */
function WallpaperNotice({title, reason, onClose}: {title: string; reason: string; onClose: () => void}) {
    return (
        <div className="wallpaper-notice" role="status">
            <div className="wallpaper-notice-text">
                <strong>「{title}」没能渲染</strong>
                <span>已退回这张壁纸的预览图。原因：{reason.slice(0, 300)}</span>
            </div>
            <button type="button" onClick={onClose}>
                知道了
            </button>
        </div>
    )
}
