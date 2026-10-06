import {useEffect} from 'react'

/**
 * 后端状态的轮询节奏 —— 音轨分离、扒谱两页共用一套。
 *
 * ⚠️ 空闲时必须降速：这些状态本身不会自己变（除非服务在跑、或有下载/后台删除），
 * 而每一轮 `setState` 都带着整页重画。有事 2 秒一追，没事 15 秒一问。
 */
export const POLL_STATUS = 2000
export const POLL_IDLE = 15000

/**
 * 按 `fast` 决定节奏地轮询 `refresh`；`paused` 为真时完全不问
 * （安装循环自己每 1.75 秒拉一次，再叠一层就是双倍请求）。
 */
export function useStatusPoll(
    refresh: () => Promise<unknown>,
    fast: boolean,
    paused = false,
): void {
    useEffect(() => {
        if (paused) return
        void refresh()
        const t = window.setInterval(() => void refresh(), fast ? POLL_STATUS : POLL_IDLE)
        return () => window.clearInterval(t)
    }, [refresh, fast, paused])
}
