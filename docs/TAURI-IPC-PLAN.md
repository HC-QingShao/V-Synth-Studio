# 减法重构：剔除 Axum，全面回归 Tauri 原生生态

> 状态：**方案，未动代码**（2026-10-04 第二版）
> 取代：上一版 `TAURI-IPC-PLAN.md`（双通道 + 自定义协议 + 临时迁移口）—— **那版作废**，
> 它把系统推向「HTTP + IPC + 自定义协议」三套传输层，与「做减法」的目标相反。
> 本版原则：**技术栈归一（只有 IPC）、物理删除优先、不留过渡态**。

---

## 0. 一句话结论

现在的 `app/desktop/src/server/`（**5,610 行 / 9 个文件**）里，
**只有约 20 个业务动作是真需求**，其余 **36 个端点全是「因为前端加载的是 `http://127.0.0.1`，
拿不到 Tauri 能力」而捏出来的替身** —— 静态文件伺服、文件对话框、上传落地、SSE、
范围请求、健康探针、端口等待，全部由 Tauri 原生设施替代。

**净删除量估算：约 −20,500 行，新增约 +2,600 行 ⇒ 净减 ≈ 18,000 行**（详见第 1 节）。

---

## 1. 直接删除清单（物理删除，不留开关）

### 1.1 整目录删除

| 路径 | 行数 | 为什么能删 |
|---|---|---|
| `app/desktop/src/server/` | **5,610**（9 文件） | 整个 HTTP 传输层。业务逻辑搬进 `ipc/`，其余全部由 Tauri 原生替代 |
| `tests/contract/` | **5,224**（19 文件） | 契约测试退役（约束 3）：夹具是「HTTP 响应」的冻结基准，传输层没了它就没有对照对象；改为 Rust 单测 |
| `tests/manual/*.mjs` | **~9,700** | 除下面保留的 2 个外全删（它们全靠 `--serve` + 无头 Edge 打 8891） |

`tests/manual/` 里**保留 2 个、其余删**：

| 保留 | 行数 | 为什么留 |
|---|---|---|
| `fix-ps1-bom.mjs` | 57 | 与传输层无关：`write`/`edit` 工具会丢 `.ps1` 的 BOM，这是硬规矩（AGENTS.md 零·7） |
| `check-resources.mjs` | ~110 | 纯 Node 脚本，查 `resources.json` 的外链死活（第六节规则），从不碰应用端口 |

删除：`next-smoke.mjs`(16.9 KB)、`glass-probe.mjs`(23.8 KB)、`slider-probe.mjs`、
`topbar-probe.mjs`、`pv-verify.mjs`、`pv-export-probe.mjs`、`pv-import-probe.mjs`、
`storage-verify.mjs`、`lyrics-import-verify.mjs`、`convert-samples.mjs`、`dump-resources.mjs`、
`verify.mjs`(在 `tests/contract/`)、`out/`(探针截图)。

### 1.2 文件内删除

| 文件 | 删什么 | 行数 |
|---|---|---|
| `app/desktop/src/main.rs`（477→约 300） | `serve()`、`pick_port()`、`port_is_free()`、`free_port()`、`wait_for_port()`、`STARTUP_TIMEOUT_SECS`、`PREFERRED_PORT`、`tokio::async_runtime::spawn(serve…)`、`WebviewUrl::External` | **−180** |
| `app/web-next/src/lib/api.ts`（1067→约 180） | `request/get/post/postForm/postBinary` **全部** fetch 助手（1-139 行）；`api` 对象里 55 个 fetch 实现删掉，改为 `invoke` 薄壳（写在 `ipc.ts`）；`FsListRaw`/`toEntries` 归一化层删掉（Rust 直接回扁平数组） | **−420** |
| `app/web-next/src/lib/useJob.ts`（112→约 60） | `EventSource` + `fetch` 轮询整段；改成 `Channel` + 事件 | **−50** |
| `app/web-next/src/components/FilePick.tsx`（261→约 120） | `api.fsPick` / `api.fsUpload` 两条上传逻辑；改成 Tauri 原生 dialog + 全局 drop 事件（拿到的是**真路径**） | **−140** |
| `app/web-next/src/components/Preview.tsx` | `mediaProxyUrl` 相关说明与 loading 分支；改 `convertFileSrc` | **−20** |
| `app/desktop/Cargo.toml` | `axum`、`tower-http` 两个依赖（`tokio` **留**：svsep/midi 的子进程与下载还在用） | −2 行（但少两棵依赖树） |
| `app/desktop/build.ps1` | `--serve` 相关的提示与 echo；`app/web` 复制步骤保留（`frontendDist` 仍指它） | −15 |
| `AGENTS.md` / `docs/FRONTEND.md` | 「56 条路由」「`--serve` 模式」「每请求从磁盘读」「端口 17878 / localStorage 按 origin 隔离」等已失效段落 | −60（文档） |

