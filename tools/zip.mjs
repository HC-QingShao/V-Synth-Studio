/**
 * 一个够用的 zip 写入器 —— 只用 Node 自带的东西。
 *
 * 为什么自己写：Node 没有内置的 zip **写入**，而打包的对象里有几 GB 的包
 * （`runtime.zip` 压缩后仍 >4 GiB），所以必须支持 **Zip64**（64 位的条目偏移）。
 * 引入第三方 native 依赖不值得，这点东西自己写更可控。
 *
 * 只做「我们需要的」：deflate 压缩、UTF-8 文件名、目录条目可选、Zip64（按需）。
 * 每个文件先压到一个临时文件、拿到真实大小再写本地头 —— 不用数据描述符，
 * 结构简单、好核。
 */

import fs from 'node:fs'
import path from 'node:path'
import zlib from 'node:zlib'
import {pipeline} from 'node:stream/promises'
import {Transform, Writable} from 'node:stream'

const SIG_LOCAL = 0x04034b50
const SIG_CENTRAL = 0x02014b50
const SIG_EOCD = 0x06054b50
const SIG_EOCD64 = 0x06064b50
const SIG_LOCATOR = 0x07064b50

const U32 = 0xffffffff
const U16 = 0xffff

// ── CRC-32 ──────────────────────────────────────────────────────────
const CRC_TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c >>> 0
  }
  return t
})()

class Crc32 {
  constructor() { this.c = 0xffffffff }
  update(buf) {
    let c = this.c
    for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8)
    this.c = c
  }
  digest() { return (this.c ^ 0xffffffff) >>> 0 }
}

// ── 小端写 ──────────────────────────────────────────────────────────
function u16(n) { const b = Buffer.alloc(2); b.writeUInt16LE(n >>> 0); return b }
function u32(n) { const b = Buffer.alloc(4); b.writeUInt32LE(n >>> 0); return b }
function u64(n) {
  const b = Buffer.alloc(8)
  b.writeBigUInt64LE(BigInt(n))
  return b
}

/** DOS 时间戳。固定成一个值 —— 可复现的包比「记得住打包时刻」值钱。 */
const DOS_TIME = 0
const DOS_DATE = (1 << 5) | 1   // 1980-01-01

/** 把一个文件压成 deflate 流落到 `tmp`，返回 {crc, uncompressed, compressed}。 */
async function deflateTo(diskPath, tmp, level) {
  const crc = new Crc32()
  let uncompressed = 0
  let compressed = 0
  const fd = fs.openSync(tmp, 'w')
  try {
    await pipeline(
      fs.createReadStream(diskPath, {highWaterMark: 1 << 20}),
      new Transform({
        transform(chunk, _enc, cb) { crc.update(chunk); uncompressed += chunk.length; cb(null, chunk) },
      }),
      zlib.createDeflateRaw({level}),
      new Writable({write(chunk, _enc, cb) { compressed += chunk.length; fs.writeSync(fd, chunk); cb() }}),
    )
  } finally { fs.closeSync(fd) }
  return {crc: crc.digest(), uncompressed, compressed}
}

/**
 * 打一个 zip。
 *
 * @param entries      [{diskPath, name}]（顺序即写入顺序）
 * @param outPath      产物路径
 * @param level        zlib 压缩档位（0 = 不压）
 * @param onProgress   每收一个文件回调 (done, total)
 */
export async function writeZip(entries, outPath, {level = 9, onProgress} = {}) {
  fs.mkdirSync(path.dirname(outPath), {recursive: true})
  fs.rmSync(outPath, {force: true})
  const tmp = outPath + '.deflate'
  const fd = fs.openSync(outPath, 'w')
  let offset = 0
  const put = (buf) => { fs.writeSync(fd, buf); offset += buf.length }
  const central = []

  try {
    let done = 0
    for (const e of entries) {
      const {crc, uncompressed, compressed} = await deflateTo(e.diskPath, tmp, level)
      const name = Buffer.from(e.name, 'utf8')
      const localOffset = offset
      const needZip64 = localOffset >= U32 || uncompressed >= U32 || compressed >= U32

      // 本地头（Zip64 额外字段在需要时补）
      const extra = needZip64
        ? Buffer.concat([u16(0x0001), u16(24), u64(uncompressed), u64(compressed), u64(localOffset)])
        : Buffer.alloc(0)
      const header = Buffer.concat([
        u32(SIG_LOCAL), u16(needZip64 ? 45 : 20), u16(0x0800), u16(8),
        u16(DOS_TIME), u16(DOS_DATE), u32(crc),
        u32(compressed), u32(uncompressed), u16(name.length), u16(extra.length),
      ])
      put(header); put(name); put(extra)

      // 文件数据：从临时文件分块拷进去
      const rfd = fs.openSync(tmp, 'r')
      try {
        const buf = Buffer.alloc(1 << 20)
        for (;;) {
          const k = fs.readSync(rfd, buf, 0, buf.length, null)
          if (!k) break
          put(buf.subarray(0, k))
        }
      } finally { fs.closeSync(rfd) }

      central.push({name, crc, compressed, uncompressed, localOffset})
      if (onProgress) onProgress(++done, entries.length)
    }

    // 中央目录
    const cdStart = offset
    for (const c of central) {
      const needZip64 = c.localOffset >= U32 || c.uncompressed >= U32 || c.compressed >= U32
      const extra = needZip64
        ? Buffer.concat([u16(0x0001), u16(24), u64(c.uncompressed), u64(c.compressed), u64(c.localOffset)])
        : Buffer.alloc(0)
      put(Buffer.concat([
        u32(SIG_CENTRAL), u16(0x031e), u16(needZip64 ? 45 : 20), u16(0x0800), u16(8),
        u16(DOS_TIME), u16(DOS_DATE), u32(c.crc),
        u32(needZip64 ? U32 : c.compressed), u32(needZip64 ? U32 : c.uncompressed),
        u16(c.name.length), u16(extra.length), u16(0), u16(0), u16(0),
        u32(0), u32(needZip64 ? U32 : c.localOffset),
      ]))
      put(c.name); put(extra)
    }
    const cdSize = offset - cdStart

    // Zip64 结束记录（按需）
    const needEocd64 = cdStart >= U32 || cdSize >= U32 || central.length > U16
    if (needEocd64) {
      const eocd64Off = offset
      put(Buffer.concat([
        u32(SIG_EOCD64), u64(44), u16(0x031e), u16(45), u32(0), u32(0),
        u64(central.length), u64(central.length), u64(cdSize), u64(cdStart),
      ]))
      put(Buffer.concat([u32(SIG_LOCATOR), u32(0), u64(eocd64Off), u32(1)]))
    }

    put(Buffer.concat([
      u32(SIG_EOCD), u16(0), u16(0),
      u16(Math.min(central.length, U16)), u16(Math.min(central.length, U16)),
      u32(Math.min(cdSize, U32)), u32(Math.min(cdStart, U32)), u16(0),
    ]))
  } finally {
    fs.closeSync(fd)
    fs.rmSync(tmp, {force: true})
  }
  return fs.statSync(outPath).size
}
