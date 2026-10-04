import { useCallback, useEffect, useRef, useState } from 'react'

/**
 * 预览 —— 不用先下载就能看一眼。
 *
 * 两种形态，都能出现：
 *
 * | 形态 | 什么时候 | 长什么样 |
 * |---|---|---|
 * | 有音有画 | 站点给了合流直链（或 B 站整段流） | 一个 `<video>` |
 * | 音画分开 | B 站的 DASH 流、yt-dlp 只给了分离轨 | 一个 `<video>`（静音）+ 一个**隐藏但真在播**的 `<audio>` |
 *
 * ⚠️ **隐藏的 `<audio>` 必须留在 DOM 里。** 它是 `display: none`（`.preview-audio`），
 * 但照样能播放；反过来，要是每次播放都新建一个 `new Audio()`，就跟 `<video>` 对不上拍
 * —— 用户看到的是「视频在动、声音零零碎碎」。所以这里只用 ref 操作同一批元素，
 * 靠 `video` 上的 play / pause / seeked / ratechange 事件把 audio 拉回同步。
 *
 * 两条地址都是**本机文件**的 asset 地址（`fileUrl(path)` → `http://asset.localhost/…`）。
 * 远端直链不能直接塞进来：B 站 CDN 看 `Referer`，浏览器直连在部分节点上回 403；
 * 所以页面那边先用 `api.previewFetch` 把流缓存到本机，这里只管播。
 * （2026-10-04 之前这一层是本机 HTTP 反代 `/api/media/proxy`，那条路由已随 HTTP 层删除。）
 */
export function VideoPreview({
  videoUrl,
  audioUrl,
  poster,
  note,
}: {
  videoUrl: string
  /** 有独立音轨时给；没有（或已经有声）就别传 */
  audioUrl?: string
  poster?: string
  /** 底下一句实话：这一段是什么、为什么只有第一段、卡了怎么办 */
  note?: string
}) {
  const videoRef = useRef<HTMLVideoElement>(null)
  const audioRef = useRef<HTMLAudioElement>(null)
  const [failed, setFailed] = useState(false)

  /* 换一条直链 = 换一个视频：失败的标记要跟着清掉，不然第二个视频明明能放，
     屏幕上还写着上一句「这个直链播不了」。 */
  useEffect(() => {
    setFailed(false)
  }, [videoUrl, audioUrl])

  /**
   * 把音轨拉回和画面同一时刻。
   *
   * 容差 **0.25 秒**：人耳对「声画不同步」的容忍度大概就是两三百毫秒，
   * 而每帧都硬对齐会让音频不断做微小的跳变（听上去像爆音）。超了才纠。
   * 暂停状态下只对位置、不播放 —— 否则预览一加载就开始放声音。
   */
  const sync = useCallback((force = false) => {
    const v = videoRef.current
    const a = audioRef.current
    if (!v || !a) return
    if (force || Math.abs(a.currentTime - v.currentTime) > 0.25) {
      a.currentTime = v.currentTime
    }
    a.playbackRate = v.playbackRate
  }, [])

  return (
    <div className="preview">
      <div className="preview-box">
        <video
          ref={videoRef}
          className="preview-video"
          /* ⚠️ `crossOrigin` **不能加**：不加就是普通的跨源取流（媒体元素不需要 CORS 头
             也能播），加了反而要求 asset 协议回一堆我们不需要的头。 */
          src={videoUrl}
          poster={poster || undefined}
          controls
          preload="metadata"
          /* 有独立音轨时画面先静音，声音由下面那个 `<audio>` 出 */
          muted={!!audioUrl}
          onError={() => setFailed(true)}
          onLoadedMetadata={() => sync(true)}
          onPlay={() => {
            sync(true)
            void audioRef.current?.play().catch(() => {
              /* 浏览器可能因为「用户手势」拦下这里 —— 画面照常走，别报错吓人 */
            })
          }}
          onPause={() => audioRef.current?.pause()}
          onSeeked={() => sync(true)}
          onRateChange={() => sync()}
          onTimeUpdate={() => sync()}
          onEnded={() => audioRef.current?.pause()}
        />
      </div>

      {audioUrl && (
        /* `display:none` 但**留在 DOM 里**（见文件头那段）。`src` 只在真的有音轨时给，
           否则它自己也会去请求一次不存在的地址、在控制台留一条报错。 */
        <audio ref={audioRef} className="preview-audio" src={audioUrl} preload="auto" />
      )}

      {failed ? (
        <p className="preview-note preview-err">
          这个直链播不了（站点可能限了直连或已经过期）。直接下载之后本地看是一样的。
        </p>
      ) : (
        note && <p className="preview-note">{note}</p>
      )}
    </div>
  )
}
