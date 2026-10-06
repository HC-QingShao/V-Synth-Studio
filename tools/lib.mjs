/**
 * `tools/` 里几个脚本共用的底座：路径、删除、遍历、下载、代理、哈希。
 *
 * 这不是「工具库」，是**几件具体的事只有一处实现**：下载的截断/限速判据、
 * 代理的选取、`<root>` 的定位 —— 抄第二份就会出现「一个脚本跟上了、另一个没有」。
 *
 * 跑在哪儿：仓库根由本文件位置推出来，不依赖当前工作目录。
 */

import {spawnSync} from 'node:child_process'
import {once} from 'node:events'
import {createHash} from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import {fileURLToPath} from 'node:url'

export const MB = 1024 * 1024
export const HERE = path.dirname(path.dirname(fileURLToPath(import.meta.url)))

export function say(msg = '') {
  // flush：CI 上要能实时看到进度
  process.stdout.write(msg + '\n')
}

/**
 * 这个模块是不是正在被当脚本直接跑。
 *
 * 传入**调用方自己的** `import.meta.url`：写成 `if (isMain(import.meta.url))`。
 * 传别处的 URL（比如 lib.mjs 自己的）会永不相等，脚本静默变成空操作。
 *
 * ⚠️ **别写 `import.meta.url === `file://${process.argv[1]}`** —— Windows 上
 * `process.argv[1]` 是 `C:\...`，拼出来是 `file://C:\...`，与 `import.meta.url`
 * 的 `file:///C:/...` 永不相等 → 脚本静默变成空操作（不报错，什么都不干）。
 */
export function isMain(metaUrl) {
  return process.argv[1] ? fileURLToPath(metaUrl) === path.resolve(process.argv[1]) : false
}

export const exists = (p) => fs.existsSync(p)
export const isFile = (p) => fs.existsSync(p) && fs.statSync(p).isFile()
export const isDir = (p) => fs.existsSync(p) && fs.statSync(p).isDirectory()
export const size = (p) => fs.statSync(p).size
export const mkdirp = (p) => fs.mkdirSync(p, {recursive: true})
export const rmrf = (p) => fs.rmSync(p, {recursive: true, force: true})
export const rmdirIf = (p) => { if (fs.existsSync(p)) fs.rmSync(p, {recursive: true, force: true}) }
export const copyFile = (src, dst) => { mkdirp(path.dirname(dst)); fs.copyFileSync(src, dst) }
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** 目录里所有文件的路径（递归）。目录不存在时回空数组。 */
export function walkFiles(dir) {
  const out = []
  const stack = [dir]
  while (stack.length) {
    const d = stack.pop()
    let ents
    try { ents = fs.readdirSync(d, {withFileTypes: true}) } catch { continue }
    for (const e of ents) {
      const p = path.join(d, e.name)
      if (e.isDirectory()) stack.push(p)
      else if (e.isFile()) out.push(p)
    }
  }
  return out
}

export function countFiles(dir) {
  if (!isDir(dir)) return 0
  return walkFiles(dir).length
}

export function dirSize(dir) {
  let n = 0, bytes = 0
  for (const f of walkFiles(dir)) { n++; bytes += fs.statSync(f).size }
  return {n, bytes}
}

export function sha256File(file) {
  const h = createHash('sha256')
  const fd = fs.openSync(file, 'r')
  try {
    const buf = Buffer.alloc(1 << 20)
    for (;;) {
      const k = fs.readSync(fd, buf, 0, buf.length, null)
      if (!k) break
      h.update(buf.subarray(0, k))
    }
  } finally { fs.closeSync(fd) }
  return h.digest('hex').toUpperCase()
}

export const human = (n) => `${(n / MB).toFixed(1)} MB`

/** 读一个环境变量；没设/为空回 `null`（好区分「没设」与「设成空」）。 */
export function env(key) {
  const v = process.env[key]
  return v === undefined ? null : v
}

