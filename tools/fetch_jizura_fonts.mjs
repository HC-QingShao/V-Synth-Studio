#!/usr/bin/env node
/**
 * 把 JIZURA 文字 PV 需要的东西补齐到 `public/vendor/jizura/`。
 *
 *     node tools/fetch_jizura_fonts.mjs
 *     node tools/fetch_jizura_fonts.mjs --only Dela+Gothic+One   # 只抓一个家族（调试）
 *     node tools/fetch_jizura_fonts.mjs --force                  # 连 CSS 缓存一起重抓
 *
 * 干三件事：
 *   1. 从上游取 `index.html` 与 `LICENSE`（`852wa/JIZURA`，按 assets.mjs 钉的 commit），
 *      并给 `index.html` 打 3 处补丁：把它的 Google Fonts 引用换成自托管的 `fonts.css`
 *   2. 用现代浏览器 UA 取 `fonts.googleapis.com/css2?family=…`（不加 UA 会给 ttf 而非 woff2）
 *   3. 下 CSS 引用的每个 `fonts.gstatic.com` 上的 woff2，URL 改写成相对路径，合并进 `fonts.css`
 *
 * ── 两条硬约束 ─────────────────────────────────────────────────────
 *
 * **字体家族清单从 `index.html` 的 `gf:` 字段抠**，不硬编码 —— 硬编码就等于
 * 「换 JIZURA 版本时记得手工同步」，漏了表现为 PV 页少字形且不报错。
 *
 * **那 3 处补丁是 `JIZURA_PATCHES`，三处都必须恰好命中。** 只改其中一处会让
 * 「首屏字体正常、用户换家族时才去连 Google」这种半吊子状态通过检查，所以改不到就报错退出。
 *
 * ⚠️ 「代理」只作用于 Google Fonts 那一步；上游两个文件走 `github.com`，境内直连可用。
 */

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'

import * as assets from './assets.mjs'
import {HERE, ensureFetchProxy, fetchBuffer, isFile, mkdirp, say, isMain} from './lib.mjs'
import {verify} from './fetch_tools.mjs'

// 从表里取落点，不自己拼 —— 「jizura 在哪儿」只有一处定义。
const JIZURA = assets.byId('jizura')
export const DEST = path.join(HERE, JIZURA.into.replace(assets.ROOT + '/', ''))
const FONT_DIR = path.join(DEST, 'fonts')
// ⚠️ 缓存**不能放在 `DEST` 下**（`public/vendor/jizura/css-raw`）：`public/` 会被 Vite
//    拷进 `dist/`、再被 Tauri 嵌进 exe。放 `src-tauri/target/vss-cache/`（已被 .gitignore 忽略，且在随包范围外）。
export const RAW_DIR = path.join(HERE, 'src-tauri', 'target', 'vss-cache', 'jizura-css-raw')

// ⚠️ 必须用浏览器的 UA：Google 按 UA 决定给什么格式 —— 现代 UA 给 woff2，默认给 ttf。
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'

const MAX_WORKERS = 12
const TIMEOUT_MS = 90_000

// gstatic 的路径里**允许点**（`....cjb4.85.woff2`），所以文件名那段不能贪婪地吃到最后一个点。
// ⚠️ 域名同时收 `.com` 与 `.cn`：走中国域名取 CSS 时，里面的链接是 `fonts.gstatic.cn`。
const URL_RE = /https:\/\/fonts\.gstatic\.(?:com|cn)\/[A-Za-z0-9/._\-]*?[A-Za-z0-9_\-]\.woff2/g
// `gf: 'Dela+Gothic+One'` —— index.html 里字体家族的唯一声明处
const GF_RE = /gf:\s*'([^']+)'/g

