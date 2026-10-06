import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import process from 'node:process'
import { fileURLToPath, URL } from 'node:url'

/**
 * ⚠️ **为什么需要这个插件：lightningcss 会删掉标准的 `backdrop-filter`。**
 *
 * Vite 8（rolldown 版）用 lightningcss 压缩 CSS，它按浏览器目标自动裁剪厂商前缀。
 * 这个项目里它**裁错了**：把标准属性 `backdrop-filter` 删掉、只留
 * `-webkit-backdrop-filter`。而 WebView（WebKitGTK / WebView2 都一样）**只认标准属性名**：
 *
 *     CSS.supports('backdrop-filter','blur(1px)')         → true
 *     CSS.supports('-webkit-backdrop-filter','blur(1px)') → false
 *
 * 于是库的样式表里明明写着 `backdrop-filter: blur(30px)`，
 * 浏览器里 `getComputedStyle` 却报 `none`，内容面板变成一片不透明的底色。
 * **查源码怎么查都是对的 —— 问题只在构建产物里**，这类 bug 极难自查。
 *
 * 试过、**都不行**的办法（别再试一遍）：
 *   - `build.target: 'chrome120'`     → 管不到 CSS 这一步
 *   - `css.lightningcss.targets`      → 无效（实测产物里标准版仍是 0 处）
 *   - `build.cssMinify: 'esbuild'`    → Vite 8 不再自带 esbuild，直接构建失败
 *   - `build.cssMinify: false`        → 有效，但产物从 120KB 涨到 220KB（+100KB）
 *
 * 实测确认**只有 `backdrop-filter` 这一个属性被删错**
 * （`-webkit-mask-composite`、`-webkit-user-drag`、`-webkit-tap-highlight-color`
 * 的标准版本来就不存在，不需要补）。
 *
 * 所以：保留默认压缩（体积不变），只把这一条补回去。
 * `enforce: 'post'` + `generateBundle` 保证跑在压缩**之后**。
 */
function restoreStandardBackdropFilter(): Plugin {
  return {
    name: 'restore-standard-backdrop-filter',
    enforce: 'post',
    generateBundle(_options, bundle) {
      for (const file of Object.values(bundle)) {
        if (file.type !== 'asset' || !file.fileName.endsWith('.css')) continue
        const src =
          typeof file.source === 'string' ? file.source : new TextDecoder().decode(file.source)
        /**
         * 按**声明块**处理，块里已经有标准 `backdrop-filter` 就整块跳过。
         *
         * 两个必须注意的点：
         *
         * 1. **不能整文件盲目替换。** 库里有 `[data-transparency="opaque"]` 这类规则，
         *    原本就是 `-webkit-backdrop-filter:none`（故意关掉模糊的）。
         *    复制成标准属性没错，但如果**插错位置**就会把别的规则弄坏 ——
         *    插到 `var()` 声明前面去，`.lg-backdrop`（玻璃面自己的模糊，
         *    走 `backdrop-filter:var(--lg-backdrop,…)`）就变成先 `none` 再 `var(...)`，
         *    **玻璃面直接失去模糊**（顶栏 `backdrop=none`）。
         *
         * 2. **负向后顾 `(?<![\w-])` 是必须的。** 库里有自定义属性
         *    `--lg-backdrop`，它的名字以 `backdrop-filter` 结尾；不加后顾
         *    会把 `--lg-backdrop:` 也当成属性来插，产出一堆垃圾。
         *
         * 按 `{...}` 分块 + 检查块内是否已有标准属性，两个问题一起解决：
         * 只在「有前缀版、没标准版」的块里补，且插在该前缀版**前面**。
         */
        const fixed = src.replace(/\{[^{}]*\}/g, (block) => {
          if (/(?<![\w-])backdrop-filter\s*:/.test(block)) return block
          return block.replace(
            /(?<![\w-])-webkit-backdrop-filter\s*:\s*([^;}]+)/g,
            (whole, value) => `backdrop-filter:${value};${whole}`,
          )
        })
        if (fixed === src) continue
        const added = (fixed.match(/(?<!-)\bbackdrop-filter\s*:/g) ?? []).length
        this.warn(`补回标准 backdrop-filter：${file.fileName}（共 ${added} 条）`)
        file.source = fixed
      }
    },
  }
}

const host = process.env.TAURI_DEV_HOST

export default defineConfig(() => ({
  base: '/',
  plugins: [react(), restoreStandardBackdropFilter()],

  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },

  // Vite 侧针对 Tauri 的设置：别遮住 cargo 的报错
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: {
      // src-tauri 由 cargo 管，别让 Vite 为它重构建
      ignored: ['**/src-tauri/**'],
    },
  },

  build: {
    // `public/` 由 Vite 原样拷进 dist，所以 img/ 与 vendor/jizura/ 不需要 emptyOutDir=false
    chunkSizeWarningLimit: 1500,
  },
}))