// ── 代理 ────────────────────────────────────────────────────────────
// 只用 Google 域名那一步要代理（境内 `fonts.googleapis.com` 基本不动）。
//
// ⚠️ Node 的内建代理（`NODE_USE_ENV_PROXY`）**只在进程启动时读环境变量**：
//    起来之后再改 `process.env` 不生效。所以检测到代理时要**用代理环境变量重跑
//    一次自己**（见 `ensureFetchProxy`），不重跑就等于没代理。

/** 定用哪个代理。`VSYNTH_FONT_PROXY`（可设成空 = 强制直连）→ 本机 7890 有监听 → 都没有。 */
export function detectProxy() {
  const explicit = env('VSYNTH_FONT_PROXY')
  if (explicit !== null) return explicit.trim()
  return probeProxySync()
}

/** 同步探一次 127.0.0.1:7890。（Node 的 `net.connect` 是异步的，借 `spawnSync` 起一个子进程同步判。） */
function probeProxySync() {
  const code = 'const n=require("net");const s=n.connect(7890,"127.0.0.1");' +
    's.on("connect",()=>{s.destroy();process.exit(0)});s.on("error",()=>process.exit(1));' +
    'setTimeout(()=>process.exit(1),500)'
  const r = spawnSync(process.execPath, ['-e', code], {stdio: 'ignore', timeout: 1500})
  return r.status === 0 ? 'http://127.0.0.1:7890' : ''
}

/**
 * 确保「被 fetch 采信的」代理环境变量已就位；没有就什么都不做。
 * 检测到代理但环境里没有 → **重跑一次自己**（Node 只在启动时读这些变量）。
 */
export function ensureFetchProxy() {
  if (env('__VSS_PROXY_SET')) return          // 已经重跑过，别再套娃
  const proxy = detectProxy()
  if (!proxy) return
  const kid = spawnSync(process.execPath, process.argv.slice(1), {
    stdio: 'inherit',
    env: {...process.env, ...proxyEnv(proxy), __VSS_PROXY_SET: '1'},
  })
  process.exit(kid.status ?? 1)
}

/** 一个代理 URL → 一组环境变量（大小写都给，Node/npm 各认一种）。 */
export function proxyEnv(proxy) {
  return {
    NODE_USE_ENV_PROXY: '1',
    HTTP_PROXY: proxy, HTTPS_PROXY: proxy,
    http_proxy: proxy, https_proxy: proxy,
  }
}

// ── 下载（带进度、截断检测、限速）────────────────────────────────────
/**
 * 速度低到不像在传数据 —— 断开重来。**不重试**：再试一次还是那个速度。
 */
export class SlowTransfer extends Error {}

function delay(ms) { return new Promise((r) => setTimeout(r, ms)) }

/**
 * 一个地址下到 `out`，先写 `out.part`、成功才改名 —— 断了不会留下看着完整的文件。
 *
 * ⚠️ 用 `fetch` 而不是 `https.get`：**只有 `fetch` 认 `NODE_USE_ENV_PROXY`**，
 *    所以走代理时它才是通的（与 Python 的 urllib 行为一致）。重定向也由它自己跟。
 *
 * ⚠️ 三件事都**不是**可选的：
 *   · 回 `text/html` 直接判失败 —— 有些发布点对不存在的路径回 200 + 一张错误页；
 *   · 核对 `Content-Length` —— 连接断开时流只是 `done`，和「读完了」长得一样；
 *   · 卡住检测 —— 连接超时/中途停顿时 abort，不能无限等。
 */
