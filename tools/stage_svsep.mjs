#!/usr/bin/env node
/**
 * 把「炽小阳音轨分离站离线版」的素材搬进 `data/svsep/`，供打包用。
 *
 *     node tools/stage_svsep.mjs --source /path/to/炽小阳音轨分离站离线版
 *     node tools/stage_svsep.mjs --source ... --target data/svsep
 *     node tools/stage_svsep.mjs --source ... --skip-runtime    # 只补 backend
 *     node tools/stage_svsep.mjs --source ... --dry-run         # 只说会搬什么
 *
 * 产出（就是 `pack_assets.mjs --only svsep.runtime,svsep.models` 的输入）：
 *
 *     data/svsep/
 *       models/     两个模型（BS-Roformer-SW.ckpt 667 MB + UVR-MDX-NET-Inst_HQ_3.onnx 63.7 MB）
 *       runtime/    Python 3.10 embeddable + torch/onnxruntime/audio-separator（7.6 GB）
 *       backend/    Flask 后端（不带前端：templates/ 与 static/ 不搬）
 *       bin/        ffmpeg.exe（159 MB）
 *
 * ⚠️ **这不是给用户跑的东西** —— 它只服务于开发机的打包流程。用户拿到的是 zip。
 *
 * ── 三条约束 ──────────────────────────────────────────────────────
 *
 * **① 路径是参数，不是常量。** `--source` 必给、`--target` 默认 `<root>/data/svsep`。
 * **② 搬动只用 Node 的 fs，不用 robocopy。** robocopy 只在 Windows 有。
 * **③ `templates/` 与 `static/` 不搬。** 原软件那套 Jinja 模板 + app.js/style.css 已经
 * 没人读（工作站用自己的 React 界面，后端只当 JSON API 用）。
 */

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'

import * as assets from './assets.mjs'
import {HERE, say, walkFiles, isMain} from './lib.mjs'

// 搬哪些子树、各自怎么过滤。与 assets.mjs 里 svsep.runtime 那条的 `exclude` 一致
// （那边管「打包时不收什么」，这里管「搬过来时就不要什么」—— 两边剔的是同一类东西）。
const TREES = [
  // backend 要剔的东西最多：它是从「离线版」搬的，那边自带一套前端。
  ['backend', ['__pycache__', '*.pyc', 'templates', 'static', 'index.html']],
  ['bin', []],
]

/**
 * 按**目录/文件名**剔（这里是 fs 层，拿到的是单个名字，不是相对路径）——
 * 所以只支持「名字等于 / 后缀等于」。别在这里用带 `/` 的模式，匹配不上。
 */
function shouldDrop(name, patterns) {
  return patterns.some((p) => (p.startsWith('*.') ? name.endsWith(p.slice(1)) : name === p))
}

function dirSize(dir) {
  const files = walkFiles(dir)
  return {n: files.length, bytes: files.reduce((s, f) => s + fs.statSync(f).size, 0)}
}

function copyTree(src, dst, patterns, dry) {
  say(`→ ${src}`)
  say(`  ${dst}`)
  if (!fs.existsSync(src) || !fs.statSync(src).isDirectory()) throw new Error(`源不存在：${src}`)
  const {n, bytes} = dirSize(src)
  say(`  源：${n.toLocaleString()} 个文件，${(bytes / assets.MB).toFixed(1)} MB` +
    (patterns.length ? `（剔掉 ${patterns.join(', ')}）` : ''))
  if (dry) return
  // ⚠️ 先删再拷，而不是「已存在就合并」：合并会**留下上次的残渣** —— 比如上一次搬进去、
  //    后来又被上游删掉的文件，会一直躺在那里、被打进 zip。
  fs.rmSync(dst, {recursive: true, force: true})
  fs.mkdirSync(path.dirname(dst), {recursive: true})
  fs.cpSync(src, dst, {
    recursive: true,
    filter: (from) => !shouldDrop(path.basename(from), patterns),
  })
}

