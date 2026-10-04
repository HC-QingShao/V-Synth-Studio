import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

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
import { ensureConfig } from './lib/config'

const root = document.getElementById('root')
if (!root) throw new Error('找不到 #root 挂载点')

/*
 * ⚠️ **挂载 React 之前先把配置拉回来。**
 *
 * 用户的主题与玻璃等级现在住在 `config.json` 里（不再有浏览器存储），而它们是
 * **第一帧就要对**的东西 —— 晚一步拿到就是「先闪一下跟随系统，再跳成黑暗」。
 * `get_config` 只读一个几百字节的文件，比首屏渲染快得多，等它一下很划算。
 *
 * 真出错也不拦着界面出来：`ensureConfig()` 内部已经把错误报进控制台 / toast，
 * 回落到默认值。启动遮罩（`#boot`）这会儿还盖着，用户看不到这一小段等待。
 *
 * 写成 `.then` 而不是顶层 `await`：构建目标里带不带 top-level await 取决于
 * Vite / esbuild 的默认 target，`await` 一旦不被认就是**构建失败**，
 * 而这里等的东西根本不需要它。
 */
void ensureConfig()
  .catch(() => {})
  .then(() => {
    createRoot(root).render(
      <StrictMode>
        <App />
      </StrictMode>,
    )
  })
