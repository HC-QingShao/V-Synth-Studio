import {useCallback, useEffect, useRef, useState} from 'react'
import {api} from './api'
import {watchJob} from './ipc'
import type {Job} from './types'

/**
 * 订阅一个后端任务的进度。
 *
 * 契约：优先 `job_watch`（一条长驻 IPC 命令 + 一个 `Channel`，Rust 每有
 * 变化就推一份**完整快照**），连接断了自动退回 700ms 轮询；`done | error | canceled`
 * 三个状态是终态，到终态就收订阅、只回调一次。
 *
 * ⚠️ **那条 700ms 轮询兜底不要删。** 它是断线保险，与「IPC 会不会失败」无关：
 * 通道建不起来（页面切走、后端提前 drop）、或者推流中途断了，都由它接管。
 *
 * ⚠️ **页面卸载时要停订阅**：钩子里在 `useEffect` 的清理函数里做了，
 * 但**手动 `start()` 第二个任务前也要 `stop()`** —— 否则两个订阅同时刷同一份 state。
 */
interface JobHandlers {
    onUpdate?: (job: Job) => void
    onDone?: (job: Job) => void
    onError?: (err: Error, job: Job) => void
    onCancel?: (job: Job) => void
}

export function useJob() {
    const [job, setJob] = useState<Job | null>(null)
    const handlersRef = useRef<JobHandlers>({})
    const stopRef = useRef<(() => void) | null>(null)

    const stop = useCallback(() => {
        stopRef.current?.()
        stopRef.current = null
    }, [])

    const start = useCallback(
        (jobId: string, handlers: JobHandlers = {}) => {
            stopRef.current?.()
            handlersRef.current = handlers
            setJob(null)

            let pollTimer: number | null = null
            let stopped = false
            let sawTerminal = false

            const finish = (j: Job) => {
                if (sawTerminal || stopped) return
                sawTerminal = true
                if (j.status === 'error') handlersRef.current.onError?.(new Error(j.error ?? '任务失败'), j)
                else if (j.status === 'canceled') handlersRef.current.onCancel?.(j)
                else handlersRef.current.onDone?.(j)
            }

            const handle = (j: Job | undefined | null) => {
                if (stopped || !j) return
                setJob(j)
                handlersRef.current.onUpdate?.(j)
                if (j.status === 'done' || j.status === 'error' || j.status === 'canceled') {
                    finish(j)
                    stopLocal()
                }
            }

            const startPolling = () => {
                if (pollTimer !== null || stopped) return
                pollTimer = window.setInterval(async () => {
                    try {
                        const {job: j} = await api.job(jobId)
                        handle(j)
                    } catch {
                        /* 瞬时错误忽略，下一拍再试 */
                    }
                }, 700)
            }

            const stopLocal = () => {
                stopped = true
                if (pollTimer !== null) {
                    clearInterval(pollTimer)
                    pollTimer = null
                }
            }
            stopRef.current = stopLocal

            /* `job_watch` 是**长驻**命令：它 await 到任务到终态才返回。所以这条 promise
               在任务跑完之前一直挂着 —— 正是我们要的「推流」。它抛错（通道建不起来、
               任务表锁坏了）就退回轮询。 */
            watchJob(jobId, (j) => handle(j as Job)).catch(() => {
                if (!stopped) startPolling()
            })

            return stopLocal
        },
        [],
    )

    /* 离开页面时收干净 —— 挂着订阅会让后端一直广播给一个没人看的页面 */
    useEffect(() => () => stopRef.current?.(), [])

    return {job, start, stop}
}