// ── Google Fonts 的中国域名回退 ──────────────────────────────────────
// `fonts.googleapis.com` 在境内基本不动（~2 KB/s），而 `.cn` 是 **Google 自己的**中国域名
// （不是第三方镜像）、可到 ~700 KB/s。换域名时 CSS 与字体要一起换。
const CN_HOSTS = {
  'fonts.googleapis.com': 'fonts.googleapis.cn',
  'fonts.gstatic.com': 'fonts.gstatic.cn',
}

/** 一个 URL 按顺序要试的地址：原生 → Google 的 `.cn` 域名。没有就只回它自己。 */
function mirrors(url) {
  for (const [old, next] of Object.entries(CN_HOSTS)) {
    if (url.includes(old)) return [url, url.replace(old, next)]
  }
  return [url]
}

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// ── 上游 index.html 的那 3 处补丁 ────────────────────────────────────
// 每一条是 (正则, 换成什么)，要求恰好匹配 1 次；匹配不到就退出。
const JIZURA_PATCHES = [
  // ① <head> 里的两条 preconnect → 一份指向本地 fonts.css 的样式表
  [
    new RegExp(escapeRe(
      '<link rel="preconnect" href="https://fonts.googleapis.com">\n' +
      '<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>')),
    '<link rel="stylesheet" href="fonts.css">',
  ],
  // ② JS 里那个「首屏要加载哪些家族」的 URL 常量。前缀锚定、后面用 `[^']*` 吃掉。
  [
    new RegExp(escapeRe("J.GOOGLE_FONTS_URL = 'https://fonts.googleapis.com/css2?family=") + "[^']*';"),
    "J.GOOGLE_FONTS_URL = 'fonts.css'; /* 已本地化，见 tools/fetch_jizura_fonts.mjs */",
  ],
  // ③ `attachFamily()` —— 用户换家族时按需插 <link> 的那条
  [
    new RegExp(escapeRe(
      "l.rel = 'stylesheet'; l.href = 'https://fonts.googleapis.com/css2?family=' " +
      "+ spec + '&display=swap';")),
    "l.rel = 'stylesheet'; l.href = 'fonts.css';",
  ],
]

// ── 取数据 ──────────────────────────────────────────────────────────
/**
 * 取一个 URL；原生不通就试境内 `.cn` 域名。全不通时抛 `Error`。
 *
 * ⚠️ 只在**一个字节都没拿到**时才换下一个 —— 拿到了就是成功，不管快慢。
 */
async function fetchTextOrBuffer(url, {binary = false} = {}) {
  const urls = mirrors(url)
  let last = null
  for (let i = 0; i < urls.length; i++) {
    const u = urls[i]
    try {
      const {contentType, buffer} = await fetchBuffer(u, {timeoutMs: TIMEOUT_MS, ua: UA})
      // ⚠️ **只有 woff2 那条路**能按内容类型判「这是不是错误页」：字体不可能是 HTML。
      if (binary && contentType.includes('text/html')) {
        last = new Error(`要的是字体却回了 HTML（${contentType}）—— ${u} 大概失效了`)
        say(`    ${u} 回的是 HTML，换下一个`)
        continue
      }
      return binary ? buffer : buffer.toString('utf8')
    } catch (e) {
      last = e
      if (i === 0 && urls.length > 1) say(`    ${u} 不通（${e.message}），试镜像`)
    }
  }
  throw last || new Error(url)
}

