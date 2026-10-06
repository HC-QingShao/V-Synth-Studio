#!/usr/bin/env node
/**
 * 补齐 `tools/` —— 干净机器 / CI 上编译前要跑的第一件事。
 *
 *     node tools/fetch_tools.mjs
 *     node tools/fetch_tools.mjs --only ffmpeg        # 只补一件
 *     node tools/fetch_tools.mjs --force
 *     node tools/fetch_tools.mjs --root <目录>        # 补到别处（验证用）
 *
 * 这个仓库只放源码。`tools/`（ffmpeg + yt-dlp + LibreSVIP CLI + onnxruntime，
 * 约 200MB）不入库，但它由 tauri.conf.json 的 `bundle.resources` 打进安装包，
 * 缺了程序就没有音频转换 / MV 下载 / 工程格式互转 / 人声转 MIDI。
 *
 * 各件产物**互相独立**：各自一个上游、各自判「缺不缺」。URL 见 `UPSTREAM`（按平台分列）。
 *
 * ⚠️ 「运行期去哪儿找」只在 `src-tauri/src/artifact/mod.rs` 那张表里写一次。
 * 下面 `verify()` 列的是「打包前必须存在」的文件，与 `build.rs` 的 MUST_HAVE 对应 ——
 * 这份重复是有意的：为了在「还没编译」时给人一句能读懂的错。
 */

import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import process from 'node:process'
import {spawnSync} from 'node:child_process'

import * as assets from './assets.mjs'
import {HERE, copyFile, download, isDir, isFile, mkdirp, rmrf, say, walkFiles, isMain} from './lib.mjs'
import {archiveSuffix, extractAny, findOne} from './archive.mjs'

// ── 平台 ─────────────────────────────────────────────────────────────
// 三端各取各的上游二进制，落点形状一致（`tools/ffmpeg/bin/ffmpeg`…），
// 只是文件名在 Windows 上带 `.exe`。
const PLATFORM = {win32: 'windows', darwin: 'macos'}[process.platform] || 'linux'
const MARK_ROOT = assets.ROOT
const MARK_WRITABLE = assets.WRITABLE

const exe = (name) => (PLATFORM === 'windows' ? `${name}.exe` : name)

// ── 产物：每件独立 ──────────────────────────────────────────────────
// 各自一个上游、各自判「缺不缺」。URL 都是上游发布页的直链，没有镜像、没有网盘兜底。
// ⚠️ 上游换版本时这里要一起改（版本号在 URL 里）。
const UPSTREAM = {
  // BtbN 的 lgpl 档：程序只用音频编码器（mp3lame / vorbis / opus + 内建的 aac/flac/pcm），
  // lgpl 全都有。gpl 档多带 x264/x265，而程序从不编码视频（6 处调用都是 `-vn`）。
  ffmpeg: {
    windows: 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl.zip',
    linux: 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-lgpl.tar.xz',
    // BtbN 不出 macOS 档；evermeet.cx 是 macOS ffmpeg 的常用发布点。
    macos: 'https://evermeet.cx/ffmpeg/getrelease/zip',
  },
  ytdlp: {
    windows: 'https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe',
    linux: 'https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_linux',
    macos: 'https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos',
  },
  // ⚠️ LibreSVIP 只发 Windows 构建；其余平台这一件补不了（见 sourceOf 的报错）。
  libresvip: {
    windows: 'https://github.com/SoulMelody/LibreSVIP/releases/download/v2.9.0/LibreSVIP-CLI-2.9.0.win-amd64.zip',
  },
  // ONNX Runtime 官方 release。整包里只拎出那一个动态库。
  onnxruntime: {
    windows: 'https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-win-x64-1.23.2.zip',
    linux: 'https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-x64-1.23.2.tgz',
    macos: 'https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-osx-arm64-1.23.2.tgz',
  },
}

function sourceOf(name) {
  const url = UPSTREAM[name][PLATFORM]
  if (!url) throw new Error(`${name} 没有 ${PLATFORM} 的上游地址（上游只发了别的平台）`)
  return url
}

// ── 解包落点 ────────────────────────────────────────────────────────
/** 用户可写目录。⚠️ 必须与 Rust 侧 `lib.rs::user_data_dir()` 一致，抄错了会「下完了但认不出来」。 */
function writableDir() {
  if (process.platform === 'win32') {
    const base = process.env.APPDATA || path.join(os.homedir(), 'AppData/Roaming')
    return path.join(base, 'com.qingmu.vocalworkstation')
  }
  if (process.platform === 'darwin') {
    return path.join(os.homedir(), 'Library/Application Support/com.qingmu.vocalworkstation')
  }
  const base = process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local/share')
  return path.join(base, 'com.qingmu.vocalworkstation')
}

