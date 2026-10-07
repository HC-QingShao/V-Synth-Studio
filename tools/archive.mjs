/**
 * 解压 zip 与 tar.* —— 只用 Node 自带的东西。
 *
 *   zip     自己读中央目录 + `zlib.inflateRawSync`
 *   tar.*   `zlib.createGunzip` + 自己解析 512 字节块
 *   .tar.xz Linux 的 ffmpeg 上游只出 xz，而 Node 没有 liblzma —— 这一步交给系统的
 *           `xz -dc`（Linux/macOS 一般自带）。Windows 的产物全是 zip，碰不到这里。
 *
 * ⚠️ **不用 `unzip` / `tar` 命令行解 zip/tar**：那会引入一堆平台差异
 *    （Windows 上不一定有），而 zip/tar 的解析就这么点东西，自己来更可控。
 *
 * 安全：条目名一律先规范化再落到 `into` 下，越界的当场报错 —— 不静默改写。
 * 与 Rust 侧（`svsep.rs` 用 `enclosed_name()`）取同一条线。
 *
 * tar 里只认四种条目：常规文件、目录、符号链接、硬链接（别的类型跳过）。
 * ⚠️ **符号链接必须建出来**：LibreSVIP 的 macOS 包靠 `_internal/Python` 那几条链接
 * 找解释器，漏了不报错、只是那个程序起不来。
 */

import {spawnSync} from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import zlib from 'node:zlib'
import {fileURLToPath} from 'node:url'
import {download, isFile, mkdirp, rmdirIf, say, walkFiles} from './lib.mjs'

/** 解压之后必须存在的东西写在表里，解压本身只负责「别炸、别越界」。 */
function safeJoin(base, name) {
  const norm = name.replaceAll('\\', '/')
  const dest = path.resolve(base, norm)
  const rel = path.relative(base, dest)
  if (rel === '') return dest
  if (rel.startsWith('..') || path.isAbsolute(rel)) {
    throw new Error(`压缩包里的路径不安全（会解到目标目录外面）：${name}`)
  }
  return dest
}

// ── zip ─────────────────────────────────────────────────────────────
function unzip(archive, into) {
  const buf = fs.readFileSync(archive)
  // 从尾部找中央目录结束记录（EOCD，签名 0x06054b50），注释最长 64KB。
  let eocd = -1
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 22 - 0xffff); i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) { eocd = i; break }
  }
  if (eocd < 0) throw new Error(`不是 zip（找不到中央目录）：${archive}`)
  const count = buf.readUInt16LE(eocd + 10)
  let off = buf.readUInt32LE(eocd + 16)
  const base = path.resolve(into)

  for (let n = 0; n < count; n++) {
    if (buf.readUInt32LE(off) !== 0x02014b50) throw new Error('zip 中央目录坏了')
    const method = buf.readUInt16LE(off + 10)
    const compSize = buf.readUInt32LE(off + 20)
    const nameLen = buf.readUInt16LE(off + 28)
    const extraLen = buf.readUInt16LE(off + 30)
    const commentLen = buf.readUInt16LE(off + 32)
    const localOff = buf.readUInt32LE(off + 42)
    const name = buf.subarray(off + 46, off + 46 + nameLen).toString('utf8')
    off += 46 + nameLen + extraLen + commentLen

    if (name.endsWith('/')) { mkdirp(safeJoin(base, name)); continue }
    // 本地头的 name/extra 长度可能和中央目录不同，按本地头重算数据起点
    if (buf.readUInt32LE(localOff) !== 0x04034b50) throw new Error(`zip 本地头坏了：${name}`)
    const lNameLen = buf.readUInt16LE(localOff + 26)
    const lExtraLen = buf.readUInt16LE(localOff + 28)
    const dataStart = localOff + 30 + lNameLen + lExtraLen
    const raw = buf.subarray(dataStart, dataStart + compSize)
    const data = method === 0 ? raw : zlib.inflateRawSync(raw)

    const dest = safeJoin(base, name)
    mkdirp(path.dirname(dest))
    fs.writeFileSync(dest, data)
  }
}