### 1.3 localStorage（26 处，全部拆除）

| 文件 | 键 | 改法 |
|---|---|---|
| `App.tsx` | `qingmu.theme` | → `config.theme` |
| `lib/useGlass.ts` | `qingmu.glassLevel`、`qingmu.globalGlass` | → `config.glassLevel`（**顺带删掉那个 3 档时代的兼容分支**） |
| `components/Glass.tsx` | `qingmu.glass` | → 删（材质由 `glassLevel` 派生，这个键已经冗余） |
| `pages/Audio.tsx` | `fandiao.audio.settings` | → `config.audio` |
| `pages/Video.tsx` | `fandiao.video.settings` | → `config.video` |
| `pages/Convert.tsx` | `fandiao.convert.options` | → `config.convert` |
| `pages/Midi.tsx` | 输出目录键 | → `config.midi.outDir` |
| `pages/Lyrics.tsx` ↔ `pages/Pv.tsx` | `qingmu.pv.lyrics` 交接 | → `config.pv.pendingLyrics`（**跨页交接不再依赖同源存储**，这本来就是它最脆的地方） |

> ⚠️ **`usage` 迁移**：老用户的这些键在新 origin 下读不到（`tauri.localhost` ≠ `127.0.0.1:17878`）。
> 处理方式是**前端首启一次性上缴**（`migrate_legacy_settings`）：在本 origin 读得到就读
> （升级安装时 WebView2 的用户数据目录是同一个，旧 origin 的值**仍在**）、读不到就用默认值。
> **不需要任何 HTTP**，就是一次 `invoke`。

### 1.4 合计

| | 行数 |
|---|---|
| 删除 | ≈ **−20,500**（其中 `server/` 5,610、`tests/` 14,900 已计入 1.1；上面 1.2/1.3 是重叠部分的分项口径） |
| 新增（`ipc/` + 协议/缓存/拖放/配置四件新事） | ≈ **+2,600** |
| **净减** | **≈ 18,000 行** |

---

## 2. `tauri::command` 的映射与收敛

### 2.1 56 个端点里，36 个是「为绕过 Tauri 限制而捏造的」

| 捏造的端点 | 条数 | 它替代了什么 Tauri 原生能力 |
|---|---|---|
| `GET /api/fs/raw` | 1 | **asset 协议**（Range 206 内置） |
| `POST /api/fs/pick` | 1 | **`tauri-plugin-dialog`**（`open` / `save` / 选目录） |
| `POST /api/fs/upload` | 1 | **原生拖放事件**（`DragDropEvent::Drop { paths }` **直接给真路径**，不需要「上传字节换路径」） |
| `GET /api/{svsep,midi}/task/{id}/file/{name}` | 2 | asset 协议 |
| `GET /api/media/proxy` | 1 | 见 2.3（改成「后端落缓存文件 + asset 协议」） |
| `GET /api/jobs/{id}/stream`（SSE） | 1 | **`tauri::ipc::Channel`**（约束下唯一正确的流式原语） |
| `POST /api/{convert/run-upload,convert/preview-upload,svsep/separate}` | 3 | IPC 直接传字节（`Uint8Array` ↔ `Vec<u8>`） |
| `POST /api/pv/save`（分块） | 1 | IPC 分块（保留 offset/total 语义，避免整段进内存） |
| `GET /api/health` | 1 | 端口探针的产物，**端口没了它就没意义** |
| 静态文件 fallback | 1 | **`frontendDist`**（Tauri 自己伺服，还带 gzip 与缓存策略） |
| `GET/POST /api/config` | 2 | 保留但**语义变了**：不再是「浏览器读不到文件」的替身，而是唯一配置源 |
| `GET /api/fs/{roots,list,mkdir,delete}` | 4 | 删（系统对话框自带常用位置/新建文件夹；`delete` 用 dialog+`remove_file`） |
| `GET /api/state` | 1 | 与 `config` + `tools_status` + `svsep_status` 合并 |
| 「响应包一层 `{ok:…}`」 | — | IPC 自带成败语义（`Result<T, E>`） |
| `--serve` / `pick_port` / `wait_for_port` / 17878 | — | 整个端口概念消失 |

