/**
 * 第 1 步验收：起真窗口，确认「Tauri 资源协议 + 无 HTTP 服务」这一版能跑。
 *
 *   node tests\manual\cdp-window-check.mjs [CDP端口]     默认 9335
 *
 * 六条判据：
 *   1. 窗口连得上 CDP（WebView2 认 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`）
 *   2. 17878 上没有任何东西在听（HTTP 服务真的没了）
 *   3. 页面 origin 是 Tauri 的资源协议，`window.__TAURI_INTERNALS__` 在（IPC 通了）
 *   4. `convertFileSrc` 能拿到 @tauri-apps/api（前端能 import 到它）
 *   5. 9 个页面都能切、每页都有玻璃面（渲染没坏）
 *   6. 控制台没有**非法调用**类报错
 *      （第 1 步只砍到「窗口自己加载前端」，主页那 4 条 /api/ 请求会打到资源协议上、
 *        必然 404 —— 那是第 3 步才换的，所以这里只把与它们无关的错算红）
 *
 * 进程卫生：起窗口用 Start-Process，结束时只按**精确 PID** 杀（绝不按名字/命令行子串）。
 */

import { spawn, execFileSync } from 'node:child_process'
import { mkdirSync, rmSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const CDP_PORT = Number(process.argv[2] ?? 0) || 9335
/**
 * 要验的 exe。
 *
 * ⚠️ 2026-10-05：原来这里写死 `C:\Users\qingm\Downloads\工作站\v-synth-studio.exe`，
 * CI 上必然找不到（那边是 `app/desktop/target/release/` 下的 release 版）。
 * 现在：`VS_EXE` 环境变量优先（CI 用它指 release 产物），否则按**脚本位置**推出仓库根
 * ——本机默认还是根目录那个 exe（`build.ps1` 会把 dev 产物复制到那儿）。
 */
const EXE = process.env.VS_EXE
  ? resolve(process.env.VS_EXE)
  : resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'v-synth-studio.exe')
const PROFILE = `${process.env.TEMP}\\vsynth-ipc-check-${CDP_PORT}`
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** 控制台里与「第 3 步还没做」有关的噪音，先不算红 */
const EXPECTED = [
  '/api/',          // 旧前端还在 fetch 那 4 条接口（第 3 步换成 invoke）
  'Failed to load resource',
  'Unexpected token',       // 打到资源协议上返回的是 index.html，JSON.parse 会炸
  'not valid JSON',
  'NetworkError',
]

function startApp() {
  rmSync(PROFILE, { recursive: true, force: true })
  mkdirSync(PROFILE, { recursive: true })
  const child = spawn(EXE, [], {
    stdio: 'ignore',
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: PROFILE,
    },
  })
  return child
}

function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl)
    let id = 0
    const pending = new Map()
    const consoleErrors = []
    ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(ev.data)
      if (msg.method === 'Runtime.consoleAPICalled' && msg.params.type === 'error') {
        consoleErrors.push(msg.params.args.map((a) => a.value ?? a.description ?? '').join(' ').slice(0, 200))
      }
      if (msg.method === 'Runtime.exceptionThrown') {
        const d = msg.params.exceptionDetails
        consoleErrors.push((d.exception?.description ?? d.text ?? '异常').slice(0, 200))
      }
      if (msg.id && pending.has(msg.id)) {
        const { resolve: res, reject: rej } = pending.get(msg.id)
        pending.delete(msg.id)
        if (msg.error) rej(new Error(JSON.stringify(msg.error)))
        else res(msg.result)
      }
    })
    ws.addEventListener('error', (e) => reject(new Error(`WebSocket 出错：${e.message ?? e.type}`)))
    ws.addEventListener('open', () => {
      /* ⚠️ 每一跳都要有超时。没有超时的 CDP 调用会**永远挂着**（踩过：窗口其实
         加载失败成了 `chrome-error://` 页面，`Runtime.evaluate` 就再也没有回音，
         Node 报 unsettled top-level await 后直接退出，看不出任何原因）。 */
      const send = (method, params = {}) => new Promise((res, rej) => {
        const myId = ++id
        const timer = setTimeout(() => {
          if (pending.delete(myId)) rej(new Error(`CDP 调用超时（8 秒）：${method}`))
        }, 8000)
        pending.set(myId, {
          resolve: (v) => { clearTimeout(timer); res(v) },
          reject: (e) => { clearTimeout(timer); rej(e) },
        })
        ws.send(JSON.stringify({ id: myId, method, params }))
      })
      const evalJs = async (expr) => {
        const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })
        if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text)
        return r.result.value
      }
      resolve({ send, evalJs, consoleErrors, clearErrors: () => { consoleErrors.length = 0 }, close: () => ws.close() })
    })
  })
}

const PAGES = [
  ['dashboard', '总览'], ['convert', '工程转换'], ['video', '视频解析'], ['svsep', '音轨分离'],
  ['midi', '人声转 MIDI'], ['audio', '音频工具'], ['lyrics', '网易云专栏'], ['pv', '文字 PV'],
  ['resources', '资源库'], ['settings', '设置'],
]

