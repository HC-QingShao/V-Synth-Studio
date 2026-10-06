/**
 * 音轨分离「提交」探针 —— 直接问 IPC：`svsep_separate` 到底通不通。
 *
 *   node tests\manual\svsep-separate-check.mjs 9349 "D:\某个.wav" [engine]
 *
 * ## 为什么要单有一个探针
 *
 * 2026-10 试过线上反馈：「音轨分离点了报 400」。根因是 Rust 把音频**裸字节**直接 POST
 * 给 Python 服务，而上游 Flask 要的是 `multipart/form-data` 里那个 `file` 字段 ——
 * 它读不到 `request.files["file"]` 就回 `{"ok": false, "error": "未检测到上传文件"}` + **400**。
 * 这类错**渲染层看不出来**（页面照样绿），只有真提交一次才现形，所以单独留一个探针。
 *
 * 判据（三档）：
 *   - `task_id` + `ok:true` ⇒ 通（HTTP 200 且上游认了这次上传）
 *   - `HTTP 400`          ⇒ 正是上面那个 bug
 *   - 其它                ⇒ 原样打出来，别猜
 *
 * ⚠️ 探针会**真的提交一次分离**（服务会被起起来读模型）。跑完自己调 `svsep_stop` 收尾，
 *    在任务真跑起来之前就把它取消掉 —— 它只验「提交这一跳」，不验分离质量。
 * ⚠️ CDP 的 `Runtime.evaluate` 回包是两层 `result`（见 `window-check.js` 的注释）。
 */

import { writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const CDP = Number(process.argv[2] ?? 0) || 9349
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** 没给音频就给一个 2 秒 44.1k 单声道正弦 WAV（省得每次都去找素材） */
function makeWav() {
  const rate = 44100
  const secs = 2
  const n = rate * secs
  const data = Buffer.alloc(n * 2)
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * 440 * i) / rate) * 12000), i * 2)
  }
  const head = Buffer.alloc(44)
  head.write('RIFF', 0)
  head.writeUInt32LE(36 + data.length, 4)
  head.write('WAVE', 8)
  head.write('fmt ', 12)
  head.writeUInt32LE(16, 16)
  head.writeUInt16LE(1, 20)
  head.writeUInt16LE(1, 22)
  head.writeUInt32LE(rate, 24)
  head.writeUInt32LE(rate * 2, 28)
  head.writeUInt16LE(2, 32)
  head.writeUInt16LE(16, 34)
  head.write('data', 36)
  head.writeUInt32LE(data.length, 40)
  const p = join(tmpdir(), 'svsep-probe.wav')
  writeFileSync(p, Buffer.concat([head, data]))
  return p
}

const audio = process.argv[3] ? String(process.argv[3]) : makeWav()
const engine = process.argv[4] ? String(process.argv[4]) : 'uvr'
/** 第 5 个参数 `wait` 才真等任务跑完（默认只验「提交这一跳」就取消，省几分钟 CPU） */
const WAIT = String(process.argv[5] ?? '') === 'wait'

