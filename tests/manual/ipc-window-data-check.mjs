/**
 * 逐页「有没有真数据」探针 —— 第 3 步（前端换 IPC）的验收探针。
 *
 *   node tests\manual\ipc-window-data-check.mjs [CDP端口]     默认 9349
 *
 * ## 它验什么（`cdp-window-check.mjs` 验不到的那一半）
 *
 * `cdp-window-check.mjs` 的 10/10 是「**渲染 + 控制台无非法报错**」—— 换 IPC 之前
 * 那样跑也是全绿，可页面显示的是「连不上本地服务」。所以这一份问的是**内容**：
 * 每一页的正文里有没有**只有后端答得上来**的那些串（工具版本、格式数量、模型状态、
 * 资源库分组名、程序版本…）。
 *
 * 外加一段「直接问 IPC」：从页面上下文里 `invoke` 几条命令，看回包形状对不对。
 * 两段合起来才说明「前端换出口」这件事真的通了 —— 只验一段都可能是假的。
 *
 * ⚠️ **表达式一律写在这个文件里，命令行只传端口。** 内联进 PowerShell 的
 * `node -e "…"` 会被 shell 吃掉引号，报告里全是 `undefined`（踩过两次）。
 * ⚠️ **CDP 的 `Runtime.evaluate` 回包是两层 `result`**（外层 JSON-RPC 壳、内层
 * RemoteObject）—— 见 `window-check.js` 的注释。
 */

const CDP = Number(process.argv[2] ?? 0) || 9349
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/* ── 每一页「有真数据」的判据 ──────────────────────────────────────────────
 * `must` 里任意一条命中即算过（正则），`never` 里命中任何一条即算失败。
 * 注释写的是「这条为什么只可能来自后端」。 */
const PAGES = [
  {
    hash: '#/dashboard',
    name: '总览',
    settle: 3500,
    // 扩展包状态来自 svsep_status / midi_status，工具行来自 get_state
    must: [/音轨分离扩展包/, /(已安装|未安装|检测中)/],
    never: [/读不到工作台状态/],
  },
  {
    hash: '#/convert',
    name: '工程转换',
    settle: 2500,
    /* 「N 种格式可用」的 N 来自 get_state 的 formats[]（LibreSVIP 插件元数据扫出来的）。
       ⚠️ **N 可以是 0**：`tools/` 没补齐（干净 clone）时 LibreSVIP 不在，格式表就是空的 ——
       那是「后端如实答了 0」，不是「连不上」。所以判据只看那一格在不在，
       另加一条：真连不上时 App 会整页换成「读不到工作台状态」。 */
    must: [/(\d+)\s*种格式可用/],
    never: [/读不到工作台状态/],
  },
  {
    hash: '#/video',
    name: '视频解析',
    settle: 2500,
    // 「输出目录」那一格被 get_state 的 paths.downloadDir 灌过（灌没灌到都会有一句 hint）
    must: [/下载选项/, /(默认下载目录|已覆盖设置里的默认下载目录)/],
    never: [/读不到工作台状态/],
  },
  {
    hash: '#/svsep',
    name: '音轨分离',
    settle: 3500,
    // 「服务 / 运行时 / 模型」三条 Stat 全部来自 svsep_status；解压后的容量 7.4 GB 是后端算的
    must: [/运行时/, /(就绪|缺失)/, /(4\.7 GB|7\.4 GB)/],
    never: [/读不到工作台状态/],
  },
  {
    hash: '#/midi',
    name: '人声转 MIDI',
    settle: 3500,
    // 推理方式与许可文案来自 midi_status（`status.device` / `status.license`）
    must: [/推理方式/, /(自动|GPU|CPU)/, /CC BY-NC-SA 4\.0/],
    never: [/读不到工作台状态/],
  },
  {
    hash: '#/audio',
    name: '音频工具',
    settle: 2500,
    // 格式清单来自 get_state 的 audioFormats
    must: [/(WAV|MP3|FLAC)/],
    never: [/读不到工作台状态/],
  },
  {
    hash: '#/lyrics',
    name: '网易云专栏',
    settle: 2500,
    // 登录态来自 state.config.neteaseCookie（没登录时页面必须说得出「未登录」）
    must: [/(未登录|已登录|登录)/],
    never: [/读不到工作台状态/],
  },
  {
    hash: '#/pv',
    name: '文字 PV',
    settle: 6000,
    // 状态条是这一页唯一的输出：编辑器载入完了它会写「已就绪」
    must: [/(编辑器已就绪|已把歌词填进编辑器|正在载入编辑器)/],
    never: [/文件缺失或损坏/],
  },
  {
    hash: '#/resources',
    name: '资源库',
    settle: 2500,
    // 分组与条数来自 get_resources（app/data/resources.json）
    must: [/(个分组|条资源|通过率|更新于)/],
    never: [/读不到工作台状态/, /资源库为空/],
  },
  {
    hash: '#/settings',
    name: '设置',
    settle: 2500,
    /* 「程序版本」是 get_state 里的 version（APP_VERSION），「配置形态」来自 installed。
       ⚠️ 这两格在**「关于」小节**里，而设置页默认停在「外观」—— 所以先点一下那一行。 */
    click: '关于',
    must: [/(1\.\d|beta)/i, /(绿色版|安装版)/],
    never: [/读不到工作台状态/],
  },
]