// ── 上游文件（index.html / LICENSE）────────────────────────────────
/** 把 `index.html` 里对 Google Fonts 的引用换成自托管的 `fonts.css`。三处都要改。 */
function patchIndex(html) {
  JIZURA_PATCHES.forEach(([pattern, replacement], i) => {
    const matches = html.match(new RegExp(pattern.source, pattern.flags + 'g')) || []
    if (matches.length !== 1) {
      throw new Error(
        `index.html 补丁 第 ${i + 1} 处匹配到 ${matches.length} 次（应为 1 次）。\n` +
        `  上游改了这段代码，而补丁是按原文匹配的 —— 看一眼 commit\n` +
        `  ${assets.UPSTREAM_JIZURA_SHA.slice(0, 12)} 里现在是怎么加载字体的，\n` +
        `  然后改 fetch_jizura_fonts.mjs::JIZURA_PATCHES。\n` +
        `  别手工编辑 public/vendor/jizura/index.html —— 它是生成物。`
      )
    }
    html = html.replace(pattern, replacement)
  })

  const left = (html.match(/fonts\.googleapis/g) || []).length
  if (left) {
    throw new Error(
      `打完补丁还剩 ${left} 处 fonts.googleapis —— 上游新增了字体加载点，` +
      `补丁只覆盖了 ${JIZURA_PATCHES.length} 处，要一起补上`
    )
  }
  if ((html.match(/fonts\.css/g) || []).length < JIZURA_PATCHES.length) {
    throw new Error('补丁没打进去（fonts.css 的引用数不对）')
  }
  return html
}

/** 按表里钉的 commit 取上游的 index.html 与 LICENSE。返回失败列表。 */
async function fetchUpstreamFiles(force) {
  const bad = []
  for (const rel of assets.UPSTREAM_JIZURA_FILES) {
    const url = assets.UPSTREAM_JIZURA_URL + rel
    const out = path.join(DEST, rel.split('/').pop())   // zh-hans/index.html → index.html
    if (isFile(out) && !force) {
      say(`  已有 ${path.basename(out)}（--force 可重取）`)
      continue
    }
    say(`  取 ${url}`)
    let data
    try {
      data = await fetchTextOrBuffer(url, {binary: false})
    } catch (e) {
      bad.push(`${rel}: ${e.message}`)
      continue
    }
    if (!data.trim()) { bad.push(`${rel}: 下到的是空的`); continue }
    // 取不到时 GitHub 会给一个 404 页面，而 HTTP 状态可能是 200 —— 所以内容也要看。
    const head = data.replace(/^\s+/, '').slice(0, 200).toLowerCase()
    if (rel.endsWith('index.html') && !head.startsWith('<!doctype')) {
      bad.push(`${rel}: 内容不像 HTML（开头是 ${JSON.stringify(data.slice(0, 60))}）`)
      continue
    }
    if (rel.endsWith('LICENSE') && !head.includes('mit license')) {
      bad.push(`${rel}: 内容不像 MIT 许可证（开头是 ${JSON.stringify(data.slice(0, 60))}）`)
      continue
    }
    if (rel.endsWith('index.html')) data = patchIndex(data)
    fs.writeFileSync(out, data, 'utf8')
    say(`    ${path.basename(out)}  ${Math.round(Buffer.byteLength(data) / 1024)} KB`)
  }
  return bad
}

/**
 * 从 JIZURA 的 index.html 里抠 `gf:` 家族清单。
 * ⚠️ 别改回硬编码清单：漏同步的后果是「PV 页少字体」——**不报错**，只少几个字形。
 */
function specsFromIndex() {
  const index = path.join(DEST, 'index.html')
  if (!isFile(index)) {
    throw new Error(
      `找不到 ${index}\n  它应该入库（.gitignore 里放行了 index.html 与 LICENSE）。找不到说明仓库不完整。`
    )
  }
  const list = [...new Set([...fs.readFileSync(index, 'utf8').matchAll(GF_RE)].map((m) => m[1]))].sort()
  if (list.length === 0) throw new Error(`${index} 里抠不出任何 \`gf: '…'\` —— JIZURA 换了构建方式？`)
  return list
}