async function connect() {
  let page = null
  for (let i = 0; i < 30; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${CDP}/json/list`)).json()
      page = Array.isArray(list) ? list.find((x) => x.type === 'page') : null
      if (page) break
    } catch { /* 还没起来 */ }
    await sleep(400)
  }
  if (!page) return null
  const ws = new WebSocket(page.webSocketDebuggerUrl)
  let id = 0
  const pend = new Map()
  ws.addEventListener('message', (ev) => {
    const m = JSON.parse(ev.data)
    const slot = m.id ? pend.get(m.id) : undefined
    if (!slot) return
    pend.delete(m.id)
    clearTimeout(slot.timer)
    if (m.error) slot.reject(new Error(JSON.stringify(m.error)))
    else slot.resolve(m)
  })
  await new Promise((r, j) => {
    ws.addEventListener('open', r)
    ws.addEventListener('error', () => j(new Error('WebSocket 连不上')))
  })
  const send = (method, params = {}) => new Promise((res, rej) => {
    const my = ++id
    // 提交这一跳要读文件 + （必要时）起 Python 服务，给足 5 分钟
    const timer = setTimeout(() => { if (pend.delete(my)) rej(new Error(`CDP 调用超时：${method}`)) }, 300000)
    pend.set(my, { resolve: res, reject: rej, timer })
    ws.send(JSON.stringify({ id: my, method, params }))
  })
  const ev = async (expr) => {
    const m = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })
    if (m.exceptionDetails) {
      throw new Error(m.exceptionDetails.exception?.description ?? m.exceptionDetails.text ?? 'eval 失败')
    }
    const remote = m.result?.result
    if (!remote) throw new Error(`Runtime.evaluate 没给 RemoteObject：${JSON.stringify(m).slice(0, 200)}`)
    if (remote.type === 'undefined') return undefined
    return remote.value
  }
  /* ⚠️ **页面里没有裸 `invoke`**（`window.__TAURI_INTERNALS__.invoke` 才是真的那一个）。
     踩过：写成 `invoke(...)` 时 typeof 是 undefined，而 CDP 对「被拒的 promise + awaitPromise」
     回的是一坨空对象，于是探针照样打印「提交成功」——**假绿灯**。所以统一从这里拼表达式。 */
  const ipc = (body) =>
    `(async () => { const invoke = window.__TAURI_INTERNALS__.invoke; ${body} })()`
  return { ev, ipc, close: () => { try { ws.close() } catch { /* 无所谓 */ } } }
}

const cdp = await connect()
if (!cdp) {
  console.log(`连不上窗口的 CDP（端口 ${CDP}）—— 应用没在跑，或者没带 --remote-debugging-port 启动。`)
  process.exit(1)
}

let failed = 0
try {
  const st = await cdp.ev(cdp.ipc('const s = await invoke("svsep_status"); return s'))
  console.log('状态：', JSON.stringify({ runtimeReady: st?.runtimeReady, modelsOk: !!st?.models?.ok, running: st?.running }))
  console.log(`提交：${audio}（engine=${engine}）`)

  let res = null
  let err = null
  try {
    res = await cdp.ev(cdp.ipc(`const r = await invoke("svsep_separate", ${JSON.stringify({ path: audio, engine })}); return r`))
  } catch (e) {
    err = e instanceof Error ? e.message : String(e)
  }

  /* ⚠️ Tauri 的 `__TAURI_INTERNALS__.invoke` 对**命令报错是 resolve 成一个字符串**、
     不是 reject（`@tauri-apps/api` 的 `invoke()` 包装层才转成 rejection）。所以这里
     不能只看 `res` 有没有值 —— 那样报错也会被判成成功（踩过，假绿灯）。 */
  const isErr = typeof res === 'string'
  if (res && !isErr) {
    console.log(`  · 回包（原样）：${JSON.stringify(res)?.slice(0, 400)}`)
    const id = res?.task?.id ?? res?.task_id
    if (!id) {
      failed += 1
      console.log('  ✗ 回包里没有 task.id —— 上游收下了但形状不对，去看 svsep.rs 的 flat_task')
    } else {
      console.log(`  ✓ 提交成功：task_id=${id}`)
    }
    if (id) {
      if (WAIT) {
        /* `wait` 模式：真等它跑完（默认 uvr + 2 秒素材，模型加载是大头）。
           验的是「整条链」——提交、排队、出结果、结果落盘。 */
        const t0 = Date.now()
        for (;;) {
          const t = await cdp.ev(cdp.ipc(`const t = await invoke("svsep_task", { id: ${JSON.stringify(String(id))} }); return t`))
          const st = String(t?.status ?? '?')
          const outs = (t?.outputs ?? []).map((o) => o.filename).filter(Boolean)
          console.log(`    [${Math.round((Date.now() - t0) / 1000)}s] ${st} ${t?.progress ?? '?'}% ${outs.length ? outs.join(', ') : ''}`)
          if (st === 'done') {
            if (!outs.length) {
              failed += 1
              console.log('  ✗ 任务说 done 但 outputs 是空的')
            } else {
              console.log(`  ✓ 分离完成，产出 ${outs.length} 个文件：${outs.join(', ')}`)
            }
            break
          }
          if (st === 'failed' || st === 'cancelled') {
            failed += 1
            console.log(`  ✗ 任务收场是 ${st}：${t?.error ?? t?.message ?? ''}`)
            break
          }
          if (Date.now() - t0 > 600000) {
            failed += 1
            console.log('  ✗ 10 分钟还没跑完，放弃等待（默认档本来就慢，别据此报 bug）')
            break
          }
          await sleep(3000)
        }
      } else {
        /* 只验「提交这一跳」：任务真跑起来就取消掉，别让探针占着 CPU 跑几分钟 */
        try {
          await cdp.ev(cdp.ipc(`await invoke("svsep_cancel", { id: ${JSON.stringify(String(id))} }); return true`))
          console.log('  · 已取消这次任务（探针不验分离质量；想跑完加参数 wait）')
        } catch (e) {
          console.log(`  · 取消失败（不影响判据）：${e instanceof Error ? e.message : String(e)}`)
        }
      }
    }
  } else {
    failed += 1
    const msg = String(err ?? res)
    console.log(`  ✗ 提交失败：${msg}`)
    if (/HTTP 400/.test(msg)) {
      console.log('    ⇒ 这就是「未检测到上传文件」那个 400：发给 Python 的不是 multipart/form-data。')
    }
  }

  try {
    await cdp.ev(cdp.ipc('await invoke("svsep_stop"); return true'))
    console.log('  · 已停掉分离服务')
  } catch { /* 服务本来就没起 */ }
} finally {
  cdp.close()
}

console.log(`\n═══ ${failed === 0 ? '通过' : '失败'} ═══`)
process.exit(failed === 0 ? 0 : 1)