### 2.2 真正保留的业务 Command（20 个）

**规则：command 只做「解参数 → 调领域函数 → 回值」，领域逻辑全部在 `ipc/` 之外的现有模块里**
（`libresvip.rs` / `media.rs` 里那部分 / `lyrics.rs` / `bili.rs` / `ytdlp.rs` / `audio.rs` /
`svsep.rs` / `midi_transcribe.rs` / `platform.rs` / `data.rs` —— 这些文件**一行都不搬**）。

| # | Command | 直接调用的领域函数（现成） | 说明 |
|---|---|---|---|
| 1 | `get_config()` | `load_config(&writable)` | 启动拉一次；含全部用户偏好 |
| 2 | `set_config(patch)` | `save_config(&writable, …)` | 局部更新 |
| 3 | `get_state()` | `probe_cached(false)` + 格式表 + `svsep::status` | 首屏聚合（**合并**旧 `state`+`health`+`tools/detect`） |
| 4 | `pick_paths({mode})` | `platform::open_file_dialog()`（**Rust 侧调 Tauri dialog**）→ `asset_protocol_scope().allow_*()` | 打开/保存/选目录一个命令 |
| 5 | `open_path({path})` / `open_url({url})` | `platform::open_url()` / `reveal_in_explorer()` | 合并旧 `fs/open` + `fs/reveal` |
| 6 | `convert_inspect({path})` | `libresvip::inspect()` | |
| 7 | `convert_preview({inputs,toFormat})` | `libresvip::preview()` | |
| 8 | `convert_run({…})` | `libresvip::convert()`（`spawn_blocking`） | 直接吃**本地路径**，上传版随之消失 |
| 9 | `video_parse({url})` | `bili::parse()` / `ytdlp::parse()` | |
| 10 | `video_download({…})` | `media` 下载链（`bili`/`ytdlp` + `ffmpeg` 合流） | |
| 11 | `audio_probe({input})` | `audio::probe()` | |
| 12 | `audio_run({…})` | `audio::run()` | |
| 13 | `audio_waveform({path,buckets})` | 新增 ~60 行：`ffmpeg -f f32le` 读出后取包络 | **替代**「前端 `fetch` 整个 wav 再 `decodeAudioData`」——省一次全量传输 |
| 14 | `bili_qr_generate()` / `bili_qr_poll({key})` / `bili_logout()` | `bili::qr_generate` / `qr_poll` / 清配置 | 3 条 |
| 15 | `lyrics_search/get/parse_link/import/save/cover/song` + `lyrics_login_sms/cellphone/logout` | `lyrics.rs` 对应函数 | 10 条，**纯搬运** |
| 16 | `svsep_status()` | `Svsep::status()`（含 download 进度） | **合并**旧 `status`+`backend/status`+`system-stats` |
| 17 | `svsep_start/stop/*_download/pause/stop/deps_delete` | `Svsep::*` | 7 条 |
| 18 | `svsep_separate({path,engine})` | `Svsep::separate()`（**收本地路径**，不再 multipart 上传） | 上传版消失 |
| 19 | `midi_status/device_set/models_download/runtime_download/deps_delete/transcribe/cancel` | `midi_transcribe.rs` 对应函数 | 7 条 |
| 20 | `job_watch({id}, on_event: Channel<Job>)` | `JobTable`（`simple.rs` 里那套移进 `ipc/jobs.rs`） | SSE 的替代 |

**另外 3 个横切动作**（不占 command 数量）：`app_cache_dir()` 下的**预览缓存**
（`preview_fetch({url,kind})` + `preview_clear()`）。

> 合计 **≈ 47 个 command**，且**一半以上是纯搬运**（Lyrics 10 条、Svsep 7 条、Midi 7 条）。
> 旧 `server/*.rs` 里那些 `Json<Value>` 拼装、路由注册、`ApiError`、body limit、
> CORS/trace 中间件**全部消失**。

### 2.3 媒体：`convertFileSrc` 的边界，以及为什么它反而更简单

asset 协议只服务**本地文件**，所以：

