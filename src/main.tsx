import {StrictMode} from 'react'
import {createRoot} from 'react-dom/client'
import {getCurrentWindow} from '@tauri-apps/api/window'

/**
 * 样式表顺序**不能改**：
 *   1. 库自己的 token + 组件样式（`tokens.css` + `components.css` 拼成，约 197KB）
 *   2. 我们的外壳样式（只排布局，**没有一行玻璃**）
 *
 * 库的 token（`--lg-label` / `--lg-bg` / `--lg-space-*` / `--lg-radius-*` …）
 * **全部挂在 `[data-lg-theme="light|dark"]` 下**，`:root` 上没有裸定义 ——
 * 所以必须引它，否则我们的 `var(--lg-*)` 全解析不出来。
 *
 * README 的要求：「样式表在应用入口引一次就够了。」
 */
import '@ttqtt/liquid-glass-react/style.css'
import './index.css'

import App from './App'
import {ensureConfig, onConfigChange} from './lib/config'
import {syncGlassLevel} from './lib/useGlass'

/**
 * 玻璃等级跟着配置走。订阅挂在模块级、`ensureConfig()` 之前 ——
 * 启动那次 `get_config` 落定时也要触发一次，晚一步第一帧就是默认档。
 */
onConfigChange(syncGlassLevel)

const root = document.getElementById('root')
if (!root) throw new Error('找不到 #root 挂载点')

/* 窗口在 `tauri.conf.json` 里 `visible: false`，首帧不闪主题靠这里等 `get_config`
   落定后再显示。超时兜底是必需的：`get_config` 卡死时窗口不能永远不可见。
   `show()` 要 `core:window:allow-show` 权限（见 `capabilities/default.json`）。 */
const REVEAL_TIMEOUT_MS = 6000
let revealed = false

function reveal() {
    if (revealed) return
    revealed = true
    createRoot(root as HTMLElement).render(
        <StrictMode>
            <App/>
        </StrictMode>,
    )
    // 先打入场标记（App 提交后侧栏与内容区才会做入场，见 index.css）
    document.documentElement.dataset.enter = 'out'
    /* ⛔ **不能用 `requestAnimationFrame` 等一帧再显示**：窗口 `visible: false`
    时不参与合成，rAF 回调**不会触发**，`show()` 就永远等不到 —— 表现是进程活着、
    窗口始终不出现。`setTimeout` 也有被后台节流的风险，所以这里直接调。 */
    void getCurrentWindow().show()
}

window.setTimeout(reveal, REVEAL_TIMEOUT_MS)
void ensureConfig()
    .catch(() => {
    })
    .then(reveal)
