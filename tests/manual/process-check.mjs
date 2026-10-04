/**
 * 一次性排查：应用运行时**到底有没有进程**。
 *
 *   node tests\manual\process-check.mjs [CDP端口]
 *
 * 回答三个问题（省得靠猜）：
 *   1. 应用自己是不是只有 `v-synth-studio.exe` 一个进程（+ WebView2 的渲染进程）
 *   2. 有没有 node / python / ffmpeg / yt-dlp 子进程
 *   3. 有没有在听任何端口（旧架构会有 17878，新架构应该没有）
 *
 * 需要窗口已经用 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>` 起着。
 *
 * ⚠️ **问应用的那些表达式写在 `window-check.js` 里，别内联进命令行。** 踩过两次同一个坑：
 * 把多行 JS 内联进 PowerShell 的 `node -e "…"`，`"` 会被外层 shell 吃掉。不过要注意 ——
 * **2026-10-04 复查发现，当时报告的 `origin: undefined` 其实另有真因**：
 * CDP 的 `Runtime.evaluate` 回包是**两层 `result`**
 * （`{"id":1,"result":{"result":{"type":"string","value":…}}}`），外层是 JSON-RPC 响应壳、
 * 内层才是 RemoteObject。写成 `r.result.value` 会**每一项都拿到 `undefined`**，
 * 而报告照样打印得整整齐齐，看着像「应用没有 origin / 没有 IPC」——**这个坑白查了一轮**。
 * 所以文件里那条 `ev()` 两个都防住了：只传路径（防 shell 吃引号）+ 正确剥两层（防假 undefined）。
 */

import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const WINDOW_CHECK = join(HERE, 'window-check.js')
const CDP = Number(process.argv[2] ?? 0) || 9337

console.log('进程侧的检查请用 PowerShell 跑（本仓库的测试都跑在 Windows 上）：')
console.log('')
console.log('  # 1) 带调试端口起窗口（复制粘贴整段）')
console.log('  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=\'--remote-debugging-port=' + CDP + '\'')
console.log('  $app = Start-Process .\\v-synth-studio.exe -PassThru')
console.log('  Remove-Item Env:\\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS')
console.log('  Start-Sleep -Seconds 9')
console.log('')
console.log('  # 2) 进程与端口')
console.log('  Get-Process v-synth-studio | Select-Object Id,@{n=\'MB\';e={[math]::Round($_.WorkingSet64/1MB,1)}}')
console.log('  (Get-Process msedgewebview2 -EA SilentlyContinue).Count   # WebView2 渲染进程，正常')
console.log('  foreach ($n in \'node\',\'python\',\'ffmpeg\',\'yt-dlp\',\'libresvip-cli\') { "$n : " + @(Get-Process $n -EA SilentlyContinue).Count }')
console.log('  Get-NetTCPConnection -State Listen -EA SilentlyContinue | Where-Object { $_.LocalPort -in 17878,8891,1420 }')
console.log('')
console.log('  # 3) 窗口侧（用 CDP 问它自己）')
console.log('  node tests\\manual\\window-check.mjs ' + CDP)
console.log('')
console.log('  # 4) 收尾 —— 只按精确 PID 杀，别按进程名/命令行子串')
console.log('  Stop-Process -Id $app.Id -Force')
console.log('  Get-Process msedgewebview2 -EA SilentlyContinue | Where-Object { $_.MainWindowHandle -eq 0 } | Stop-Process -Force')
console.log('')

/* 如果窗口已经在跑，顺手用一次（走文件、不内联，见上面的告诫）。 */
if (existsSync(WINDOW_CHECK)) {
  try {
    const out = execFileSync(process.execPath, [WINDOW_CHECK, String(CDP)], { encoding: 'utf8' })
    console.log('窗口已经在跑，窗口侧的结果：')
    console.log(out.trim().split('\n').map((l) => '  ' + l).join('\n'))
  } catch {
    console.log(`（连不上 ${CDP} 端口的窗口 —— 还没起，或者没带 --remote-debugging-port）`)
  }
}