const app = startApp()
let cdp = null
const results = []
let listenerCheck = ''
/**
 * ⚠️ 2026-10-05：这个脚本以前**失败也退出 0**（catch 里只打印一行），
 * 所以它能当人工看的报告、却当不了 CI 门禁 —— 步骤永远绿。
 * 现在有任何失败就 exit 1。
 */
let exitCode = 0

try {
  let targets = null
  for (let i = 0; i < 60; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)
      const list = await res.json()
      targets = Array.isArray(list) ? list : null
      if (targets?.some((t) => t.type === 'page')) break
    } catch { /* 还没起来 */ }
    await sleep(500)
  }
  if (!targets?.length) throw new Error(`连不上 WebView2 的 CDP 端口 ${CDP_PORT}（窗口没起来？）`)

  const page = targets.find((t) => t.type === 'page' && t.webSocketDebuggerUrl)
  cdp = await connect(page.webSocketDebuggerUrl)
  await cdp.send('Runtime.enable')
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1440, height: 900, deviceScaleFactor: 1, mobile: false,
  })

  /* ① 等应用就绪（遮罩揭开 + 有玻璃面 + 正文非空） */
  let ready = false
  for (let i = 0; i < 60; i++) {
    ready = await cdp.evalJs(`!document.getElementById('boot') &&
      document.querySelectorAll('[data-material]').length > 0 &&
      (document.body.innerText || '').length > 200`)
    if (ready) break
    await sleep(400)
  }

  const env = await cdp.evalJs(`({
    origin: location.origin,
    href: location.href,
    hasIpc: !!window.__TAURI_INTERNALS__,
    hasApiPkg: typeof window.__TAURI_INTERNALS__?.invoke === 'function',
    glass: document.querySelectorAll('[data-material]').length,
    text: (document.body.innerText || '').slice(0, 120),
  })`)

  /* ② 端口检查：HTTP 服务应该已经不存在了 */
  try {
    const out = execFileSync('powershell', ['-NoProfile', '-Command',
      `(Get-NetTCPConnection -LocalPort 17878 -State Listen -EA SilentlyContinue | Measure-Object).Count`],
      { encoding: 'utf8' }).trim()
    listenerCheck = out === '0' ? '✓ 17878 无监听' : `✗ 17878 上还有 ${out} 个监听`
  } catch (e) {
    listenerCheck = `✗ 端口检查跑不起来：${e.message}`
  }

  /* ③ 逐页切 */
  for (const [id, name] of PAGES) {
    cdp.clearErrors()
    await cdp.evalJs(`(() => { location.hash = '#/${id}'; return true })()`)
    await sleep(700)
    const st = await cdp.evalJs(`({
      text: (document.body.innerText || '').length,
      glass: document.querySelectorAll('[data-material]').length,
      title: document.querySelector('.page-title')?.textContent ?? '',
    })`)
    const errs = cdp.consoleErrors.filter((e) => !EXPECTED.some((x) => e.includes(x)))
    results.push({ id, name, ...st, errs })
  }

  console.log(`\n应用就绪：${ready ? '是' : '否（18 秒内没就绪）'}`)
  console.log(`页面 origin  ：${env.origin}`)
  console.log(`__TAURI_INTERNALS__：${env.hasIpc ? '在 ✓' : '不在 ✗'}（invoke：${env.hasApiPkg ? '可用' : '不可用'}）`)
  console.log(listenerCheck)
  console.log(`首屏文本：${JSON.stringify(env.text)}\n`)
  console.log('页面'.padEnd(16, '　') + '玻璃面  文本长度  非预期报错')
  for (const r of results) {
    const bad = r.errs.length ? `✗ ${r.errs.length} 条：${r.errs[0]}` : '无'
    console.log(`${r.name.padEnd(14, '　')} ${String(r.glass).padStart(4)}  ${String(r.text).padStart(7)}   ${bad}`)
  }
  const failed = results.filter((r) => r.errs.length || r.glass === 0 || r.text < 50)
  console.log(`\n═══ 通过 ${results.length - failed.length} / 失败 ${failed.length} ═══`)
  if (failed.length) exitCode = 1
} catch (e) {
  console.log(`\n✗ 验收脚本失败：${e.message}`)
  exitCode = 1
} finally {
  try { cdp?.close() } catch { /* 无所谓 */ }
  try { app.kill() } catch { /* 无所谓 */ }
  await sleep(500)
  /* 只按精确 PID 收尾；WebView2 的子进程会跟着宿主一起走 */
  try { execFileSync('taskkill', ['/PID', String(app.pid), '/T', '/F'], { stdio: 'ignore' }) } catch { /* 已经没了 */ }
  rmSync(PROFILE, { recursive: true, force: true })
}
process.exit(exitCode)
