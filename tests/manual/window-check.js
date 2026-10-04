/**
 * 问窗口自己「你是谁」—— CDP 的 `Runtime.evaluate` 包装。
 *
 *   node tests\manual\window-check.js [CDP端口]     默认 9337
 *
 * **这个文件存在的唯一理由**：那些要问应用的表达式**必须住在文件里**。
 * 踩过两次同一个坑：把多行 JS 内联进 PowerShell 的 `node -e "…"`，外层 shell 会把
 * `"` 吃掉，于是 `location.origin` 求值成 `undefined`、报告读起来像「应用没有 origin」
 * —— 而应用完全正常，白查一轮。
 *
 * 所以分工是：**命令只传路径与端口，表达式一律写在文件里。**
 * `cdp-window-check.mjs`（10 页冒烟）用的是同一个连法，那边还带 8 秒超时保护 ——
 * 没有超时的 CDP 调用会永远挂着，Node 只会报一句 `unsettled top-level await`。
 */

const CDP = Number(process.argv[2] ?? 0) || 9337
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** 要问窗口的问题。**写在这里，不要内联到命令行。** */
const QUESTIONS = {
  origin: 'location.origin',
  href: 'location.href',
  ipc: 'typeof window.__TAURI_INTERNALS__ !== "undefined"',
  invoke: 'typeof window.__TAURI_INTERNALS__?.invoke === "function"',
  glass: 'document.querySelectorAll("[data-material]").length',
  textLen: '(document.body.innerText || "").length',
  text: '(document.body.innerText || "").replace(/\\s+/g, " ").slice(0, 160)',
}

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
    // ⚠️ 每一跳都要超时：没有超时的 CDP 调用会永远挂着（见文件头）。
    const timer = setTimeout(() => {
      if (pend.delete(my)) rej(new Error(`CDP 调用超时（8 秒）：${method}`))
    }, 8000)
    // ⚠️ 存的是 `{ resolve, reject, timer }` **对象**，不是函数 ——
    // 消息回调里要按字段取（第一版写成 `pend.get(id)(m)`，直接报
    // `TypeError: pend.get(...) is not a function`）。
    pend.set(my, { resolve: res, reject: rej, timer })
    ws.send(JSON.stringify({ id: my, method, params }))
  })

  const ev = async (expr) => {
    const m = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })
    if (m.exceptionDetails) {
      throw new Error(m.exceptionDetails.exception?.description ?? m.exceptionDetails.text ?? 'eval 失败')
    }
    /* ⚠️ **CDP 的回包是两层 `result`**：`{"id":1,"result":{"result":{"type":"string","value":…}}}`。
       外层是 JSON-RPC 的响应壳，内层才是 RemoteObject。写成 `m.result.value` 会**每一项都拿到
       `undefined`** —— 而报告照样打印得整整齐齐，看着像「应用没有 origin / 页面里没有 IPC」，
       白查一轮（这个坑真踩过：先误判成 PowerShell 吃掉引号，其实两处 JS 一直是好的）。 */
    const remote = m.result?.result
    if (!remote) throw new Error(`Runtime.evaluate 没给 RemoteObject：${JSON.stringify(m).slice(0, 200)}`)
    if (remote.type === 'undefined') return undefined
    return remote.value
  }
  return { ev, close: () => { try { ws.close() } catch { /* 无所谓 */ } } }
}

const cdp = await connect()
if (!cdp) {
  console.log(`连不上窗口的 CDP（端口 ${CDP}）—— 应用没在跑，或者没带 --remote-debugging-port 启动。`)
  process.exit(1)
}

try {
  const out = {}
  for (const [k, expr] of Object.entries(QUESTIONS)) out[k] = await cdp.ev(expr)

  console.log('窗口：')
  console.log(`  origin               ${out.origin}`)
  console.log(`  href                 ${out.href}`)
  console.log(`  __TAURI_INTERNALS__  ${out.ipc ? '在' : '不在'}${out.invoke ? '（invoke 可用）' : ''}`)
  console.log(`  已渲染               ${out.glass} 个玻璃面，正文 ${out.textLen} 字`)
  console.log(`  正文开头             ${JSON.stringify(out.text)}`)

  /* 期望值：新架构下窗口加载的是 Tauri 的资源协议（Windows 上是 `http://tauri.localhost`），
     而 IPC 必须在。这两条不对就说明窗口没走对加载方式。 */
  if (!out.ipc) {
    console.log('\n⚠️ 页面里没有 Tauri IPC —— 窗口可能还在加载 http://127.0.0.1（旧架构的地址）。')
  }
  if (out.origin && !/tauri\.localhost|^tauri:|localhost:1420/.test(out.origin)) {
    console.log(`\n⚠️ origin 是 ${out.origin}，不是 Tauri 的资源协议 —— 检查 tauri.conf.json 的 frontendDist。`)
  }
} finally {
  cdp.close()
}