/** 取一个家族的 CSS；`css-raw/` 里有就直接用（除非 --force）。 */
async function getCss(spec, force) {
  const safe = spec.replace(/[^A-Za-z0-9]/g, '_')
  const cache = path.join(RAW_DIR, `${safe}.css`)
  if (isFile(cache) && !force) return fs.readFileSync(cache, 'utf8')

  const url = `https://fonts.googleapis.com/css2?family=${spec}&display=swap`
  let css
  try {
    css = await fetchTextOrBuffer(url, {binary: false})
  } catch (e) {
    throw new Error(`取 CSS 失败（${spec}）：${e.message}`)
  }
  if (!css.includes('@font-face')) throw new Error(`${spec} 的 CSS 里没有 @font-face —— 家族名写错了？`)
  mkdirp(RAW_DIR)
  fs.writeFileSync(cache, css, 'utf8')
  return css
}

/** 远程 URL → 相对路径（`fonts/<文件名>`）。与 URL_RE 保持同一套匹配规则。 */
function relativize(css) {
  return css.replace(URL_RE, (whole) => 'fonts/' + whole.split('/').pop())
}

async function downloadOne(url) {
  const leaf = url.split('/').pop()
  const dest = path.join(FONT_DIR, leaf)
  const buf = await fetchTextOrBuffer(url, {binary: true})
  if (!buf || buf.length === 0) throw new Error('下到的文件是空的')
  // 先写 .part 再改名 —— 断了不会留下一个看着完整的 woff2
  const part = dest + '.part'
  fs.writeFileSync(part, buf)
  fs.renameSync(part, dest)
}

/** 并发跑 `tasks`（最多 `limit` 个）。`onDone(i)` 每个完成时回调。 */
async function pool(items, limit, worker, onDone) {
  let next = 0
  const run = async () => {
    for (;;) {
      const i = next++
      if (i >= items.length) return
      await worker(items[i], i)
      onDone(i)
    }
  }
  await Promise.all(Array.from({length: Math.min(limit, items.length)}, run))
}

function parseArgs(argv) {
  const a = {only: null, force: false}
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--force') a.force = true
    else if (argv[i] === '--only') a.only = argv[++i]
  }
  return a
}