| 场景 | 做法 |
|---|---|
| 本地素材（音频页/分离页/扒谱页） | 用户选的就是本地路径 → 直接 `convertFileSrc(path)` 给 `<video>/<audio>`；**Range 由 asset 协议内置**（`tauri-2.12.0/src/protocol/asset.rs:98-116` 就是拿 `http_range` 实现的 206，生产验证过） |
| 分离/扒谱产物 | 本来就是本地文件 → 同上（旧代码还要拼 `/api/svsep/task/{id}/file/{name}`） |
| **预览远端 MV** | ⚠️ 唯一需要改行为的地方：远端直链**不能**喂 `convertFileSrc`。改成 `preview_fetch({url})` 把流落到 `app_cache_dir()/preview/`（带进度，可取消，LRU 上限 1 GB），**播的是本地缓存文件**。用户体验是「点预览 → 转几秒 → 能拖动进度条」，而旧实现那个「代理直链」在 B 站本来就常 403/限速；且从今往后**同一支 MV 只看一次就落地了** |

**关键实现细节（已核实，不是推测）**：asset 协议默认 scope 只允许配置里的几个目录，
用户随便挑的 `D:\曲库` **不在里面**。正解**不是**手搓协议，而是运行时扩 scope：

```rust
// tauri-2.12.0/src/scope/fs.rs:351 / :370 ，Manager::asset_protocol_scope() 见 src/lib.rs:789
app.asset_protocol_scope().allow_directory(dir, true)?;   // 用户在对话框里选了哪个目录
app.asset_protocol_scope().allow_file(path)?;             // 或具体文件
```

于是 `pick_paths` / `preview_fetch` 里选完就 `allow_*`，`convertFileSrc` 立刻可用 ——
**零自定义协议代码**。同时 `Cargo.toml` 要开 `tauri` 的 `protocol-asset` feature，
`tauri.conf.json` 开 `app.security.assetProtocol.enable = true`（scope 交给运行时填）。

### 2.4 拖放：删掉的那一层最厚

现在的链条是「HTML5 drop → `File` → `fetch('/api/fs/upload')` 传字节 → 后端落临时文件 →
换回一个路径」。因为页面没有 IPC，才必须这么绕。

改完：`main.rs` **删掉 `disable_drag_drop_handler()`**（它是为「页面拿不到 IPC、只能靠
HTML5 拖放」而关的），让 Tauri 接管 —— WebView 会收到
`DragDropEvent::Drop { paths, position }`（`tauri-2.12.0/src/webview.rs:745`，
**变体里直接就有 `paths`**）。

```
拖文件进窗口 → 前端监听 drop 事件 → 拿到真路径 → 直接喂给 convert/svsep/midi
```

`/api/fs/upload`、`fs_upload` 的临时目录逻辑、`postBinary`、`FormData` 全部删除。
音轨分离也不再需要 multipart —— 它本来就是「拿本地文件路径去跑 Python 子进程」。

---

## 3. `lib/api.ts` 清道夫计划

### 3.1 分层

```
lib/ipc.ts        新：唯一出口。invoke 包装 + 超时 + 错误规范化 + 事件订阅
lib/types.ts      手写收敛（约束 5：不引 ts-rs），只留 IPC 真正回的形状
lib/config.ts     新：配置读写 + useConfig() 钩子（替代 26 处 localStorage）
lib/api.ts        删 → 内容并入 ipc.ts（保留文件名会让 9 个页面的 import 少改一次；
                  但 **fetch 助手一行不留**）
```

`ipc.ts` 的形状（让 9 个页面**几乎不用改**）：

```ts
const call = <T>(cmd: string, args?: Record<string, unknown>) =>
  invoke<T>(cmd, args).catch((e) => { throw new Error(typeof e === 'string' ? e : e?.message ?? '命令失败') })

export const api = {
  state:        () => call<AppState>('get_state'),
  config:       () => call<Config>('get_config'),
  saveConfig:   (patch: Partial<Config>) => call<Config>('set_config', { patch }),
  convertRun:   (args: ConvertRun) => call<{ jobId: string }>('convert_run', { args }),
  theFileUrl:   (p: string) => convertFileSrc(p),        // 取代 6 个拼 URL 的助手
  watchJob:     (id: string, onUpdate: (j: Job) => void) => watchJob(id, onUpdate),
  // …其余同名同参数
}
```

**要改的页面代码量**：只有「拼 URL 的 6 处助手」和「26 处 localStorage」，
其余 9 个页面里的 `await api.xxx()` 调用**一个字都不用动**。

### 3.2 6 个 URL 助手 → 1 个

| 现在 | 改完 |
|---|---|
| `mediaProxyUrl(u, src)` | `api.theFileUrl(cachePath)`（缓存文件路径由 `preview_fetch` 回） |
| `svsepFileUrl(id, name)` | `api.theFileUrl(outputsDir + name)` |
| `midiFileUrl(id, name)` | 同上 |
| `Audio.tsx` 的 `rawUrl(path)` | 直接 `api.theFileUrl(path)`（波形改走 `audio_waveform` 命令，这个连 URL 都不用了） |
| `Pv.tsx` 的 `/api/pv/save?…` | `call('pv_save_chunk', { name, offset, total, bytes })` |
| `Convert.tsx` 的 `postUpload` | 删（`convert_run` 收路径） |