async function downloadOnce(url, out, {connectTimeout = 20, speedLimit = 2048, speedTime = 30} = {}) {
  const part = out + '.part'
  const ac = new AbortController()
  const stallMs = Math.max(connectTimeout, 30) * 1000
  let stall = setTimeout(() => ac.abort(new Error('连接超时')), connectTimeout * 1000)
  const touch = () => { clearTimeout(stall); stall = setTimeout(() => ac.abort(new Error('连接卡住（无数据）')), stallMs) }

  let res
  try {
    res = await fetch(url, {headers: {'User-Agent': 'v-synth-studio-fetch'}, signal: ac.signal})
  } catch (e) {
    clearTimeout(stall)
    throw e
  }
  if (!res.ok) { clearTimeout(stall); throw new Error(`HTTP ${res.status}`) }
  const ctype = String(res.headers.get('content-type') || '').toLowerCase()
  if (ctype.includes('text/html')) {
    clearTimeout(stall)
    throw new Error(`回的是 HTML（${ctype}），不是产物 —— 这个地址大概失效了`)
  }
  const total = parseInt(res.headers.get('content-length') || '0', 10) || 0

  const ws = fs.createWriteStream(part)
  const reader = res.body.getReader()
  let got = 0, winBytes = 0, winStart = Date.now(), lastTick = 0
  try {
    for (;;) {
      const {done, value} = await reader.read()
      if (done) break
      touch()
      got += value.length; winBytes += value.length
      const now = Date.now()
      if ((now - winStart) / 1000 >= speedTime) {
        const rate = winBytes / ((now - winStart) / 1000)
        if (rate < speedLimit) throw new SlowTransfer(`速度只有 ${(rate / 1024).toFixed(1)} KB/s，判定为卡死`)
        winStart = now; winBytes = 0
      }
      if (total && (now - lastTick) / 1000 >= 5) {
        lastTick = now
        say(`    ${(got * 100 / total).toFixed(1)}%  ${(got / MB).toFixed(1)}/${(total / MB).toFixed(1)} MB`)
      }
      if (!ws.write(Buffer.from(value))) await once(ws, 'drain')
    }
  } finally {
    clearTimeout(stall)
    try { await reader.cancel() } catch { /* 已经读完 */ }
  }
  await new Promise((resolve, reject) => { ws.end(resolve); ws.on('error', reject) })

  // ⚠️ **必须核对 Content-Length。** 读到流结束和「连接被截断」长得一模一样。
  if (got === 0) throw new Error('下到的文件是空的')
  if (total && got !== total) {
    throw new Error(`下到 ${(got / MB).toFixed(1)} MB，但服务器说 ${(total / MB).toFixed(1)} MB（被截断了）`)
  }
  fs.renameSync(part, out)
}

/**
 * 同一个地址最多试 `tries` 次。全失败抛 `Error`。
 *
 * ⚠️ 两种失败**不重试**：一个字节都没下到（地址不通）、`SlowTransfer`（再试还是那个速度）。
 */
export async function download(url, out, {tries = 3, connectTimeout = 20, speedLimit = 2048, speedTime = 30} = {}) {
  say(`  下载 ${url}`)
  const part = out + '.part'
  let last = ''
  for (let attempt = 1; attempt <= tries; attempt++) {
    try {
      await downloadOnce(url, out, {connectTimeout, speedLimit, speedTime})
      return
    } catch (e) {
      last = `${e.constructor.name}: ${e.message}`
      if (e instanceof SlowTransfer) { say(`  ${e.message}`); break }
      if (!isFile(part) || size(part) === 0) { say(`  连不上（${last}）`); break }
      say(`  第 ${attempt} 次失败（${last}），重试…`)
      await sleep(3000)
    }
  }
  fs.rmSync(part, {force: true})
  throw new Error(`下载失败：${url}\n  最后一次：${last}`)
}

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'

/**
 * 取一个 URL 进内存（给字体 / CSS / 上游小文件用）。
 *
 * ⚠️ 用 `AbortSignal.timeout` 是**总时长**上限，与 Python 的 socket 超时语义不同 ——
 *    这里都是几十 KB 的小文件，总时长就够用。大文件走 `download`（它按 socket 判超时）。
 * 网络失败一律归一化成 `Error`，调用方只捕一种。
 */
export async function fetchBuffer(url, {timeoutMs = 90_000, ua = UA} = {}) {
  const res = await fetch(url, {headers: {'User-Agent': ua}, signal: AbortSignal.timeout(timeoutMs)})
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  const ctype = String(res.headers.get('content-type') || '').toLowerCase()
  const buffer = Buffer.from(await res.arrayBuffer())
  return {status: res.status, contentType: ctype, buffer}
}