export async function main(argv = process.argv.slice(2)) {
  const a = parseArgs(argv)

  const problems = assets.selfCheck()
  if (problems.length) {
    say('assets.mjs 自检没过：')
    for (const item of problems) say(`  - ${item}`)
    return 1
  }

  const proxy = process.env.VSYNTH_FONT_PROXY
  say(`代理：${proxy || (process.env.HTTPS_PROXY || process.env.https_proxy || '(无，直连)')}`)

  // ① 先把上游那两个文件取下来（index.html 顺手打补丁）。
  //    ⚠️ 必须在 specsFromIndex() 之前 —— 家族清单是从 index.html 里读的。
  mkdirp(DEST)
  say(`上游文件（${assets.UPSTREAM_JIZURA_REPO} @ ${assets.UPSTREAM_JIZURA_SHA.slice(0, 12)}）`)
  const bad = await fetchUpstreamFiles(a.force)
  if (bad.length) {
    for (const item of bad) say(`  失败：${item}`)
    say('上游文件没取全 —— index.html / LICENSE 只有上游这一个来源')
    return 1
  }

  let specs = specsFromIndex()
  if (a.only) {
    if (!specs.includes(a.only)) {
      say(`index.html 里没有家族 ${a.only}（有的是 ${specs}）`)
      return 1
    }
    specs = [a.only]
  }
  say(`家族：${specs.length} 个（从 index.html 的 gf: 字段读出）`)

  mkdirp(FONT_DIR)
  mkdirp(RAW_DIR)

  // ① 逐个家族取 CSS，拼成一份大的
  const parts = [
    '/* Google Fonts used by JIZURA, self-hosted (SIL Open Font License 1.1).',
    ' * Generated by tools/fetch_jizura_fonts.mjs -- do not edit by hand. */',
  ]
  const allCss = []
  for (let i = 0; i < specs.length; i++) {
    const spec = specs[i]
    const cached = isFile(path.join(RAW_DIR, spec.replace(/[^A-Za-z0-9]/g, '_') + '.css')) && !a.force
    say(`[${i + 1}/${specs.length}] ${spec}` + (cached ? '（用缓存）' : ''))
    const css = await getCss(spec, a.force)
    parts.push(`/* ${spec} */`)
    parts.push(css)
    allCss.push(css)
  }

  // ② 收集要下的 woff2。同一个文件常被多条 @font-face 引用 —— 去重。
  const refs = [...new Set((allCss.join('\n').match(URL_RE) || []))].sort()
  const leaves = new Map()
  for (const url of refs) leaves.set(url.split('/').pop(), url)

  const todo = [...leaves.entries()].sort().filter(([leaf]) => !isFile(path.join(FONT_DIR, leaf))).map(([, u]) => u)
  say(`${refs.length} 条 URL 引用 / ${leaves.size} 个不同文件，还要下 ${todo.length} 个`)

  // ③ 并发下载。串行实测要两个多小时（~11 个/分钟），必须并发。
  const failed = []
  if (todo.length) {
    let done = 0
    await pool(todo, MAX_WORKERS, async (url) => {
      try { await downloadOne(url) } catch (e) { failed.push(`${url}  (${e.constructor.name}: ${e.message})`) }
    }, () => {
      done++
      if (done % 200 === 0) say(`  ${done} / ${todo.length}`)
    })
    say(`  ${done} / ${todo.length}`)
  }

  // ④ 写 fonts.css（URL 已改成相对路径）
  // ⚠️ `--only` 时**不写** fonts.css，改写旁边的 `fonts.css.debug` —— fonts.css 是生成物、
  //    不入库，被调试覆盖掉没有 git 能救回来，表现只是「PV 页少字体、不报错」。
  let cssPath = path.join(DEST, 'fonts.css')
  if (a.only) {
    cssPath = path.join(DEST, 'fonts.css.debug')
    say(`（--only：清单写到 ${path.basename(cssPath)}，不动 fonts.css）`)
  }
  fs.writeFileSync(cssPath, relativize(parts.join('\n')), 'utf8')

  const files = fs.readdirSync(FONT_DIR).filter((f) => f.endsWith('.woff2'))
  const total = files.reduce((s, f) => s + fs.statSync(path.join(FONT_DIR, f)).size, 0)
  say(`完成：盘上 ${files.length} 个文件，${(total / assets.MB).toFixed(1)} MB → ${DEST}`)

  if (failed.length) {
    for (const item of failed) say(`  失败：${item}`)
    say('这套字体不完整，不要当成能用的')
    return 1
  }

  // ⑤ 自检：fonts.css 引用的每一个文件都得在盘上（缺一个就是一个字形缺失）。
  const refsInCss = [...new Set(
    [...fs.readFileSync(cssPath, 'utf8').matchAll(/fonts\/([A-Za-z0-9._\-]+\.woff2)/g)].map((m) => m[1])
  )].sort()
  const missing = refsInCss.filter((n) => !isFile(path.join(FONT_DIR, n)))
  if (missing.length) {
    say(`fonts.css 引用了 ${missing.length} 个盘上没有的字体，头一个是 ${missing[0]}`)
    return 1
  }
  say(`${path.basename(cssPath)} 引用 ${refsInCss.length} 个，全部在盘上`)

  // ⑥ 再按表的 need 核一遍（含 fonts 的 min_files）。借用 fetch_tools 的核对。
  //    ⚠️ `--only` 时跳过 —— 那时 fonts.css 写去了 .debug，盘上仍是上次的完整状态。
  if (a.only) {
    say('（--only：跳过按表的完整性核对）')
    return 0
  }
  const problems2 = verify([JIZURA], HERE)
  if (problems2.length) {
    for (const item of problems2) say(`  缺：${item}`)
    return 1
  }
  return 0
}

if (isMain(import.meta.url)) {
  ensureFetchProxy()
  main().then((code) => process.exit(code), (e) => { say(`✗ ${e.message}`); process.exit(1) })
}