### 3.3 配置钩子取代 localStorage

```ts
// lib/config.ts —— 26 处 localStorage 的统一替身
export function useConfig() { /* 启动 invoke('get_config')，写回走 set_config（防抖合并） */ }
```
启动时序变成：`App.tsx` 挂载 → `Promise.all([get_config(), get_state()])` → 渲染。
`boot.ts` 的「1.2 秒兜底揭遮罩」保留（`get_state` 仍要探测外部工具）。

---

## 4. 实施步骤（每一步都是物理删除，无过渡态）

> 原则：**先删后加**。每一阶段结束时应用**能跑**，且比上一阶段**更小**。
> 不允许出现「HTTP 与 IPC 并存」的中间提交。

### 第 1 步：砍端口与静态伺服（删，不加）

1. `tauri.conf.json`：`build.frontendDist: "../../app/web"`、`devUrl: "http://localhost:1420"`（Vite dev）。
2. `main.rs`：删 `serve/pick_port/port_is_free/free_port/wait_for_port`、
   `WebviewUrl::External` → `WebviewUrl::App("index.html")`；删 `--serve` 分支。
3. 删 `server/simple.rs` 里的 `static_files`、`fs_raw`、`health`。
4. `build.ps1`：去掉 `--serve` 提示；`npm run watch` 改为 `npm run dev`（Vite 热更新）。

**验收**：窗口能开、9 页能渲染、**17878 上没有任何监听**、`--serve` 参数已无效。

### 第 2 步：砸掉 `server/` 的壳，业务落进 `ipc/`

1. 新建 `src/ipc/{mod,jobs,fs,convert,media,lyrics,svsep,midi,tools,config}.rs`，
   把 `server/*.rs` 里**业务那几行**搬过去（`spawn_blocking`、进度、领域调用），
   **丢掉**路由、`Json<Value>` 拼装、`ApiError`、`DefaultBodyLimit`、CORS/trace。
2. `main.rs`：`app.manage(AppState)` + `.invoke_handler(tauri::generate_handler![…])`。
   ⚠️ 顺带修好 `RunEvent::Exit` 里那段死代码（现在 `try_state` 永远 `None`，
   因为 `AppState` 从没 `manage` 过 —— `main.rs:205-217` 的注释自己写了这件事）。
3. `Cargo.toml`：删 `axum`、`tower-http`。
4. **删目录** `src/server/`。
5. `capabilities/default.json`（新建）：`core:default` + `dialog:default` + asset 协议读权限。

**验收**：`cargo build` 过；`grep -r "/api/" app/desktop/src` 为 **0**。

### 第 3 步：前端换出口（一次性，不留旧路径）

1. 新建 `lib/ipc.ts` + `lib/config.ts`；`lib/api.ts` 里**删掉 139 行 fetch 助手**，
   55 个实现改 `invoke`。
2. `useJob.ts`：`EventSource` → `Channel`。
3. `FilePick.tsx`：对话框改走 `pick_paths`；拖放改走原生 `Drop{paths}`；删 `fsUpload` 链。
4. 6 个 URL 助手 → `convertFileSrc`；`Convert.tsx` / `Svsep.tsx` 的 `*-upload` 调用改传路径。
5. 26 处 localStorage → `useConfig()`；加一次性 `migrate_legacy_settings`。
6. 删 `tests/contract/` 与 12 个 `.mjs`。
7. **删 `lib/api.ts`**（内容已全部并入 `ipc.ts`），9 个页面改 import 一行。

**验收**：9 页全功能手测一遍（选文件 → 预检 → 跑任务 → 看产物 → 拖放 → 播放）；
`grep -rn "fetch(" app/web-next/src` 只剩 0 处；`grep -rn "localStorage"` 只剩 migration 那一处。

### 第 4 步：测试收敛到 Rust（约束 3）

1. 把契约夹具里**有业务价值**的那些（`state` 的字段形状、`video-parse` 的归一化、
   `convert-preview` 的 findings）改写成**对领域函数的单测**：
   `normalize_info()` / `preview()` / `build_state()` 直接喂固定输入、断言输出。