/** 把表里的 `<root>` / `<writable>` 标记换成真路径。 */
function destOf(asset, root) {
  if (asset.into.startsWith(MARK_WRITABLE + '/')) {
    return path.join(writableDir(), asset.into.slice(MARK_WRITABLE.length + 1))
  }
  if (asset.into.startsWith(MARK_ROOT + '/')) {
    return path.join(root, asset.into.slice(MARK_ROOT.length + 1))
  }
  throw new Error(`${asset.id}: into 既不是 ${MARK_ROOT} 也不是 ${MARK_WRITABLE}`)
}

// ── 从上游补齐 ──────────────────────────────────────────────────────
const PARTS = {
  ffmpeg: {what: 'ffmpeg（音频转换 / 合并 / 扒谱解码）'},
  ytdlp: {what: 'yt-dlp（MV 解析下载）'},
  libresvip: {what: 'LibreSVIP CLI（工程格式互转）'},
  onnxruntime: {what: 'ONNX Runtime（人声转 MIDI 的推理运行时）'},
}

/** 某件产物在本平台下的相对落点（Windows 加 `.exe`；`onnxruntime` 换动态库名）。 */
function needRel(name) {
  const dll = {windows: 'onnxruntime.dll', macos: 'libonnxruntime.dylib'}[PLATFORM] || 'libonnxruntime.so'
  if (name === 'ffmpeg') return `ffmpeg/bin/${exe('ffmpeg')}`
  if (name === 'ytdlp') return exe('yt-dlp')
  if (name === 'libresvip') return `libresvip/libresvip-cli/${exe('libresvip-cli')}`
  return `onnxruntime/${dll}`
}

function installFfmpeg(ex, base) {
  // ⚠️ **只要 ffmpeg**，不拷 ffprobe —— 媒体信息由 `ffmpeg -i` 自己读（见 audio.rs）。
  const found = findOne(ex, exe('ffmpeg'))
  if (!found) throw new Error(`包里找不到 ${exe('ffmpeg')}（上游布局变了？看 ${ex}）`)
  copyFile(found, path.join(base, 'ffmpeg', 'bin', exe('ffmpeg')))
}

function installLibresvip(ex, base) {
  // 整个 `libresvip-cli/` 目录搬到位 —— 只搬 exe 会缺 `_internal/` 里的插件。
  const found = findOne(ex, exe('libresvip-cli'))
  if (!found) throw new Error(`包里找不到 ${exe('libresvip-cli')}（上游布局变了？看 ${ex}）`)
  const dst = path.join(base, 'libresvip', 'libresvip-cli')
  rmrf(dst)
  mkdirp(path.dirname(dst))
  fs.renameSync(path.dirname(found), dst)
}

function installOnnxruntime(ex, base) {
  // 只拎出那一个动态库（含相邻的 providers_shared），头文件/.lib/PDB 一律不落地。
  const name = {windows: 'onnxruntime.dll', macos: 'libonnxruntime.dylib'}[PLATFORM] || 'libonnxruntime.so'
  const found = findOne(ex, name)
  if (!found) throw new Error(`包里找不到 ${name}（上游布局变了？看 ${ex}）`)
  const dstDir = path.join(base, 'onnxruntime')
  mkdirp(dstDir)
  copyFile(found, path.join(dstDir, name))
  // `shared` provider 用不上，但放在同目录无害，且 ORT 会在本目录找它
  const shared = {windows: 'onnxruntime_providers_shared.dll', macos: 'libonnxruntime_providers_shared.dylib'}[PLATFORM]
    || 'libonnxruntime_providers_shared.so'
  const src = path.join(path.dirname(found), shared)
  if (isFile(src)) copyFile(src, path.join(dstDir, shared))
}

export async function fetchTools(tmp, force, root, only = null) {
  const base = destOf(assets.byId('tools'), root)
  mkdirp(base)

  const todo = only === null ? Object.keys(PARTS) : Object.keys(PARTS).filter((n) => only.includes(n))
  const missing = todo.filter((name) => force || !isFile(path.join(base, needRel(name))))
  if (missing.length === 0) {
    say(`  ${todo.join(' / ')} 都已在，跳过`)
    return
  }
  say(`  缺：${missing.join('、')}`)

  const failed = []
  for (const name of missing) {
    say(`  ── ${PARTS[name].what} ──`)
    try {
      const url = sourceOf(name)
      // `yt-dlp` 的上游就是一个裸二进制，不是压缩包 —— 直接下到目标位置。
      if (name === 'ytdlp') {
        await download(url, path.join(base, needRel(name)))
        continue
      }
      const z = path.join(tmp, `${name}${archiveSuffix(url)}`)
      await download(url, z)
      const ex = path.join(tmp, name)
      extractAny(z, ex, tmp)
      ;({ffmpeg: installFfmpeg, libresvip: installLibresvip, onnxruntime: installOnnxruntime}[name])(ex, base)
      rmrf(ex)
      fs.rmSync(z, {force: true})
    } catch (e) {
      failed.push(`${name}: ${e.message}`)
    }
  }

  if (failed.length) {
    say('')
    for (const item of failed) say(`  ✗ ${item}`)
    throw new Error(
      `${failed.length} 件补不齐。上游地址见 \`UPSTREAM\`（按平台分列）—— ` +
      `检查它是否还有效、这个平台上游发没发。`
    )
  }
}

