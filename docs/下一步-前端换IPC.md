# 下一步：前端换出口（第 3 步）

> **这份是给下一轮对话的执行清单。** 背景与设计理由在 `TAURI-IPC-PLAN.md`，
> 这里只讲「现在要做什么、按什么顺序、怎么算做完」。
>
> 生成于 2026-10-04，对应提交 `9979c0d`。

---

## 0. 现状（30 秒读完）

这套东西**已经从「同进程内嵌 axum HTTP 服务」改成「Tauri IPC + asset 协议」**，
**后端侧全部做完并已入库**：

| | |
|---|---|
| 窗口怎么加载前端 | `WebviewUrl::App("index.html")` 加载内嵌资源（`frontendDist: ../../app/web`） |
| 前端怎么调后端 | `invoke('命令名', args)` —— **70 条**，唯一登记处是 `main.rs` 的 `generate_handler!` |
| 本地音视频怎么播 | `convertFileSrc(path)` —— Tauri 内置 asset 协议，Range/206 是自带的 |
| 旧 HTTP 层 | **已删除**：`app/desktop/src/server/`（9 文件 / 5,610 行 / 56 条路由）+ `axum` + `tower-http` |
| 端口 | **概念消失**。17878 / 8891 / 1420 上都没有监听 |
| `--serve` | **已删除**，写它没有任何效果 |

**所以窗口能开、能画、IPC 也通，但页面显示「连不上本地服务」** ——
因为 `app/web-next/src/lib/api.ts` 还在 `fetch('/api/…')`，而那些路由不存在了。
**后端侧一行不用改，只差前端。**

---

## 1. 六件事，按顺序做

### ① 新建 `app/web-next/src/lib/ipc.ts`

`invoke` 的薄包装。要点：

```ts
import { invoke } from '@tauri-apps/api/core'   // 这个包已经在 package.json 里了

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args)
  } catch (e) {
    // 后端回的是 Err(String) —— 一句给人看的中文，直接透出去
    throw new Error(typeof e === 'string' ? e : (e as { message?: string })?.message ?? '命令失败')
  }
}
```

⚠️ **没有 `{ok:true}` 信封了**。IPC 用 `Result<T, E>` 表达成败：成功就是那个 JSON 对象，
失败是 `Err(字符串)`，`invoke` 把它抛成 `Error`。**别再去读 `res.ok`。**

### ② `lib/api.ts` 的 55 个方法逐个换成 `call('命令名', …)`

**方法名与参数保持原样**，页面里的 `await api.xxx()` 一个字都不用改 ——
后端收的就是 `args: Value`（当初这么设计就是为了这一步）。

**70 条命令全清单**（也可以从 `main.rs` 现抠）：

```
allow_path              audio_probe             audio_run               bili_logout
bili_qr_generate        bili_qr_poll            cancel_job              check_resources
convert_collect         convert_inspect         convert_preview         convert_run
get_config              get_job                 get_resources           get_state
job_watch               list_jobs               lyrics_cover            lyrics_get
lyrics_import           lyrics_login_cellphone  lyrics_login_sms        lyrics_logout
lyrics_parse_link       lyrics_save             lyrics_search           lyrics_song
midi_cancel             midi_deps_delete        midi_device_get         midi_device_set
midi_download_stop      midi_models_download    midi_open_output        midi_runtime_download
midi_status             midi_task               midi_transcribe         migrate_legacy_settings
mkdir                   open_path               open_url                pick_paths
preview_clear           preview_fetch           pv_save_chunk           read_bytes
remove_path             set_config              svsep_backend_status    svsep_cancel
svsep_deps_delete       svsep_download_pause    svsep_download_stop     svsep_inference_get
svsep_models_download   svsep_open_output       svsep_runtime_download  svsep_separate
svsep_set_inference     svsep_start             svsep_stop              svsep_status
svsep_task              tools_detect            tools_launch            upload_dropped
video_download          video_parse
```

**⚠️ 别一次改完 55 个再编译。按页改、每改完一页就跑一次真窗口看那一页活没活。**
建议顺序（从简单到复杂）：

```
设置(6) → 总览(3) → 资源库(4) → 文字 PV(5) → 网易云(12) → 音频工具(5)
        → 视频解析(8) → 音轨分离(14) → 人声转 MIDI(15) → 工程转换(9)
```

括号里是该页大概要接几条。**每页改完单独验证一次**，别攒着。

### ③ `lib/useJob.ts`：`EventSource` → `Channel`

后端是 `job_watch` 命令 + `tauri::ipc::Channel<Value>`（`ipc/jobs.rs`）。
前端建一个 Channel 传进去，后端每有更新就推一次。

```ts
import { Channel } from '@tauri-apps/api/core'
const ch = new Channel<Job>()
ch.onmessage = (j) => onUpdate(j)
await call('job_watch', { id, onUpdate: ch })
```

**保留那 700ms 的轮询兜底** —— 它是 `useJob.ts` 现有设计的一部分，不要删。

### ④ 26 处 `localStorage` → 新建 `lib/config.ts` 的 `useConfig()`

* 启动时**一次** `get_config` 拉回来，写回走 `set_config`（防抖合并）；
* **首启调一次 `migrate_legacy_settings`** —— 它把旧的 `qingmu.theme` /
  `qingmu.glassLevel` / `fandiao.audio.settings` / `fandiao.video.settings` /
  `fandiao.convert.options` / `qingmu.pv.lyrics` 搬进 `config.json`。
  **合并规则是「已有的键优先」，所以幂等，多调几次无害**；
* 别删旧键 —— 迁移逻辑靠它们判断「搬没搬过」。

