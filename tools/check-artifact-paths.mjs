#!/usr/bin/env node
/**
 * 产物路径收口校验：生产代码里不该再出现「自己拼产物落点」。
 *
 * 允许的例外（每一条都有理由）：
 *   · 压缩包**内部**的布局（`lib/onnxruntime.dll`）—— 那不是产物落点，是上游 zip 的形状；
 *   · `lib.rs::resolve_paths` 里的 `resources.json` —— 它是启动哨兵（鸡生蛋）；
 *   · `xtask` 式的一次性 `write!` 落位（DML 包自带的目录形状）。
 *
 * 顺带跑 `assets.mjs` 的自检与 `assets.mjs ↔ build.rs` 的对照。
 */

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'

import * as assets from './assets.mjs'
import {HERE} from './lib.mjs'

const PATTERNS = {
  'ffmpeg 路径': /join\("ffmpeg"\)/,
  'libresvip 路径': /join\("libresvip"\)/,
  'yt-dlp 路径': /join\("yt-dlp"\)/,
  'svsep/models': /join\("svsep"\)\s*\.join\("models"\)/,
  'game/models': /join\("game"\)\s*\.join\("models"\)/,
  'pinyin.json': /join\("pinyin\.json"\)/,
  'resources.json': /join\("resources\.json"\)/,
  'tools 目录': /join\("tools"\)/,
}

// 允许自己拼 `tools/` 的地方（每一处都要有理由，写在代码注释里）
const TOOLS_ALLOWED = {
  // 界面上要显示这个路径 —— 不用于定位产物（那走 artifact 表）。
  'src-tauri/src/ipc/state.rs': '只用于界面显示',
  // yt-dlp 找不到随包副本时要写个「落点」目录给上游下载用。
  'src-tauri/src/ytdlp.rs': '上游下载的落点（待改问表）',
}

const relOf = (p) => path.relative(HERE, path.resolve(p)) || p

function walkRs(dir) {
  const out = []
  for (const e of fs.readdirSync(dir, {withFileTypes: true})) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) out.push(...walkRs(p))
    else if (e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

let bad = 0
const rsRoot = path.join(HERE, 'src-tauri', 'src')
for (const f of walkRs(rsRoot).sort()) {
  if (f.includes(`${path.sep}artifact${path.sep}`)) continue
  const lines = fs.readFileSync(f, 'utf8').split('\n')
  const ti = lines.findIndex((l) => l.trim() === '#[cfg(test)]')
  const end = ti >= 0 ? ti : lines.length
  for (let i = 0; i < end; i++) {
    const l = lines[i]
    const t = l.trimStart()
    // 注释里提到路径不算：Rust 行注释、块注释续行、以及 `⚠️` / `⛔` 起头的说明行
    if (t.startsWith('//') || t.startsWith('*') || t.startsWith('/*') || t.startsWith('⚠️') || t.startsWith('⛔')) continue
    for (const [name, pat] of Object.entries(PATTERNS)) {
      if (!pat.test(l)) continue
      if (name === 'resources.json' && f.endsWith('lib.rs')) continue
      if (name === 'tools 目录') {
        const key = relOf(f).split(path.sep).join('/')
        if (TOOLS_ALLOWED[key]) continue
        console.log(`✗ ${relOf(f).split(path.sep).join('/')}:${i + 1}  [${name}]  ${l.trim()}`)
        console.log('     ↑ 如果这处该留着，把它加进 TOOLS_ALLOWED 并写清理由')
        bad++
        continue
      }
      console.log(`✗ ${relOf(f).split(path.sep).join('/')}:${i + 1}  [${name}]  ${l.trim()}`)
      bad++
    }
  }
}

// ── 表之间的一致性（assets.mjs ↔ build.rs）────────────────────────────
for (const item of assets.selfCheck()) { console.log(`✗ assets.mjs 自检：${item}`); bad++ }
for (const item of assets.checkTablesAgree(HERE)) { console.log(`✗ 表不一致：${item}`); bad++ }

console.log(bad === 0 ? '✓ 生产代码里没有裸拼产物落点' : `\n还有 ${bad} 处`)
process.exit(bad ? 1 : 0)
