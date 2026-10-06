#!/usr/bin/env node
/**
 * 补齐两款「不入库、但打包必需」的产物 —— `pnpm tauri dev` / `pnpm tauri build` 的入口。
 *
 *   1. tools/                 → fetch_tools.mjs        （bundle.resources 的目录映射）
 *   2. public/vendor/jizura/  → fetch_jizura_fonts.mjs （public/ → dist/ → 嵌进 exe）
 *
 * ⚠️ **必须在编译/开发之前跑**：缺产物时 Tauri 与 Vite 都不报错 —— 前者静默少打包，
 *   后者只是少字体。所以由 `beforeDevCommand` / `beforeBuildCommand` 自动接上，
 *   不靠人记得手工跑。
 *
 * 两种模式：
 *   `prepare-assets.mjs dev`      缺件只警告 —— `tauri dev` 不查产物，缺了不该拦住开发；
 *   `prepare-assets.mjs`（build） 缺件退出 1 —— 打包少一件是静默的，必须拦住。
 * `VSS_FETCH_OPTIONAL_TOOLS` / `VSS_FETCH_OPTIONAL_FONTS` 可单独放行某个来源（国内直连
 * GitHub / Google Fonts 常不通，编码期又用不到 → 用它换一个能起来的开发环境）。
 */

import process from 'node:process'
import {ensureFetchProxy, say} from './lib.mjs'
import {main as fetchTools} from './fetch_tools.mjs'
import {main as fetchJizura} from './fetch_jizura_fonts.mjs'

// 代理要在**进程启动时**设好（Node 的 fetch 只在那时读环境变量），而 jizura 那一步
// 在境内要靠本机代理。装代理的人很少走「直接跑 fetch_jizura」，正常都从这儿进来 ——
// 所以检测与重跑放在入口，而不是只放在 fetch_jizura 自己那儿。
ensureFetchProxy()

const STEPS = [
  {name: 'tools', what: '工具（ffmpeg / yt-dlp / LibreSVIP / onnxruntime）', run: () => fetchTools([])},
  {name: 'jizura', what: 'JIZURA 页面与字体', run: () => fetchJizura([])},
]

/** 允许失败的来源 —— **只为方便开发**：国内直连 GitHub / Google Fonts 常不通，
 *  而这两件编码期都用不到（tools 只在打包时查，jizura 只在「文字 PV」页用）。
 *  ⚠️ `tauri dev` 不查产物，所以「开发能起来」不等于产物齐 —— 发布前必须补齐。 */
const OPTIONAL = {
  tools: 'VSS_FETCH_OPTIONAL_TOOLS',
  jizura: 'VSS_FETCH_OPTIONAL_FONTS',
}

function optionalSources() {
  const raw = (process.env[OPTIONAL.tools] || process.env[OPTIONAL.jizura] || '').trim().toLowerCase()
  const all = raw === '1' || raw === 'all'
  const picked = new Set(raw.split(',').map((s) => s.trim()).filter(Boolean))
  return (name) => all || picked.has(name)
}

// dev 模式全放行：编译期不查产物，缺件的代价只是「少个功能」，而拦住开发代价更大。
const DEV = process.argv[2] === 'dev'
const envSoft = optionalSources()
const soft = (name) => DEV || envSoft(name)
const hardFailed = []
const softFailed = []

for (const step of STEPS) {
  say(`\n▶ 补齐${step.what}`)
  let code = 1
  try { code = await step.run() } catch (e) { say(`✗ ${e.message}`); code = 1 }
  if (code !== 0) (soft(step.name) ? softFailed : hardFailed).push(step.name)
}

if (DEV) say('\n（dev 模式：产物缺失只警告，不拦住开发）')
if (softFailed.length) {
  say(`\n⚠️  ${softFailed.join('、')} 没补齐 —— 已放行，继续。`)
  say('    开发能跑，但那部分功能会缺件；发布前必须补齐（见「如何编译打包.md」）。')
}
if (hardFailed.length) {
  say(`\n✗ ${hardFailed.join('、')} 没补齐 —— 这是打包必需的，编译不会报错但产物会缺文件。`)
  say('  重新连上下游 / 走代理再跑；只想先把开发环境跑起来，就设对应的 VSS_FETCH_OPTIONAL_* 变量。')
  process.exit(1)
}