// ── 校验：**这才是本脚本的重点** ─────────────────────────────────────
/**
 * 跑一下二进制，把首行版本号拿出来当**佐证**。
 *
 * ⚠️ 这纯属额外信息，**跑不起来不算失败**：在 Linux 上跑的可能是 Windows PE
 * （`EACCES`/`ENOEXEC`），那**正常** —— 所以措辞要能区分「平台不对」与「真坏了」。
 */
function runVersion(binary, args) {
  const r = spawnSync(binary, args, {encoding: 'utf8', timeout: 20_000})
  if (r.error) {
    const code = r.error.code
    if (code === 'EACCES' || code === 'ENOEXEC') return '（不是本平台的可执行文件，只核对存在）'
    return `（本机跑不了：${r.error.message}）`
  }
  const out = (r.stdout || r.stderr || '').trim()
  return out ? out.split('\n')[0] : '(无输出)'
}

// 想报版本的二进制：件名 → 跑它的参数。
const VERSION_ARGS = {ffmpeg: ['-version'], ytdlp: ['--version']}

/** 按表核一遍，缺什么现在就说。返回问题列表（空 = 齐了）。 */
export function verify(selected, root) {
  const bad = []
  for (const asset of selected) {
    const base = destOf(asset, root)
    const descs = []
    for (const nd of asset.need) {
      const p = path.join(base, nd.rel)
      if (nd.kind === 'dir') {
        const n = isDir(p) ? walkFiles(p).length : 0
        if (n < nd.minFiles) {
          bad.push(`${asset.into}/${nd.rel}/（只有 ${n} 个，至少要 ${nd.minFiles}）`)
          continue
        }
        descs.push(`${nd.rel}（${n} 个文件）`)
      } else if (!isFile(p)) {
        bad.push(`${asset.into}/${nd.rel}`)
      } else {
        const key = Object.keys(VERSION_ARGS).find((k) => nd.rel.endsWith(exe(k)))
        descs.push(key ? `${nd.rel}: ${runVersion(p, VERSION_ARGS[key])}` : nd.rel)
      }
    }
    if (descs.length) say(`  ${asset.id.padEnd(10)}: ${descs.join('，')}`)
  }
  return bad
}

// ── 主流程 ──────────────────────────────────────────────────────────
function parseArgs(argv) {
  const a = {only: 'all', force: false, root: null}
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--force') a.force = true
    else if (argv[i] === '--only') a.only = argv[++i]
    else if (argv[i] === '--root') a.root = argv[++i]
  }
  return a
}

export async function main(argv = process.argv.slice(2)) {
  const a = parseArgs(argv)
  const root = a.root ? path.resolve(a.root) : HERE
  if (a.root) say(`程序根（--root）：${root}`)

  const problems = assets.selfCheck()
  if (problems.length) {
    say('assets.mjs 自检没过，先修表：')
    for (const item of problems) say(`  - ${item}`)
    return 1
  }

  const want = a.only.split(',').filter(Boolean)
  if (!(want.length === 1 && want[0] === 'all')) {
    const unknown = want.filter((x) => !(x in PARTS))
    if (unknown.length) {
      say(`不认识的件：${unknown}（可选：${Object.keys(PARTS)}）`)
      return 1
    }
  }

  const tmp = path.join(root, 'src-tauri', 'target', 'vss-cache', 'tmp')
  mkdirp(tmp)

  say('补齐 tools/')
  await fetchTools(tmp, a.force, root, want.length === 1 && want[0] === 'all' ? null : want)

  say('')
  say('校验')
  const bad = verify([assets.byId('tools')], root)

  say('')
  if (bad.length) {
    say('不齐：')
    for (const item of bad) say(`  - ${item}`)
    return 1
  }
  say('全部齐了')
  return 0
}

if (isMain(import.meta.url)) {
  main().then((code) => process.exit(code), (e) => { say(`✗ ${e.message}`); process.exit(1) })
}