/* ── 直接问 IPC：这几条覆盖了「状态 / 配置 / 两个扩展包 / 资源库」────────── */
const IPC_CALLS = {
  get_state: 'const s = await invoke("get_state"); return { version: s.version, formats: (s.formats||[]).length, root: s.paths?.root, installed: s.installed }',
  get_config: 'const c = await invoke("get_config"); return { keys: Object.keys(c.config||{}).sort() }',
  svsep_status: 'const s = await invoke("svsep_status"); return { runtimeReady: s.runtimeReady, outputsDir: !!s.outputsDir, models: (s.models?.items||[]).length }',
  midi_status: 'const s = await invoke("midi_status"); return { device: s.device?.mode, modelsReady: s.models?.ready, running: s.running }',
  get_resources: 'const r = await invoke("get_resources", { reload: false }); return { groups: (r.groups||[]).length, items: (r.groups||[]).reduce((n,g)=>n+(g.items||[]).length,0), version: r.version }',
  list_jobs: 'const j = await invoke("list_jobs"); return { jobs: (j.jobs||[]).length }',
}

async function connect() {
  let page = null
  for (let i = 0; i < 30; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${CDP}/json/list`)).json()
      page = Array.isArray(list) ? list.find((x) => x.type === 'page') : null
      if (page) break
    } catch { /* 还没起来 */ }
    await sleep(400)
  }
  if (!page) return null

  const ws = new WebSocket(page.webSocketDebuggerUrl)
  let id = 0
  const pend = new Map()
  ws.addEventListener('message', (ev) => {
    const m = JSON.parse(ev.data)
    const slot = m.id ? pend.get(m.id) : undefined
    if (!slot) return
    pend.delete(m.id)
    clearTimeout(slot.timer)
    if (m.error) slot.reject(new Error(JSON.stringify(m.error)))
    else slot.resolve(m)
  })
  await new Promise((r, j) => {
    ws.addEventListener('open', r)
    ws.addEventListener('error', () => j(new Error('WebSocket 连不上')))
  })

  const send = (method, params = {}) => new Promise((res, rej) => {
    const my = ++id
    /* 音频页那一跳要真读一个文件解码，给它宽一点；其余 8 秒够 */
    const timer = setTimeout(() => {
      if (pend.delete(my)) rej(new Error(`CDP 调用超时：${method}`))
    }, 20000)
    pend.set(my, { resolve: res, reject: rej, timer })
    ws.send(JSON.stringify({ id: my, method, params }))
  })

  const ev = async (expr) => {
    const m = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })
    if (m.exceptionDetails) {
      throw new Error(m.exceptionDetails.exception?.description ?? m.exceptionDetails.text ?? 'eval 失败')
    }
    const remote = m.result?.result
    if (!remote) throw new Error(`Runtime.evaluate 没给 RemoteObject：${JSON.stringify(m).slice(0, 200)}`)
    if (remote.type === 'undefined') return undefined
    return remote.value
  }
  return { ev, close: () => { try { ws.close() } catch { /* 无所谓 */ } } }
}

const cdp = await connect()
if (!cdp) {
  console.log(`连不上窗口的 CDP（端口 ${CDP}）—— 应用没在跑，或者没带 --remote-debugging-port 启动。`)
  process.exit(1)
}

let failed = 0
try {
  console.log('═══ 一、直接问 IPC（页面上下文里 invoke）═══')
  for (const [cmd, body] of Object.entries(IPC_CALLS)) {
    try {
      const r = await cdp.ev(`(async () => { const invoke = window.__TAURI_INTERNALS__.invoke; ${body} })()`)
      console.log(`  ✓ ${cmd.padEnd(16)} ${JSON.stringify(r)}`)
    } catch (e) {
      failed += 1
      console.log(`  ✗ ${cmd.padEnd(16)} ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  console.log('\n═══ 二、逐页看正文里有没有真数据 ═══')
  for (const p of PAGES) {
    await cdp.ev(`location.hash = ${JSON.stringify(p.hash)}; true`)
    /* ⚠️ **等这一页真的上屏再读正文。** 踩过：紧接着 `cdp-window-check.mjs` 跑这一份时，
       上一次那 10 页扫荡的导航还在排队，读到的正文是**别的页**的 —— 于是「总览」那一行
       报的是音频工具的内容（看着像功能坏了，其实只是探针抢跑）。
       判据用页面标题 `.page-title`（它直接来自 App 的 PAGES 表），对上了再等一小会儿
       让这一页自己的 IPC 回来。 */
    let title = ''
    for (let i = 0; i < 40; i++) {
      title = String(
        (await cdp.ev('(document.querySelector(".page-title")?.textContent || "").trim()')) ?? '',
      )
      if (title === p.name) break
      await sleep(200)
    }
    if (title !== p.name) {
      failed += 1
      console.log(`  ✗ ${p.name.padEnd(12)} 导航没到位（当前停在「${title}」）`)
      continue
    }
    await sleep(p.settle)
    if (p.click) {
      /* 小节导航是页面内的按钮（不是路由），要点一下才渲染那一节 */
      await cdp.ev(
        `(() => { const b = [...document.querySelectorAll("button")].find(x => x.textContent.trim() === ${JSON.stringify(p.click)}); if (b) b.click(); return !!b })()`,
      )
      await sleep(600)
    }
    const text = await cdp.ev('(document.body.innerText || "").replace(/\\s+/g, " ")')
    const t = String(text ?? '')
    const hits = p.must.filter((re) => re.test(t)).map(String)
    const bad = (p.never ?? []).filter((re) => re.test(t)).map(String)
    const ok = hits.length > 0 && bad.length === 0
    if (!ok) failed += 1
    console.log(
      `  ${ok ? '✓' : '✗'} ${p.name.padEnd(12)} 命中 ${hits.length}/${p.must.length}` +
        (bad.length ? ` · 不该出现：${bad.join(' ')}` : '') +
        ` · ${t.length} 字`,
    )
    if (!ok) console.log(`      正文片段：${t.slice(0, 200)}`)
  }
} finally {
  cdp.close()
}

console.log(`\n═══ ${failed === 0 ? '全部通过' : `失败 ${failed} 项`} ═══`)
process.exit(failed === 0 ? 0 : 1)