2. 新增：`pv_save_chunk` 的 offset 拼接、`preview_fetch` 的 LRU 淘汰、
   `allow_directory` 的边界（`..` 逃逸必须被拒）、配置迁移的一次性语义。
3. `build.ps1` 的验证三件套改成：`cargo test --bins` + `npm run build` + 启动一次并确认
   无控制台报错（这一条只能手测，或留到第 5 步再看要不要引 `tauri-driver`）。

**验收**：`cargo test --bins` 全绿（**含现在那条陈旧的 ytdlp 期望值，顺手修**）；
`tests/` 只剩 `samples/`、`manual/fix-ps1-bom.mjs`、`manual/check-resources.mjs`。

### 第 5 步：文档与脚本收尾

`AGENTS.md` 第二节（架构图）、第三节（`--serve`、`npm run watch`）、第五节（验证）、
五之二（端口/localStorage 两条）、`docs/FRONTEND.md` 的接口契约节 —— 全部改写；
新增一条坑：**「asset 协议要运行时 `allow_directory`，否则新目录 403」**。

---

## 5. 三个必须接受的取舍（先说清，别到用户那里才发现）

| 取舍 | 说明 | 替代方案 |
|---|---|---|
| **远端预览要先落盘** | `convertFileSrc` 只认本地文件。预览 MV 变成「先缓存到 `app_cache_dir()/preview/` 再播」，首帧要等 | 无（B 站 CDN 本来就拒绝无 Referer 的直连，旧代理也常失败）。缓存反而让二次播放瞬时 |
| **浏览器里再也开不了这个前端** | 没有 HTTP 服务，`next-smoke.mjs` 那套「无头 Edge 打 8891」整体失效 | 回归手工验收；将来若需要自动化，引 `tauri-driver`（**本次不引**，避免新增脆弱性） |
| **`frontendDist` 让 `app/web` 变成构建产物** | 开发时用 `devUrl`（Vite 热更新，比现在的 watch+刷新更好）；但 `emptyOutDir:false` 那条硬规矩**依然要守**（`vendor/`、`img/` 不能被清） | 不变 |

---

## 6. 与上一版方案的差异（为什么这版才叫减法）

| | 上一版（作废） | 本版 |
|---|---|---|
| 传输层 | HTTP + IPC + 自定义协议 = **3 套** | **1 套（IPC）+ 1 个内置协议（asset）** |
| 自定义协议代码 | 手搓 `register_asynchronous_uri_scheme_protocol` + 自己解析 Range | **0 行**（asset 协议 + 运行时 `allow_directory`） |
| 测试 | 保留 HTTP 只为迁就 `.mjs` | **删掉 14,900 行测试脚本**，收敛到 `cargo test` |
| 配置 | 保留 localStorage + 临时 HTTP 迁移口 | **全部进 `config.json`**，一次 `invoke` 拉取 |
| 类型 | 引 `ts-rs` 做代码生成 | **手写收敛在 `types.ts`**（零构建负担） |
| 端口 | 保留 17878 给测试 | **端口概念消失** |
| 净行数 | +（多两套壳） | **≈ −18,000** |

---

## 7. 进度与接手说明（2026-10-04 记）

### 已完成

| 步骤 | 状态 | 证据 |
|---|---|---|
| 第 1 步：删端口 / 静态伺服 / `--serve`，窗口改 `WebviewUrl::App` | ✅ 完成并**实测**：`http://tauri.localhost`、`__TAURI_INTERNALS__` 在、17878 无监听、10 页全渲染零报错 | `tests/manual/cdp-window-check.mjs` |
| 第 2 步前半：`ipc/` 骨架 + **70 条命令全部注册** | ✅ `generate_handler!` 编得过 —— 这本身就是「70 条都存在」的机器证明 | `cargo build` 0 error |
| 领域逻辑搬进 `ipc/` | ✅ `AppState`（**现在真的被 `manage`**，`RunEvent::Exit` 那段死代码随之复活）· `JobTable` · 配置读写 + `mask_secrets` · 任务生命周期四件套 · 资源库 · `quiet_command` | `ipc/{state,config_file,jobs,config}.rs` |
| 已经**换了形态**（不是薄壳） | ✅ `svsep_separate` 收路径不收 multipart · `pv_save_chunk` 收 IPC 字节 · `job_watch` 用 `Channel` 取代 SSE · `preview_fetch` 落缓存文件 + `convertFileSrc` · `pick_paths` 用官方 dialog 插件 · 拖放走 `DragDropEvent` · `migrate_legacy_settings` | `ipc/{svsep,pv,jobs,media,fs,config}.rs` |