function parseArgs(argv) {
  const a = {source: null, target: null, skipModels: false, skipRuntime: false, dryRun: false}
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--source') a.source = argv[++i]
    else if (argv[i] === '--target') a.target = argv[++i]
    else if (argv[i] === '--skip-models') a.skipModels = true
    else if (argv[i] === '--skip-runtime') a.skipRuntime = true
    else if (argv[i] === '--dry-run') a.dryRun = true
  }
  return a
}

export function main(argv = process.argv.slice(2)) {
  const a = parseArgs(argv)
  if (!a.source) { say('缺少 --source（「炽小阳音轨分离站离线版」的目录）'); return 1 }

  const problems = assets.selfCheck()
  if (problems.length) {
    say('assets.mjs 自检没过：')
    for (const item of problems) say(`  - ${item}`)
    return 1
  }

  const source = path.resolve(a.source)
  const res = path.join(source, 'resources')
  if (!fs.existsSync(res) || !fs.statSync(res).isDirectory()) {
    throw new Error(`源的 resources 目录不存在：${res}\n  （--source 应该指到「…音轨分离站离线版」那一层）`)
  }

  const target = a.target ? path.resolve(a.target) : path.join(HERE, 'data', 'svsep')
  say(`源  ：${source}`)
  say(`目标：${target}`)
  say('')

  // dry-run 不落盘：连这四个空目录也不建（否则 `--dry-run` 之后会留下一个空壳）
  if (!a.dryRun) {
    for (const name of ['models', 'runtime', 'backend', 'bin']) {
      fs.mkdirSync(path.join(target, name), {recursive: true})
    }
  }

  // models / runtime 是两棵整树（一个进 models.zip、一个进 runtime.zip）
  if (a.skipModels) say('models/    跳过（--skip-models）')
  else copyTree(path.join(res, 'models'), path.join(target, 'models'), [], a.dryRun)

  if (a.skipRuntime) say('runtime/    跳过（--skip-runtime）')
  else {
    // `*.pyc` 不搬：它们是**本机**编译的字节码，跟着分发既没用又可能版本不符
    copyTree(path.join(res, 'runtime'), path.join(target, 'runtime'), ['*.pyc'], a.dryRun)
  }

  for (const [name, patterns] of TREES) {
    copyTree(path.join(res, name), path.join(target, name), patterns, a.dryRun)
  }

  if (a.dryRun) { say(''); say('（--dry-run：什么都没写）'); return 0 }

  // ── 结果 + 自检 ────────────────────────────────────────────────
  say('')
  say('=== 结果 ===')
  let grandN = 0, grandB = 0
  for (const d of fs.readdirSync(target).filter((x) => fs.statSync(path.join(target, x)).isDirectory()).sort()) {
    const {n, bytes} = dirSize(path.join(target, d))
    grandN += n; grandB += bytes
    say(`${d.padEnd(12)} ${(bytes / assets.MB).toFixed(1).padStart(8)} MB  ${n.toLocaleString().padStart(6)} 文件`)
  }
  say(`${'总计'.padEnd(12)} ${(grandB / (1024 ** 3)).toFixed(2).padStart(8)} GB  ${grandN.toLocaleString().padStart(6)} 文件`)

  // 拿表里那条 need 核一遍 —— 「搬完了」和「搬齐了」不是一回事
  const runtime = assets.byId('svsep.runtime')
  const missing = runtime.need.filter((nd) => !fs.existsSync(path.join(target, nd.rel))).map((nd) => nd.rel)
  if (missing.length) {
    say('')
    say('⚠️ 关键的没搬齐（打包出来的 runtime.zip 用户下完仍起不来）：')
    for (const m of missing) say(`  - ${m}`)
    return 1
  }

  say('')
  say('搬齐了。下一步：')
  say('  node tools/pack_assets.mjs --only svsep.runtime,svsep.models')
  return 0
}

if (isMain(import.meta.url)) {
  main()
}