⚠️ **老用户的这些键在旧 origin 下**（`http://127.0.0.1:17878`），
新窗口是 `http://tauri.localhost` —— **两个 origin 的 localStorage 不互通**，
所以迁移只能在前端读到旧 origin 的键时做（这正是 `migrate_legacy_settings`
收一个对象当参数的原因）。

### ⑤ 媒体与文件

| 现在 | 换成 |
|---|---|
| 6 个拼 URL 的助手（`mediaProxyUrl` / `svsepFileUrl` / `midiFileUrl` / `rawUrl` …） | `convertFileSrc(path)` |
| 远端预览（MV 试听） | 先 `preview_fetch(url)` 落本机缓存 → 再 `convertFileSrc(返回的 path)` |
| `FilePick` 的对话框 | `pick_paths`（→ `{path, name, kind}[]`） |
| 拖放（`onDrop` / `DataTransfer`） | `getCurrentWebview().onDragDropEvent()`，拿到 `event.payload.paths`（**真路径**） |
| 拖入的临时落盘 | `upload_dropped(paths)` |

⚠️ **`preview_fetch` 会先下载再播**（一支 1080P 的 MV 要等几秒），但**同一支看第二次是瞬时的** ——
UI 上要有个「缓存中…」的提示，别让用户以为卡住了。详见 `TAURI-IPC-PLAN.md` 第 2.3 节。

### ⑥ 两处结构性改动

* `Pv.tsx` 的 `/api/pv/save`（multipart 上传）→ `pv_save_chunk`
  （`{dir, name, part, total, bytes}`，分块传字节，上限 16 MB/块）；
* `Convert.tsx` 的 `postUpload` **删掉** —— `convert_run` 现在收**本机路径**，不收上传的字节。

---

## 2. 怎么算做完（三条都要过）

```
① 真窗口里 9 页能看到真数据（不再是「连不上本地服务」）
② grep -rn "fetch(" app/web-next/src            → 0 处
③ grep -rn "localStorage" app/web-next/src      → 只剩迁移那一处
```

---

## 3. 命令与工具（都已实测可用）

```powershell
# 编译（唯一入口 —— 直接 cargo build 不会把 exe 复制到根目录）
powershell -ExecutionPolicy Bypass -File app\desktop\build.ps1
#   ⚠️ 前端内嵌进 exe 了，改完前端**也要重跑它**（第一步就是 npm run build）

# 只编后端（省几秒）
powershell -ExecutionPolicy Bypass -File app\desktop\build.ps1 -SkipWeb

# Rust 单测（现在是主验证）
cd app\desktop; cargo test --bins        # 应 93 passed / 0 failed

# 起真窗口 + 两个探针
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9349'
$app = Start-Process -FilePath '.\v-synth-studio.exe' -PassThru
Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
Start-Sleep -Seconds 9
node tests\manual\window-check.js 9349         # origin / IPC / 渲染了多少
node tests\manual\cdp-window-check.mjs 9349    # 10 页冒烟

# 收尾（只能按精确 PID，别按进程名或命令行子串）
Stop-Process -Id $app.Id -Force
Get-Process -Name 'msedgewebview2','msedge' -EA SilentlyContinue |
  Where-Object { $_.MainWindowHandle -eq 0 } | Stop-Process -Force
```

**git 在这台机器上没进 PATH**，用全路径（机器级 PATH 里有 `C:\Program Files\Git\cmd`，
新开的终端可能有，没有就用全路径）：

```powershell
& 'C:\Program Files\Git\cmd\git.exe' -C 'C:\Users\qingm\Downloads\工作站' status -sb
```

---

## 4. 别踩的坑（都真踩过）

| 坑 | 说明 |
|---|---|
| **CDP 的 `Runtime.evaluate` 回包是两层 `result`** | `{"id":1,"result":{"result":{"type":"string","value":…}}}`。写成 `r.result.value` 会**每一项都拿到 `undefined`**，报告却照样打印得整整齐齐，看着像「应用没有 origin」。`window-check.js` 里已经防住了，照它抄 |
| **`git status` 会刷一屏换行警告** | `LF will be replaced by CRLF` —— 仓库有 `core.autocrlf` 设置，**这是提示不是错误**，`git status` 依然是干净的 |
| **`AGENTS.md` 只剩 81 字节余量** | 它在工作区指令的 65,536 字节预算上（现在 65,455）。**往它加内容前先看字节数**，超了尾部会被截断 |
| **改 `.ps1` 要补 BOM** | `write` 工具存出来没有 BOM，PS 5.1 按 ANSI 读中文 → 整篇乱码 → 语法错。`node tests\manual\fix-ps1-bom.mjs --write` |
| **容器里的 JS 别内联进命令行** | 探针的表达式一律写在 `.js` 文件里，命令只传端口 |
| **`migrate_legacy_settings` 的合并规则** | 「已有的键优先」⇒ 幂等。但**别用它覆盖用户当前设置**，那是迁移不是同步 |
| **契约测试与 `--serve` 已退役** | `tests/contract/` 那套（对冻结夹具 diff）整条路失效了，**别去修它** —— 验收收敛到 `cargo test` + 两个探针 |

---

## 5. 别做的事（用户已经拍板）

1. **别保留任何 HTTP 传输层**，也**不许把 `axum` 加回来** —— 页面里没有 HTTP 服务器的位置；
2. **媒体播放一律 `convertFileSrc`**，不许手搓 `register_asynchronous_uri_scheme_protocol`；
3. **测试收敛到 `cargo test`**，不许为了迁就 `.mjs` 再把常驻 HTTP 服务加回来；
4. **不许用 `localStorage` 存配置** —— 一律走 `config.json`；
5. **不许引 `ts-rs`** 之类的构建期类型生成 —— 前端类型手写在 `types.ts`；
6. **编译只能走 `build.ps1`**。