### 未完成

**第 2 步（剔掉 Axum）已经做完**：`server/` 目录已删除、`axum` 与 `tower-http` 已从
`Cargo.toml` 摘掉、全仓库 `crate::server::` 引用 **0 处**。搬运期那两个胶水函数
（`ipc::err` / `ipc::body`）一并删掉了。

⚠️ **`--serve` 模式与那条「浏览器里打开这个前端」的路彻底没了**：前端现在加载的是
Tauri 的资源协议，页面里没有 HTTP 服务器的位置。测试链路见第 4 步的待办。

### 剩下的是第 3 步（前端）—— 这是唯一还挡着「功能可用」的一步

现在**窗口能开、页面能画、IPC 通道是通的，但页面显示「连不上本地服务」** ——
因为 `lib/api.ts` 还在 `fetch('/api/…')`，而那些路由已经不存在了。
`cdp-window-check.mjs` 的 10/10 是「渲染 + 控制台无非法报错」，不是「功能可用」。

第 3 步要做的（`docs` 第 3 节已列）：

1. 新建 `lib/ipc.ts`（`invoke` 包装 + 错误规范化 + `Channel` 订阅助手）；
2. `lib/api.ts` 里 55 个方法逐个换成 `call('命令名', …)` —— **方法名与参数保持原样**，
   页面里的 `await api.xxx()` 一个字都不用改；
3. `useJob.ts` 的 `EventSource` → `Channel`（保留 700ms 轮询兜底）；
4. 26 处 `localStorage` → `lib/config.ts` 的 `useConfig()`（启动 `get_config`，
   写回 `set_config`，首启调一次 `migrate_legacy_settings`）；
5. 6 个拼 URL 的助手 → `convertFileSrc(path)`；`FilePick` 的对话框 → `pick_paths`；
   拖放 → `getCurrentWebview().onDragDropEvent()`；
6. `Pv.tsx` 的 `/api/pv/save` → `pv_save_chunk`；`Convert.tsx` 的 `postUpload` 删掉。

验收：真窗口里 9 页能看到真数据（不是「连不上本地服务」），且
`grep -rn "fetch(" app/web-next/src` 为 0、`grep -rn "localStorage"` 只剩迁移那一处。

### ⚠️ 五条真踩过的教训（别重走）

1. **⛔ 别用正则批量搬这 4,300 行 —— 试了两次，两次都坏。**
   * 第一次：rename 用 `(?<![\w:])name(?=\s*\()` 把 `String::from(` 也改成了
     `convert_collect(` —— 后顾只排除了 `::` 一个字符，`from` 前面就是 `String:`。
   * 第二次：为把 `Json(ok(json!({…})))` 削成 `json!({…})` 少一层括号，
     修正器**过度删除**：`Ok((json!({` 这种「少一个开括号」的行也被削，
     一轮下来新增 13 个语法错误。
   **根因**：括号平衡与名字作用域都需要理解语法，而 `Json(ok(json!({…})))` 这批
   嵌套有 100+ 处、收尾形状各不相同。**正解是按函数手工搬、一次一个文件、
   搬完立刻 `cargo build`，让编译器当校验器。** 实测：手工搬的 8 个文件里
   每一个都在「一次或两次编译」内落定。

2. **`tauri::State<Arc<T>>` 与 `axum::extract::State<Arc<T>>` 是两个类型。**
   都只是 `Arc` 的包装、但不能直接传 —— 搬迁期必须 `st.inner().clone()` 过一道。

3. **宏要按返回类型分两组。** `server/` 里一半处理器回 `Json<Value>`（永远成功）、
   一半回 `Result<Json<Value>, ApiError>`。混在一个宏里会报
   「`Json<Value>` is not an iterator」—— 看着像 `.map()` 用错，其实是分组错了。
   另有两条（`lyrics::parse_link` / `lyrics::import`）**没有 `State` 参数**，
   硬套统一宏会「takes 1 argument but 2 were supplied」。

4. **删文件会连带删掉「公共形状」，先给它们找好家。** 删 `server/simple.rs` 时
   带走了 `ApiError` 与 `ok()`（`media.rs` 还用着 22 + 29 处），删 `server/convert.rs`
   时带走了 `new_job` 四件套、删 `server/tools.rs` 时带走了 `quiet_command`
   （`platform.rs` / `svsep.rs` / `libresvip.rs` / `tools.rs` 都在用）。
   处理方式：**把实现挪到新家，再在 `server/mod.rs` 里 `pub use` 成老路径** ——
   这样待搬的那个文件一个字都不用改。