// ── tar ─────────────────────────────────────────────────────────────
/** 解析一段 tar（Buffer），把文件解到 `base`。 */
function untar(tarbuf, base) {
  let p = 0
  let longName = null
  let paxPath = null
  while (p + 512 <= tarbuf.length) {
    const block = tarbuf.subarray(p, p + 512)
    if (block.every((b) => b === 0)) break            // 结尾的全零块

    const str = (from, len) => block.subarray(from, from + len).toString('utf8').replace(/\0.*$/, '')
    const sizeField = str(124, 12).trim()
    const size = sizeField ? parseInt(sizeField, 8) : 0
    const type = String.fromCharCode(block[156] || 0x30)
    let name = str(0, 100)
    const prefix = str(345, 155)
    if (prefix) name = `${prefix}/${name}`
    const dataStart = p + 512
    const dataEnd = dataStart + size
    const next = dataStart + Math.ceil(size / 512) * 512

    if (type === 'L') {                                // GNU 长文件名
      longName = tarbuf.subarray(dataStart, dataEnd).toString('utf8').replace(/\0.*$/, '')
      p = next
      continue
    }
    if (type === 'x' || type === 'g') {                // pax 扩展头
      const rec = tarbuf.subarray(dataStart, dataEnd).toString('utf8')
      const m = rec.match(/\d+ path=([^\n]+)\n/)
      if (m) paxPath = m[1]
      p = next
      continue
    }
    if (longName) { name = longName; longName = null }
    if (paxPath) { name = paxPath; paxPath = null }

    if (type === '0' || type === '\0' || type === '') {
      const dest = safeJoin(base, name)
      mkdirp(path.dirname(dest))
      fs.writeFileSync(dest, tarbuf.subarray(dataStart, dataEnd))
    } else if (type === '5') {
      mkdirp(safeJoin(base, name))
    } else if (type === '2') {
      /* 符号链接：LibreSVIP 的 macOS 包里有 4 条（`_internal/Python ->
         Python.framework/Versions/3.14/Python` 等），PyInstaller 的包靠它们找解释器。
         不建出来的话**不报错**，只是那个程序在 mac 上起不来。 */
      const dest = safeJoin(base, name)
      const target = str(157, 100)
      /* 只查「链接指向解压目录外面」这一条：越界的链接会让**后面**的条目顺着它写到
         目标之外，而条目名本身是干净的（`safeJoin` 拦不住这种情况）。 */
      const outside = path.relative(base, path.resolve(path.dirname(dest), target))
      if (path.isAbsolute(target) || outside.startsWith('..')) {
        throw new Error(`压缩包里的链接指向解压目录外面：${name} -> ${target}`)
      }
      if (process.platform === 'win32') {
        // Windows 建符号链接要开发者模式或管理员权限；本仓在 Windows 上的产物全是 zip
        continue
      }
      mkdirp(path.dirname(dest))
      fs.rmSync(dest, {force: true})
      fs.symlinkSync(target, dest)
    } else if (type === '1') {
      // 硬链接：目标是在这份 tar 里**前面**出现过的条目，照它再挂一个名字
      const dest = safeJoin(base, name)
      const target = str(157, 100)
      mkdirp(path.dirname(dest))
      fs.rmSync(dest, {force: true})
      try {
        fs.linkSync(safeJoin(base, target), dest)
      } catch {
        fs.copyFileSync(safeJoin(base, target), dest)
      }
    }
    p = next
  }
}

function untarFile(tarfile, into) {
  untar(fs.readFileSync(tarfile), path.resolve(into))
}

/** `.tar.xz` → 先用系统的 `xz -dc` 解成 .tar，再走 tar 解析。 */
function untarXz(archive, into, tmp) {
  const tarfile = path.join(tmp, 'decompressed.tar')
  const r = spawnSync('xz', ['-dc', archive], {maxBuffer: 1 << 30, stdio: ['ignore', 'pipe', 'inherit']})
  if (r.error && r.error.code === 'ENOENT') {
    throw new Error(
      '解 .tar.xz 需要系统的 `xz` 命令（Node 没有 liblzma）。\n' +
      '  Linux：装 xz-utils；macOS：brew install xz。\n' +
      '  Windows 的产物全是 zip，碰不到这一步。'
    )
  }
  if (r.status !== 0) throw new Error(`xz -dc 失败（退出码 ${r.status}）：${archive}`)
  fs.writeFileSync(tarfile, r.stdout)
  untarFile(tarfile, into)
  fs.rmSync(tarfile, {force: true})
}

