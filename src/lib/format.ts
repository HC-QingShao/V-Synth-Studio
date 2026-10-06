/**
 * 格式化 —— 同一份数据在不同页面里要显示成同一个样子，所以只有这一份实现。
 */

export function formatBytes(n: number | undefined | null): string {
    if (!Number.isFinite(n) || (n as number) <= 0) return '-'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    let v = n as number
    let i = 0
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024
        i += 1
    }
    return `${v.toFixed(i === 0 ? 0 : v < 10 ? 2 : 1)} ${units[i]}`
}

export function formatDuration(sec: number | undefined | null): string {
    if (!Number.isFinite(sec) || (sec as number) < 0) return '-'
    const s = Math.round(sec as number)
    const m = Math.floor(s / 60)
    const ss = s % 60
    if (m >= 60) {
        const hh = Math.floor(m / 60)
        return `${hh}:${String(m % 60).padStart(2, '0')}:${String(ss).padStart(2, '0')}`
    }
    return `${m}:${String(ss).padStart(2, '0')}`
}

export function formatNumber(n: number | undefined | null): string {
    if (!Number.isFinite(n)) return '-'
    const v = n as number
    if (v >= 100000000) return `${(v / 100000000).toFixed(1)} 亿`
    if (v >= 10000) return `${(v / 10000).toFixed(1)} 万`
    return String(v)
}

/** 异常 → 能直接展示的一行文本。 */
export function errText(e: unknown): string {
    return e instanceof Error ? e.message : String(e)
}

/** 路径末段（目录或文件）。末尾带分隔符时返回去掉分隔符后的那一段。 */
export function baseName(p: string | undefined): string {
    const s = String(p ?? '').replace(/[\\/]+$/, '')
    const seg = s.split(/[\\/]/)
    return seg[seg.length - 1] || s
}

/** 路径去掉末段 —— `H:\音乐\mv.mp4` → `H:\音乐`；没有上一级就回空串。 */
export function dirName(p: string | undefined): string {
    const s = String(p ?? '').replace(/[\\/]+$/, '')
    const i = Math.max(s.lastIndexOf('\\'), s.lastIndexOf('/'))
    return i > 0 ? s.slice(0, i) : ''
}

/** 小写扩展名（不含点）；没有就回空串。 */
export function extOf(p: string | undefined): string {
    const m = /\.([^.\\/]+)$/.exec(String(p ?? ''))
    return m ? m[1].toLowerCase() : ''
}

/** 去掉末段扩展名：`mv.mp4` → `mv`。 */
export function stripExt(name: string): string {
    return name.replace(/\.[^.\\/]+$/, '')
}