### 手工搬的固定三步（每个文件都一样）

1. `State(st): State<Arc<AppState>>` → `st: super::St<'_>`，体内 `st.` → `st.inner().`；
2. `Json(body): Json<Value>` → `args: Value`；`Result<Json<Value>, ApiError>` → `Result<Value, String>`；
3. `Json(ok(json!({…})))` → `json!({…})` —— **括号连数一起改，别只改一半**。

### ⛔ 第五条教训（最贵的一条）：**别用行号手工切片改文件**
搬 `media.rs` 的那几个纯函数时，我用 PowerShell 按「起止行号」拼数组（`$lines[0..($start-2)] + 新内容 + $lines[$e..]`），
**同一个文件被切坏三次**：第一次把 1-based 行号当 0-based 用（`Select-String` 是 1-based），
第二、三次把中间整段重复拼了进去（文件从 42 KB 涨到 128 KB / 1186 行涨到 3567 行），
每次都要靠 `cargo build` 的 `unclosed delimiter` 才发现。

**代价**：三次返工，而且第一次坏的时候我差点以为原始内容丢了（`tmp-port-broken` 已删，
git 里 `server/` 又从来没入库）。**恢复靠的是「文件其实是整块的 N 份拷贝拼起来的」这个观察** ——
按锚点（`//!` 头、`mod tests`、函数名）量出每一份的边界，只留正确的那一份。

**规矩**：
* 改多行区域，用**锚点内容匹配**（`edit` 工具的 `old_string`/`new_string`），不要用行号算术；
* 非要用行号，先 `Write-Host` 把那几行的**内容**打出来确认，别信自己算的；
* 从文件中间删东西之后，**立刻** `cargo build`，别连着做第二处改动；
* 动一个「没有备份、也不在 git 里」的文件之前，**先复制一份**（这次没做，是运气好才救回来的）。

### 进度表（2026-10-04）

| 文件 | 状态 |
|---|---|
| `server/`（9 个文件 / 5,610 行 / 56 条路由） | ✅ **整个删掉**。`axum` + `tower-http` 也从 `Cargo.toml` 摘了 |
| `ipc/`（14 个文件 / **4,484 行**） | ✅ 70 条命令全部注册、全部是真实现（不再有薄壳） |

**这一步的机器证明**：`crate::server::` 引用 **0 处**、dead-code 告警从 180 条降到 **18 条**
（剩下的是 `platform.rs` 里给 macOS 预留的对话框实现、`tools.rs` 的格式探测等
「有意保留但当前没有调用方」的东西，不是搬迁残留）。

### 顺带修掉的两处探针问题（2026-10-04）

**① CDP 的 `Runtime.evaluate` 回包是两层 `result` —— 第四、五条教训之外的第六个坑。**

```jsonc
{"id":1,"result":{"result":{"type":"string","value":"http://tauri.localhost"}}}
//            ^^^^^^ JSON-RPC 响应壳        ^^^^^^ 这里才是 RemoteObject
```

写成 `r.result.value` 会**每一项都拿到 `undefined`**，而报告照样打印得整整齐齐，
读起来像「应用没有 origin / 页面里没有 IPC」—— 我因此**误判成「PowerShell 吃掉了内联 JS 的引号」**，
白查了一轮（两处 JS 一直是好的）。`tests/manual/window-check.js` 的 `ev()` 现在两个都防了：
**只传路径**（防 shell 吃引号）+ **正确剥两层**（防假 undefined）。

⚠️ 反面参照：`tests/manual/cdp-window-check.mjs` 一直是对的（它在 `send` 里就 `resolve(msg.result)`
剥了外层），所以工作区里那个 **10/10 从来不是假的** —— 别因为新脚本坏过就怀疑旧结论。

**② `tests/manual/process-check.mjs` 不再自己连 CDP**，改成打印一份可直接粘贴的
PowerShell 检查清单 + 转调 `window-check.js`。原来那版把多行 JS 内联进 `node -e`，
两个坑叠在一起（引号 + 两层 `result`），报告全 `undefined` 却看不出哪里坏了。

### 顺带修掉的一处陈旧测试

`ytdlp::tests::normalizes_ytdlp_info_the_same_way_node_does` 的期望值少了
`formats[].url` 这个字段（它是后来给前端预览加的），于是这条用例红了很久。
加上之后 **`cargo test --bins` 现在是 93 passed / 0 failed** —— 这是「改完必须全绿」
那条验收标准在这次重构里**第一次真正成立**。