// ── 对外 ────────────────────────────────────────────────────────────
const SUFFIXES = ['.tar.xz', '.tgz', '.tar.gz', '.tar', '.zip']

/** 按扩展名判该用哪种解压。 */
export function archiveSuffix(url) {
  return SUFFIXES.find((s) => url.endsWith(s)) || '.zip'
}

/** 解压到 `into`（先清空）。按扩展名分派。 */
export function extractAny(archive, into, tmp) {
  rmdirIf(into)
  mkdirp(into)
  const name = archive.toLowerCase()
  if (name.endsWith('.zip')) return unzip(archive, into)
  if (name.endsWith('.tar.xz')) return untarXz(archive, into, tmp)
  if (name.endsWith('.tar.gz') || name.endsWith('.tgz') || name.endsWith('.tar')) {
    // gzip/tar 都小（onnxruntime 几十 MB），一次读进内存解
    if (name.endsWith('.tar')) return untarFile(archive, into)
    const out = path.join(tmp, 'decompressed.tar')
    fs.writeFileSync(out, zlib.gunzipSync(fs.readFileSync(archive)))
    untarFile(out, into)
    fs.rmSync(out, {force: true})
    return
  }
  throw new Error(`不认识的压缩格式：${archive}`)
}

/** 在解开的树里找第一个名为 `filename` 的文件。 */
export function findOne(where, filename) {
  for (const f of walkFiles(where)) {
    if (path.basename(f) === filename) return f
  }
  return null
}

/**
 * 取一份**自建归档**（`tools.zip` / `jizura.zip`）：本地目录 → 自建地址 → 回 `null`（走上游）。
 *
 * ⚠️ 给了 `localDir` 就是明确要求走本地（离线打包）：那里没有这个 zip 要**报错**，
 * 不能静默回上游 —— 那会让「离线」这件事看起来做到了，而实际在联网。
 *
 * 自建地址失败只回 `null` 不抛：它本来就是退路，取不到该继续走上游。
 */
export async function obtainArchive({file, tmp, localDir, url}) {
  if (localDir) {
    const p = path.join(localDir, file)
    if (!isFile(p)) {
      throw new Error(`本地归档目录里没有 ${file}：${p}\n  --local / VSS_ASSETS_LOCAL 指的是「存着归档的目录」。`)
    }
    say(`  用本地归档：${p}`)
    return p
  }
  if (!url) return null
  const out = path.join(tmp, file)
  try {
    await download(url, out, {tries: 1})
    return out
  } catch (e) {
    say(`  自建归档取不到（${e.constructor.name}: ${e.message}），回上游`)
    return null
  }
}

// 自测入口：`node tools/archive.mjs --selftest`
if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1]) && process.argv.includes('--selftest')) {
  {
    const os = await import('node:os')
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'vss-arc-'))
    const out = path.join(tmp, 'out')
    const zip = path.join(tmp, 'a.zip')
    // 造一个 zip：用系统的 zip（只用于自测）
    fs.mkdirSync(path.join(tmp, 'src/sub'), {recursive: true})
    fs.writeFileSync(path.join(tmp, 'src/sub/f.txt'), 'hello zip')
    const cr = spawnSync('zip', ['-qr', zip, '.'], {cwd: path.join(tmp, 'src')})
    if (cr.status === 0) {
      extractAny(zip, out, tmp)
      say(`zip: ${fs.readFileSync(path.join(out, 'sub/f.txt'), 'utf8')}`)
    } else {
      say('（跳过 zip 自测：本机没有 zip 命令）')
    }
    // tar.gz
    const tgz = path.join(tmp, 'a.tgz')
    const cr2 = spawnSync('tar', ['-czf', tgz, '-C', path.join(tmp, 'src'), '.'])
    if (cr2.status === 0) {
      extractAny(tgz, out, tmp)
      say(`tgz: ${fs.readFileSync(path.join(out, 'sub/f.txt'), 'utf8')}`)
    }
    rmdirIf(tmp)
  }
}
