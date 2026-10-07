#!/usr/bin/env node
/**
 * 打两份**自建归档** —— `资料归档/tools.zip` 与 `资料归档/jizura.zip`。
 *
 *     node tools/pack_archives.mjs
 *     node tools/pack_archives.mjs --out D:\某处 --force
 *
 * 它们是补齐脚本的第二条来源（见 `fetch_tools.mjs` / `fetch_jizura_fonts.mjs` 的三条来源）：
 * 两块随包产物的上游是 GitHub 与 Google Fonts，境内直连常不通，而它们又是打包必需的。
 * 归档由人手工上传到 `assets.mjs::SELF_HOST_BASE` 指的那个目录（与 models / runtime 同一层），
 * 补件脚本先试它、取不到才回上游；别人也可以直接 `--local <目录>` 用它，全程不联网。
 *
 * ── 两条硬约束 ─────────────────────────────────────────────────────
 *
 * **布局必须与仓库一致**：解包侧是按 `assets.mjs` 的 `strip` 找文件的 ——
 *   `tools.zip` 摊平（顶层直接是 `ffmpeg/`、`libresvip/`、`onnxruntime/`、`yt-dlp.exe`），
 *   `jizura.zip` 带一层 `jizura/` 壳。多一层少一层都不报错，只会解到错的位置，
 *   表现为「工具或 PV 页静默缺件」。
 *
 * **只打齐了的**：源目录缺件时打出来的包会在用户机器上才现形，所以先按表核一遍。
 * 这个脚本**只打包、不上传**。
 */

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'

import * as assets from './assets.mjs'
import {HERE, dirSize, human, isDir, isMain, say, walkFiles} from './lib.mjs'
import {writeZip} from './zip.mjs'
import {verify} from './fetch_tools.mjs'

// tools/ 里同时住着**脚本**（本文件就是），它们不是随包产物 —— 归档里混进旧脚本，
// 解包时会把新脚本覆盖回旧版，而没有任何地方会报错。
const TOOLS_SCRIPT_RE = /\.(mjs|ps1|cmd|zip)$/i

/** `tools/` 的归档条目：摊平，且只收产物（不收脚本）。 */
function toolsEntries(dir) {
  return walkFiles(dir)
    .map((f) => ({diskPath: f, rel: path.relative(dir, f).split(path.sep).join('/')}))
    .filter((e) => !(e.rel.includes('/') === false && TOOLS_SCRIPT_RE.test(e.rel)))
    .sort((a, b) => a.rel.localeCompare(b.rel))
    .map((e) => ({diskPath: e.diskPath, name: e.rel}))
}

/** `public/vendor/jizura/` 的归档条目：带一层 `jizura/` 壳。 */
function jizuraEntries(dir) {
  return walkFiles(dir)
    .map((f) => ({diskPath: f, rel: path.relative(dir, f).split(path.sep).join('/')}))
    .sort((a, b) => a.rel.localeCompare(b.rel))
    .map((e) => ({diskPath: e.diskPath, name: `jizura/${e.rel}`}))
}

const PLAN = [
  {id: 'tools', src: path.join(HERE, 'tools'), entries: toolsEntries},
  {id: 'jizura', src: path.join(HERE, 'public', 'vendor', 'jizura'), entries: jizuraEntries},
]

function parseArgs(argv) {
  const a = {out: path.join(HERE, '资料归档'), force: false, level: 6}
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--out') a.out = path.resolve(argv[++i])
    else if (argv[i] === '--force') a.force = true
    else if (argv[i] === '--level') a.level = Number(argv[++i])
  }
  return a
}

export async function main(argv = process.argv.slice(2)) {
  const a = parseArgs(argv)

  /* ⚠️ 只在 Windows 上打：归档的内容（`ffmpeg.exe` / `yt-dlp.exe`…）与**文件名**
     都是 Windows 的，而两端的归档共用一个名字 —— 在 macOS 上打会产出一份同名的
     另一个平台的包，传上去之后没有任何地方看得出来。 */
  if (assets.PLATFORM !== 'windows') {
    say(`打归档只在 Windows 上做（当前是 ${assets.PLATFORM}）：归档内容与文件名都是 Windows 产物。`)
    return 1
  }

  // 缺件的包在用户机器上才发现，所以这里先按表核一遍（它同时报出每件实测的东西）。
  say('按表核对源目录')
  const bad = verify(PLAN.map((p) => assets.byId(p.id)), HERE)
  if (bad.length) {
    say('')
    for (const item of bad) say(`  ✗ 缺：${item}`)
    say('\n先补齐再打包（`pnpm prepare:assets`），别打出一个缺件的包。')
    return 1
  }

  fs.mkdirSync(a.out, {recursive: true})
  for (const p of PLAN) {
    const out = path.join(a.out, assets.SELF_HOST_ARCHIVE[p.id])
    say('')
    say(`▶ ${p.src}`)
    if (!isDir(p.src)) {
      say(`  ✗ 源目录不存在：${p.src}`)
      return 1
    }
    if (fs.existsSync(out) && !a.force) {
      say(`  已存在，跳过（--force 覆盖）：${out}`)
      continue
    }
    const entries = p.entries(p.src)
    if (!entries.length) {
      say('  ✗ 一个文件都没有，打出来是个没用的 zip')
      return 1
    }
    const before = dirSize(p.src)
    say(`  ${before.n} 个文件，${human(before.bytes)} → ${path.basename(out)}`)
    let last = 0
    await writeZip(entries, out, {
      level: a.level,
      onProgress: (done, total) => {
        const pct = Math.floor((done * 100) / total)
        if (pct >= last + 10) { last = pct; say(`    ${pct}%`) }
      },
    })
    say(`  ✓ ${out}  （${human(fs.statSync(out).size)}）`)
  }

  say('')
  say(`归档目录：${a.out}`)
  say(`补件脚本会去这里取：${assets.SELF_HOST_BASE}/<名字>.zip`)
  say('传上去之后不用改代码（想换地址就设 VSS_ASSETS_URL）；别人拿到这两个 zip 后：')
  say(`  VSS_ASSETS_LOCAL=${a.out} pnpm prepare:assets`)
  return 0
}

if (isMain(import.meta.url)) {
  main().then((code) => process.exit(code), (e) => { say(`✗ ${e.message}`); process.exit(1) })
}
