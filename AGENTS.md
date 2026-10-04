# AGENTS.md —— 给接手这个项目的开发者 / 智能体

这里写的是**看代码看不出来**的东西：为什么长这样、哪些做法会静默出错、怎么验证。
产品与使用说明在 `README.md`，不在这一份。

**动手之前先读「零、硬规矩」和「五、踩过的坑」** —— 那两节里的每一条都是真金白银换来的。

| 节 | 什么时候读 |
|---|---|
| [零、硬规矩](#零硬规矩) | **每次动手前扫一眼** |
| [一、这是什么](#一这是什么) | 刚接手 |
| [二、架构](#二架构) | 找东西在哪个文件、要改路由 / 路径 |
| [三、构建](#三构建) | 编译、打包、改版本号、改脚本 |
| [四、前端](#四前端) | 动 `app/web-next` |
| [五、怎么验证](#五怎么验证) | 改完必须做 |
| [五之二、踩过的坑（速查表）](#五之二踩过的坑速查表) | **出问题时先查这里** |
| [五之三、液态玻璃用现成的库](#五之三液态玻璃用现成的库别自己写) | 动界面材质 / 玻璃观感 |
| [五之四、把一个外部 AI 软件嵌进来](../docs/INTEGRATIONS.md) | 要往工作站里嵌外部 AI 项目（如自动扒谱） |
| [五之五、人声转 MIDI（GAME）](../docs/INTEGRATIONS.md) | 碰 `game/`、`midi_transcribe.rs`、扒谱页 |
| [六、资源库数据](#六资源库数据) | 改 `resources.json` |
| [七、已知问题与未完成](#七已知问题与未完成) | 想知道哪里还没做完 |
| [八、平台移植](#八平台移植) | 碰 `platform.rs` / 谈 macOS / Android |
| [九、历史](#九历史留档) | 考古、找已删代码 |
| [十、下一步](#十下一步) | 接着做 |
| [十一、减法重构方案（剔除 Axum，只留 IPC）](../docs/TAURI-IPC-PLAN.md) | 要动「窗口怎么加载前端」「前端怎么调 Rust」时 |

---

## 零、硬规矩

1. **编译只能走 `build.ps1`**
   ```powershell
   powershell -ExecutionPolicy Bypass -File app\desktop\build.ps1
   ```
   直接 `cargo build` **不会**把 exe 复制到程序根目录，你双击启动器跑的还是旧版本，
   然后你会以为改动没生效。

2. **前端也由 `build.ps1` 一步做完**（第一步 `npm run build`）。⚠️ **2026-10-04 起前端
   是内嵌进 exe 的，改完必须重跑 `build.ps1`** —— 以前那句「改前端不用重编、刷新就行」
   已经作废（那是静态伺服时代的行为）。

3. **发布版依赖只增不减地写进 `bundle.resources` 时，别把 `app/web-next` 加进去。**

4. **改完必须跑验证**（第五节），并且**只能按精确 PID 停实例**，
   别按命令行子串杀进程 —— 曾经误杀过 DSH 自己的任务进程。

5. **别自己发明路径**。应用根目录、可写目录、工具目录全部来自 `main.rs::resolve_paths()`，
   见第二节「路径模型」。

6. **大文件不许入库**。`tools/`（288 MB）与 `app/web/vendor/jizura/`（54 MB）都用
   `fetch-tools.ps1` 补。⚠️ **别因为「本地看得见」就以为它在库里**。

7. **改 `.ps1` 要补 BOM、改 `.bat` 要转回 CRLF**，`write` / `edit` 两个工具都会破坏它们。
   细节见第三节「文件编码」。

8. **用户正在用的实例别动**。⚠️ **2026-10-04 起没有端口了** —— 从前端改走 Tauri IPC
   之后「端口」这个概念整体消失（验证方式是 `Get-NetTCPConnection` 上不该有任何监听）。
   更别在用户跑着任务时重编 exe 覆盖它。

---

## 一、这是什么

**V-Synth-Studio** —— 给翻调（VOCALOID / UTAU 等歌声合成）用的桌面工具。

功能：工程格式互转（40 种）、视频解析下载、音轨分离、音频处理、歌词获取、
文字 PV 生成、资源导航。全部离线，工程文件不出本机。

> **改名历史**：原叫「清沐的虚拟歌姬工作站」，2026-09 改为 V-Synth-Studio。
> `QingMu39` 是**作者署名**，不是软件名，保留不动。
> **存储键与配置目录刻意没跟着改**（`qingmu.theme` / `qingmu.pv.*` /
> `%APPDATA%\com.qingmu.vocalworkstation`）—— 改了老用户的主题偏好、PV 歌词交接
> 和全部配置就丢。看到 `qingmu` 不要以为是漏改的。

---

## 二、架构

### 一个进程、零端口（运行时不依赖 Node，也不监听任何端口）

```
┌─ Tauri 2 外壳（原生窗口）
│   ├─ 窗口用 WebviewUrl::App("index.html") 加载**内嵌**的前端资源
│   ├─ tauri::command（IPC）—— 70 条命令，前端唯一能碰到后端的入口
│   │    └─ invoke('命令名', args) → Result<Value, String>，没有端口、没有路由表
│   └─ asset 协议（Tauri 内置）—— 本地音视频播放，Range/206 是它自带的
└─ 纯 Rust 后端（libresvip / lyrics / bili / ytdlp / audio / svsep / midi_transcribe / platform）
```

**关键点（2026-10-04 起，与以前完全相反）**：

- 窗口加载的是 Tauri 的**资源协议**（Windows 上是 `http://tauri.localhost`），
  前端资源（`app/web/`）在**编译时打包进 exe**。
- **所以「改前端不用重编」这条已经不成立了** —— 改完前端要重跑 `build.ps1`
  （它会先 `npm run build` 再编 Rust），或者在开发时用 Vite（`devUrl` 已删，
  见 `docs/TAURI-IPC-PLAN.md`）。
- **端口概念整体消失**，验证方式是「`Get-NetTCPConnection` 上不该有任何监听」。
- 本机音视频**不许自己起 HTTP 服务**：本地文件用 `convertFileSrc(path)`，
  Rust 侧读字节用 `tauri::ipc::Response`。

**历史上是 Node 后端（19,019 行）→ 内嵌 axum HTTP 服务（9 文件 / 5,610 行 / 56 条路由）
→ Tauri IPC（`src/ipc/`，14 文件 / 70 条命令）**，两次都把中间层整个删掉、没有留开关。
运行时没有 `node.exe`；`node` 只出现在**构建期**（前端走 Vite，见第三节）。

### 目录

```
app/
  desktop/            Tauri + Rust 后端
    src/
      main.rs         入口：开窗口、resolve_paths、generate_handler!（70 条命令的唯一清单）
      ipc/            **前端唯一能碰到后端的入口**（14 文件 / 70 条命令）
        mod.rs        IPC 层说明：两条铁律 + 命令命名前缀 + 注册点只有一个
        state.rs      状态聚合（AppState = 路径 + 配置快照 + 任务表）与任务表
        config.rs     配置读写 + migrate_legacy_settings（取代 localStorage）
        config_file.rs 配置文件的落盘/读取 —— 用户偏好的唯一真相
        jobs.rs       任务生命周期、资源库、job_watch（Channel 订阅）
        fs.rs         选文件 / 打开 / 拖入落盘 / 二进制读写 / asset 协议放行
        tools.rs      外部工具探测（绕缓存）与启动外部程序
        convert.rs    工程转换（LibreSVIP 编排）
        media.rs      视频解析下载 / 音频工具 / 预览缓存（convertFileSrc）
        lyrics.rs     歌词（网易云专区）
        svsep.rs      音轨分离
        midi.rs       人声转 MIDI
        pv.rs         文字 PV 分块落盘
        bili.rs       B 站扫码登录
      net.rs          共用 HTTP 客户端 + DEFAULT_UA
      platform.rs     跨平台路径、下载目录、回收站、find_binary
      tools.rs        外部工具的探测实现（被 ipc/tools.rs 调）
      libresvip.rs    LibreSVIP 引擎封装（转换 + 读工程）
      lyrics.rs       歌词：搜索 / 取词 / LRC-SRT / 封面 / 歌曲直链 / 短信登录
      svsep.rs        音轨分离：离线引擎（Python 子进程）+ 大包下载解压
      bili.rs         B 站原生解析（WBI 签名、DASH、番剧）
      ytdlp.rs        yt-dlp 桥接
      audio.rs        音频处理（ffmpeg）
      data.rs         静态数据（格式表、拼音）
    tauri.conf.json   窗口、打包、resources
    build.ps1         唯一的构建入口
  web-next/           前端**源码**（React + Vite + TS + Tailwind）—— 9 页已全部搬完
    vite.config.ts    base '/'，outDir '../web'（emptyOutDir **必须是 false**，见下）
                      ⚠️ 含 restoreStandardBackdropFilter 插件（lightningcss 会删标准
                      backdrop-filter），别删，见 docs/GLASS-HANDOFF.md §3.1
    src/
      main.tsx        入口（引库的 style.css + 我们的 index.css，**顺序不能改**）
      App.tsx         外壳：顶栏 / 侧栏 / 导航 / 主题与材质开关 / 路由（hash）
      index.css       **零手写玻璃**：只排布局，颜色全用库的 --lg-* 令牌
      components/
        Glass.tsx     玻璃材质（库的 GlassSurface 包装）+ materialOptions()
        Panel.tsx     Panel(库的 MaterialView) / GlassPanel / Chip / Finding / Stat
        Button.tsx    库的 GlassButton / GlassIconButton
        Field.tsx     Field / TextInput / TextArea
        Icon.tsx      手写 SVG path 表（**没有图标库**，要离线）
        Job.tsx       任务进度（库的 GlassProgress + 取消 + 日志）
        DirPicker.tsx 目录选择（库的 GlassDialog + PathBar + List）/ DirectoryInput
      lib/
        api.ts        后端调用（⚠️ **第 3 步待改**：现在还是 fetch('/api/…')，要换成 invoke）
        ipc.ts        （第 3 步新建）invoke 包装 + 错误规范化 + Channel 订阅助手
        config.ts     （第 3 步新建）useConfig()，取代 26 处 localStorage
        types.ts      后端数据结构（照回包形状手写，**不引 ts-rs**）
        format.ts     formatBytes / formatDuration / formatNumber
        useJob.ts     任务订阅（⚠️ 待改：SSE → Channel，保留轮询兜底）
        useGlass.ts   玻璃等级 1~4（材质 / 透明度 / 面板要不要玻璃全由它派生）
        useNavLens.ts 侧栏与小节导航的滑动高亮块
        boot.ts       揭开启动加载画面
      pages/          9 页：Dashboard / Convert / Video / Svsep / Audio / Lyrics / Pv / Resources / Settings
                      （每页自带一个同名 .css；页面约定与库组件清单见 docs/FRONTEND.md）
  web/                前端**产物 + 随包静态资源**
    index.html        ← Vite 产物（**也是 resolve_paths 的哨兵文件**）
    assets/           ← Vite 产物（带内容哈希，每次构建新增；emptyOutDir:false 所以旧的不自动删）
    vendor/jizura/    JIZURA 文字 PV（上游构建产物 + 2335 个字体）—— **不是产物，别让构建清掉**
    img/bg/           桌面背景图（明亮/黑暗）—— 同上，被 index.css 以 url() 引用
    img/logo.png      顶栏图标（源 app/desktop/icons/128x128.png，拷进来才打得到）
  data/                只读数据：resources.json / pinyin.json（schema 见第六节）
                       绿色版的可写 config.json 也落在这里（安装版在 %APPDATA%）
tools/                 随包分发：ffmpeg / yt-dlp / LibreSVIP（约 288 MB）—— **不入库**，见第三节
tests/
  contract/            接口契约（对冻结的夹具）
  manual/              浏览器 / 接口探针
docs/                  THIRD-PARTY-NOTICES / FEATURES / FRONTEND / GLASS-HANDOFF / LESSONS
```

> ⚠️ **`lib/useTheme.ts` 与 `lib/usePerfMode.ts` 都不存在了**：主题与透明度在 `App.tsx` 里
> 直接喂库的 `GlassProvider`；`perfMode` 后端有字段、**前端还没接**。启动加载画面见
> `GLASS-HANDOFF.md` §4（`#boot` + `lib/boot.ts`）。

### 路径模型（这段很重要）

`main.rs` 的 `resolve_paths()` 按顺序找应用根目录：

1. Tauri 的 `resource_dir()`（安装版）
2. 从 exe 往上找（绿色版，最多 5 层，认的是 `app/web/index.html` 这个哨兵）
3. 从 cwd 往上找（开发时）

找到后再判断 `is_writable(<root>/app/data)` 区分**绿色版**还是**安装版**：

| | 绿色版 | 安装版 |
|---|---|---|
| 只读资源 | `<root>/app/`、`<root>/tools/` | 同左（Program Files，只读） |
| 可写数据 | `<root>/app/data/` | `%APPDATA%/com.qingmu.vocalworkstation/` |

**判断依据是「能不能写」，不是「装没装」。** 曾经因为 `resource_dir()` 在绿色版
也返回 exe 目录，导致绿色版被误判成安装版、配置写到 `%APPDATA%` 去了。

⚠️ **「往上找」这套逻辑有两个已知副作用**：① 把 exe 复制到任意临时目录跑，
它会一路向上找到真正的安装目录，用的还是**真安装**的数据 —— 要隔离就在临时目录里
放一份自己的 `app/web`；② 对 macOS bundle（`Contents/Resources/` 布局）已经出问题，见第八节。

---

## 三、构建

| 事项 | 必须这样做 |
|---|---|
| 编译 | **只能** `powershell -ExecutionPolicy Bypass -File app\desktop\build.ps1` |
| | 直接 `cargo build` **不会**把 exe 复制到根目录，你跑的还是旧的 |
| `-Release` / `-Bundle` | 可选参数；`-Bundle` 打 MSI（见第七节） |
| `-SkipWeb` | 只编后端（改 Rust 时省几秒，但 `app/web/` 里会是旧产物） |
| `-FetchTools` | 先补齐 `tools/` 与 JIZURA 字体（干净机器 / CI 上用；要联网） |
| `-NoCopy` | 编完**不**把 exe 复制到程序根目录（CI 用；本地别加） |
| 工具链 | Rust 在 `H:\DevTools\cargo`、MSVC 在 `H:\VSBuildTools`（build.ps1 会加载 vcvars） |
| | **前端还要 Node + npm** —— 这是新前端引入的**构建期**依赖 |
| 打包 | CI 在 `.github/workflows/build-msi.yml`（打**版本 tag** `v1.2.0` 自动出 MSI 并传 Release）。⚠️ 规则是 `v[0-9]*` —— 附件 Release 那个 `assets-v1` 故意不开头，免得建附件就触发一次构建 |
| 跨平台 | `-Bundle` 只在 Windows 可用（脚本会主动报错），macOS / Android 要另写壳，见第八节 |

### 仓库只放源码，两大块大件编译前补齐

**约 7 MB 的仓库**是有意为之。下面两块不属于源码，但程序要能离线用就必须在打包前到位：

| 大件 | 体积 | 补齐后落在 | 谁在用 |
|---|---|---|---|
| ffmpeg + yt-dlp + LibreSVIP CLI | 约 288 MB | `tools/` | `audio.rs` / `tools.rs` / `libresvip.rs` |
| JIZURA 与 2335 个 woff2 字体 | 约 54 MB | `app/web/vendor/jizura/` | 文字 PV 页的 iframe（**离线可用靠它**） |

一条命令补齐（幂等，齐了就跳过）：

```
powershell -ExecutionPolicy Bypass -File app\desktop\fetch-tools.ps1
powershell -ExecutionPolicy Bypass -File app\desktop\fetch-tools.ps1 -Local 'D:\存着两个zip的目录'   # 不联网
```

来源是本仓库 Release 的附件（tag **`assets-v1`**）—— `tools.zip`（129 MB）与
`jizura.zip`（51 MB），由 `tools\zip-assets.ps1` 打出来（那个脚本**只打包、不上传**）。
两处 URL：`fetch-tools.ps1` 的仓库地址是**自动从 `git remote get-url origin` 推**的
（HTTPS / SSH 两种写法都认，fork 出去不用改；没有远端时退回文件里那个备选值），
附件名（`tools.zip` / `jizura.zip`）与工作流里的 `ASSETS_TAG` 才是硬编码的 ——
**换 tag 或换托管时改这两处**；想临时换地址用 `$env:VSYNTH_TOOLS_URL` / `$env:VSYNTH_JIZURA_URL`。
`fetch-tools.ps1` 拿不到存档时会退到三个上游官方地址现下（gyan.dev / yt-dlp release /
LibreSVIP release）—— 慢，但不用人去别处找。

**这两个附件要仓库主人亲手传一次**（fork 的人不用传，直接用上游的）：

```
powershell -ExecutionPolicy Bypass -File tools\zip-assets.ps1     # ① 打出资料归档\tools.zip、jizura.zip
# ② 在网页上建一个 tag 为 assets-v1 的 Release（Releases → Draft a new release → Publish）
$env:GITHUB_TOKEN = '<只给这一个仓库 Contents 写权限的 token>'
powershell -ExecutionPolicy Bypass -File tools\upload-assets.ps1  # ③ 传到那个 Release（只传，不建 Release）
Remove-Item Env:\GITHUB_TOKEN
```

`upload-assets.ps1` 会先查远端拿 `owner/repo`、再查那个 tag 的 Release，然后流式上传
（**不用 `Invoke-WebRequest -InFile`**，PS 5.1 传二进制会坏），已存在的同名附件默认跳过、
`-Force` 才先删再传。token 只走环境变量，**不要写进命令行**（会留在 PSReadLine 历史里）。
⚠️ **附件那个 Release 的 tag 别用 `v` 开头**：工作流是 `on.push.tags: ['v[0-9]*']`，
用 `v…` 建它会顺手触发一次没用的构建 —— 现有默认值 `assets-v1` 正是为避开它。
传完再打版本 tag（`git tag v1.2.0 && git push origin v1.2.0`）就会自动出 MSI。

> ⚠️ **补齐脚本的六条实测教训**（2026-10-02 干净房间验证抓出来的，别再踩）：
> ① **判「齐不齐」必须核对解压后的具体文件**，不能只看目录在不在 —— 半途失败的解压
>    会留下看似完整的空壳，要到用户点「工程转换」才现形。
> ② 下载用 curl.exe + **自己写的重试循环**：`Invoke-WebRequest` 读 GitHub release 的大
>    文件实测会 `Received an unexpected EOF or 0 bytes`；旧 curl 不认 `--retry-all-errors`。
> ③ `-Bundle` 会**在编译前**核对 `tools\`、`vendor\jizura\`、`app\web\index.html` 在不在 ——
>    `bundle.resources` 是「源目录缺文件就静默少打包」，缺了要到用户手里才现形。
> ④ **搬目录前先把父目录建出来**（脚本里的 `Move-Into`）。`Move-Item` 不建中间目录，
>    往 `app\web\vendor\jizura` 搬而 `vendor\` 不存在时报 `Could not find a part of the path.`，
>    而且**目标没到位**（源倒是没了）。同理**给函数返回值的函数里报进度要用 `Write-Host`**，
>    用 `Write-Output` 会混进返回值，调用方拿到「提示 + 路径」拼的垃圾字符串。
> ⑤ **`tools\` 里同时住着入库的源码**（`zip-assets.ps1`、`fetch-jizura-fonts.ps1`），所以补齐
>    只能**合并**进去 —— 而 `zip-assets.ps1` **不在存档里**，整目录替换必删它。
> ⑥ ⚠️ **`Move-Item` 的目标已存在时是「嵌套」不是「覆盖」**（目录对目录也一样，`-Force`
>    拦不住）：把 `_verify-hold-X\tools` 搬回已存在的 `tools\` 会得到 `tools\tools\`。
>    还原临时区要**逐项搬内容**。④⑤⑥ 同源：**别再靠肉眼审脚本** —— 那个测试脚本跑 6 次
>    抓出 4 个必然踩中的 bug，其中两个只有在「`tools\*.ps1` 留在原地」的前提下才测得出来。

### Node 只在构建期出现（别和「去 Node」搞混）

项目历史上把 Node 后端整体重写成 Rust，去掉的是**运行时**的 Node：

| 环节 | 需要 Node 吗 |
|---|---|
| 用户运行打包好的 exe | **不需要** —— 产物是静态 HTML/JS/CSS，exe 是 Rust |
| 后端运行时 | **不需要** —— 70 条 IPC 命令全在 Rust，`node.exe` 进程数为 0 |
| **编译前端**（`build.ps1` 第一步） | **需要** —— Vite 是 Node 工具 |

`app/web-next/node_modules/` 约 91 MB，**但不进安装包**：
`tauri.conf.json` 的 `resources` 只映射 `../../app/web`（Vite 产物里不含依赖），
`web-next` 一个字都没被映射。**往 `bundle.resources` 里加东西时别把 `web-next` 加进去。**

启动器 / CI 里任何「这台机器没有 Node」的假设都已失效 —— 编译机必须有。

### 版本号写在哪儿（改版本号时五处一起改）

对外显示的是 `1.2beta` 这种写法，但 Cargo / npm / 打包器各自要的是合法 semver：

| 位置 | 现在写的 | 谁读它 |
|---|---|---|
| `app/desktop/src/ipc/config_file.rs` 的 `APP_VERSION` | `1.2beta` | **界面**（总览页脚、「关于」小节的「程序版本」），`get_state` 与 `get_config` 都发它 |
| `app/desktop/Cargo.toml` 的 `version` | `1.2.0` | cargo（必须是合法 semver） |
| `app/desktop/tauri.conf.json` 的 `version` | `1.2.0` | **MSI 的版本号**（不是 git tag） |
| `app/web-next/package.json` 的 `version` | `1.2.0` | npm（只在日志里出现，但别让它落后） |
| 两个锁文件 `Cargo.lock` / `package-lock.json` | `1.2.0` | 别手改，跑下面两条命令让它们自己跟上 |

```powershell
# 锁文件（两处都别手改）
cargo update -p v-synth-studio --precise 1.2.0                                  # 在 app\desktop 下
cd app\web-next; npm install --package-lock-only --no-audit --no-fund
```

⚠️ 只改 `APP_VERSION` 而不改 `tauri.conf.json`，装出来的 MSI 版本号会跟界面显示的对不上；
只改 `tauri.conf.json` 而不改 `APP_VERSION`，界面还显示旧版本 —— **两处都没有自动同步**。
（`AUTHOR_TAG` 同理，作者署名固定 `QingMu39`，改名史见第一节。）

### 界面只有一套（2026-10-02 切换完成）

| 启动方式 | 界面 |
|---|---|
| `启动工作站.bat` | React 前端（URL 前缀 `/`） |
| 直接双击 `v-synth-studio.exe` | **同一个** React 前端 |
| `v-synth-studio.exe --serve --port=<端口>` | **已删除**（2026-10-04）：没有 HTTP 服务了，写它没有任何效果 |

**`--ui=next|old` 与启动器的 `--old` 已经删除**，现在写它们不会有任何效果（参数被忽略）。
旧的手写前端（`app/web/js/`、`app/web/css/`、手写 `index.html`）已整体删除，
`docs/LEGACY-UI.md` 一并退役。

> 这一节以前叫「切换界面（旧前端 / 新前端）」，记着「启动器默认新前端、exe 默认旧前端」
> 那套双界面机制。**那套机制已经不存在了，别再照它推理。**

⚠️ **改前端也要重跑 `build.ps1`**（前端内嵌进 exe 了）。`npm run watch` 只负责重建
`app/web/` 那份产物，它不进 exe —— 想看到效果必须重编。

### 文件编码

| 文件 | 要求 | 不遵守会怎样 |
|---|---|---|
| `.ps1` | **UTF-8 带 BOM** | PS 5.1 把中文注释按 ANSI 读 → 乱码吞掉换行 → 语法错 |
| `.bat` / `.cmd` | **CRLF** | cmd 解析不了 LF，命令会拆错 |
| `.bat` / `.cmd` | **逻辑块只用 ASCII** | 见下，中文会让 cmd 冒出假报错、甚至弄坏分支判断 |

⚠️ **`write` / `edit` 工具会丢 BOM、会把 `.bat` 存成裸 LF。** 改完检查头三字节是不是
`EF BB BF`（`.bat` 则要查 CRLF）：

```powershell
node tests\manual\fix-ps1-bom.mjs          # 只报告缺 BOM 的 .ps1
node tests\manual\fix-ps1-bom.mjs --write  # 补上
```

⚠️ **`.bat` 里的中文只能出现在 `echo` 行，别的地方一律 ASCII。** 踩过两次：

1. `REM` 注释里有中文 → 跑起来冒出一串假的
   `'xxx' is not recognized as an internal or external command`
2. **`if/else` 块里的 `echo` 带中文 → 不只冒噪音，还会弄坏分支判断本身**：
   `if /i "%~1"=="--old"` 明明该命中，却一直走 `else` 分支。
   多行括号块遇到多字节 UTF-8 时 cmd 的解析会出错。

修法：**把判断逻辑和中文彻底分开** —— 逻辑用纯 ASCII 的 `if`/`goto`，
中文只留在块外的单行 `echo`，或者干脆让程序自己报。

改 `.bat` 之后必须确认这几件事：

```powershell
$t = [IO.File]::ReadAllText('启动工作站.bat', [Text.Encoding]::UTF8)
$t.Contains("`r`n")                                      # 要 True
[regex]::IsMatch($t, "(?<!`r)`n")                        # 要 False（没有裸 LF）
($t -split "`r`n" | Where-Object { $_ -match '[^\x00-\x7F]' -and $_ -notmatch '^\s*echo' }).Count  # 要 0
```

**光看代码不够 —— 必须真的把两条分支都启动一次**（上面第 2 条坑就是静态检查全绿但分支是坏的）。

### 单独弄前端

```powershell
cd app\web-next
npm install     # 首次
npm run build   # tsc -b && vite build → 产物落 ../web/
npm run watch   # 开发时推荐：改完自动重建，浏览器刷新即可
```

⚠️ **npm 的选取有讲究**：`build.ps1` 优先用 `H:\node\npm.cmd`（自装 Node），
其次 `%ProgramFiles%\nodejs`，最后才退回 PATH 搜索。**不要改成直接取 PATH 里第一个** ——
本机 PATH 第一顺位是 DSH 运行时自带的 `...\resources\node\`，DSH 一升级就没了。
另外 `Get-Command npm.cmd` 在本机**命中 2 个**，直接取 `.Source` 会拿到数组并拼成垃圾字符串
（踩过，报错信息是 `The term '...npm.cmd H:\node\npm.cmd' is not recognized`）。

工具链（本机实测可用）：

| | 版本 | 说明 |
|---|---|---|
| Node | v24.20.0 | **只有开发机需要**，用户那边不用装（CI 用 `actions/setup-node` 装 24） |
| npm | 11.19.0 | |
| React / react-dom | 19.3 | |
| Vite | 8.3 | |
| TypeScript | **7.0** | ⚠️ TS 7 移除了 `baseUrl`，`paths` 改成相对 tsconfig 解析 |
| Tailwind | 4.3 | 用 `@tailwindcss/vite` 插件（v4 不需要 postcss/tailwind.config） |

---

## 四、前端

**只有一套界面了。** 旧的手写前端（`app/web/js` + `app/web/css` + 手写 `index.html`）
已于 2026-10-02 整体删除 —— 别再去 `app/web/js/` 找东西。

前端规范、页面约定、库组件清单、接口契约坑、验证工具与人工验收清单
**全在 `docs/FRONTEND.md`** —— 动前端之前读那份。界面上的历史取舍见 `docs/LESSONS.md`。

⚠️ **`app/web/` 里有两类东西，别搞混**：`index.html` 与 `assets/` 是 Vite 产物
（会被构建覆盖）；`vendor/`（JIZURA）与 `img/` 是**随包静态资源**，被产物引用但**不产出**。
所以 `vite.config.ts` 的 `emptyOutDir` **必须是 `false`** —— 设成 `true` 会把 vendor 和 img
一起清掉（PV 页与全部背景图失效），而构建还报成功。`build.ps1` 为此加了防线。

### 上 Vite 时的四条硬约束（已全部落地）

- Vite 的 `outDir` 指向 `app/web/`、`base: '/'` —— 那份产物是 `frontendDist` 的来源，
  `build.ps1` 编 Rust 时会把它**打包进 exe**。
- `emptyOutDir` **必须是 `false`**（理由见上）。
- ⚠️ **前端调后端只剩一条路：`invoke('命令名')`**。**别再写 `fetch('/api/…')`** ——
  那些路由已经不存在了（见 `docs/TAURI-IPC-PLAN.md`）。命令清单在 `main.rs` 的
  `generate_handler!`（70 条，唯一登记处）。
- `build.ps1` 仍是唯一构建入口（第一步就是 `npm run build`），代价是**编译机必须有 Node**。

---

## 五、怎么验证

```powershell
# 1) Rust 单测 —— 现在是主验证（应 93 passed / 0 failed）
cd app\desktop; cargo test --bins

# 2) 起真窗口（带 CDP 端口）
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9349'
$app = Start-Process -FilePath '.\v-synth-studio.exe' -PassThru
Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
Start-Sleep -Seconds 9

# 3) 问窗口自己 + 10 页冒烟（都应全绿）
node tests\manual\window-check.js 9349
node tests\manual\cdp-window-check.mjs 9349
```

- ⚠️ **契约测试与 `--serve` 已退役**（2026-10-04）：没有 HTTP 服务，`tests/contract/`
  那套（对冻结夹具 diff、17 个用例）整条路失效，**别再去修它** —— 验收收敛到 `cargo test`。
- `window-check.js` 问窗口「origin / IPC 在不在 / 渲染了多少」，`cdp-window-check.mjs`
  走 10 页冒烟。两个都用 CDP，**表达式写在文件里、命令只传端口**。
- ⚠️ **CDP 的 `Runtime.evaluate` 回包是两层 `result`**（外层是 JSON-RPC 响应壳、内层才是
  RemoteObject）。写成 `r.result.value` 会**每一项都拿到 `undefined`**，而报告照样打印得整整齐齐，
  看着像「应用没有 origin」—— 这个坑真踩过、白查一轮。

### 浏览器验证的坑

| 坑 | 说明 |
|---|---|
| 无头模式 | 必须 `--headless=old`（本机 `--headless=new` 报 Multiple targets 起不来） |
| `--disable-gpu` | **只对截图有害**：带着它 `backdrop-filter` 会糊成一片空白。<br>`next-smoke.mjs` 里带了它没关系 —— 那条路只 dump DOM，不看画面对不对 |
| `--dump-dom` 看不到 iframe 内部 | 要验 iframe 里的东西必须用 CDP（`--remote-debugging-port` + Node 自带 WebSocket，不要装包）。参考 `tests/manual/pv-verify.mjs` |
| 端口 | **没有了**（2026-10-04 起）。验证方式反过来：`Get-NetTCPConnection` 上不该有任何监听 |

### 进程卫生

跑完测试**必须**停掉测试实例、清掉无头 Edge（不清会锁住 exe 让别人编译失败；
无头 Edge 每个约 60–100 MB，会堆到十几个）：

```powershell
Stop-Process -Id $app.Id -Force      # 上面 Start-Process -PassThru 拿到的那个 PID
Get-Process -Name 'msedgewebview2','msedge' -EA SilentlyContinue |
  Where-Object { $_.MainWindowHandle -eq 0 } | Stop-Process -Force
```

⚠️ **杀进程只能按精确 PID**（端口没了，所以「按端口 owner 杀」这条老办法也失效了），
别按进程名或命令行子串匹配 —— 曾经误杀过 DSH 自己的任务进程。

---

## 五之二、踩过的坑（速查表）

> **细节与来龙去脉都在 `docs/LESSONS.md`** —— 这一节只留「别再犯」的结论。
> 出问题先查这张表：**症状 → 结论**。

| 症状 / 场景 | 结论 |
|---|---|
| `open_path` 收两种参数 | `path`（本地路径，**要求存在**）或 `url`（http/https/ftp/mailto，走系统默认程序、不做存在性检查）。**2026-10-02 之前它只读 `path`**，于是所有传 `{url}` 的调用必然 400 —— 两个前端的「在浏览器打开」都是坏的，已修（`platform::open_url` + `looks_like_url`，带单测） |
| 接口「发了没反应 / 永远空列表」 | **先对后端源码与夹具**，别信旧前端的调用姿势：`collect` 要 `{dirs:[…]}`（旧前端发 `{dir}` → 永远 0 个文件）、`preview` 要 `{inputs,toFormat}`、`fs/list` 空 `path` **必须整个省略**（发 `path=` → 400）、`fs/roots` 字段是 `name` 不是 `label`。四处都是搬页面时实测翻出来的，见 `docs/FRONTEND.md` 第 5 节 |
| 「改了界面但用户看不到变化」 | **先怀疑缓存**：静态文件必须发 Cache-Control: no-store（simple.rs 已加）。测试每次开全新浏览器，永远命中不了缓存，只有用户常驻的 WebView2 拿着旧文件 |
| **某条命令永远不返回 / 窗口像卡住了**（旧形态是「路由连回复都没有」：`Empty reply from server` / `HTTP 000` / `size 0`，**没有 500、没有 JSON、没有日志**，同一个进程别的路由照常 200） | 这是 **tokio worker 线程 panic**，不是命令本身的问题 —— panic 会把那一条命令的应答整个吞掉。真踩过：`midi_status` 里调 `ort` 的 `is_available()` 时 panic（`Failed to load ONNX Runtime dylib: MissingApi { path: "onnxruntime.dll" }`）—— ort 的 `setup_api()` 是**惰性**的，**任何 ORT 调用之前必须先 `ort::init_from(...).commit()`**，否则它去找裸文件名。见 `docs/INTEGRATIONS.md`「五之五·补」⑥ |
| 想知道「这台机器能不能用 CUDA」 | ⛔ **`ort::ep::CUDA::is_available()` 回答不了**：它只说明「这份 ORT 构建里**编进了** CUDA provider」，**AMD RX 580 上也回 `Ok(true)`**。唯一真判据是拿一张 1×1 的 Identity 图 `commit_from_memory` 一次。见「五之五·补」⑤·补 |
| 手写 protobuf 拼 ONNX 图，ORT 报 `Tensor does not have type information.` | 错在**字段号**不在长度：`ValueInfoProto{ name = 1, type = 2 }`（`type` 是**字段 2**），`shape.dim.dim_value` 也不能丢。⛔ **自己写的「线格式自检」抓不到**（它验的是同一个错误假设、照样全绿）—— 必须以真 ORT 建一次会话为准 |
| 「往上找某个目录」的函数永远找不到 | 先怀疑**判据写反了**（要找的层在**更下面**）：真踩过 `if d.file_name() == "nvidia"`，而 `nvidia` 是 `site-packages` **下面**的一层 ⇒ 条件恒假、不报错、只是永远不生效。见「五之五·补」③ |
| ~~端口不能随机~~ | **这条已作废**（2026-10-04）：端口没了，配置也不再放 `localStorage`，改成后端 `config.json`（启动时一次 `get_config` 拉回来）。**旧的 `qingmu.*` / `fandiao.*` 键**由 `migrate_legacy_settings` 一次性搬过去（已有的键优先，幂等） |
| 悬停/过渡「生硬地闪一下」 | 多半是 `var(--x)` **没定义** → 整条 `transition` 静默失效。先跑 `LESSONS.md` 里那段查未定义变量的脚本 |
| 过渡曲线 | `--ease` 管微交互、`--ease-out` 管入场、`--spring` 只给大位移；取值表在 `LESSONS.md` |
| 侧栏/导航 | **一个框 + 一个滑动高亮块**（库的 .lg-selection-lens），行本身零描边零底色；首帧不能滑、用 offsetTop 量位置（`lib/useNavLens.ts`）。⚠️ 高亮块**不是玻璃面**，库的 `--lg-lens-bg` 只按主题分档；要玻璃得自己在 `.nav-lens` 上改半透明 + 消费 `--lg-backdrop`。⚠️ 侧栏 `position: sticky` 的 `top` **必须等于初始位置**（含让开顶栏那 44px），否则一滚就先跳 44px |
| 玻璃 | 用现成的库，**永远别自己写**；材质写在 `GlassProvider` 上；背景自身模糊要小（3~5px）；栏本身不画底。细节在 `GLASS-HANDOFF.md` §2.2 |
| 苹果式圆角 | `corner-shape: squircle` + `@supports` 兜底；**别用在玻璃面上**（库的位移贴图是受限几何） |
| 自定义 CSS 与工具类 | 新前端目前是纯手写 CSS（没用 Tailwind 工具类）。哪天开始用工具类，自定义类必须进 `@layer components`，否则会静默盖掉工具类 |
| 亮色主题 | 次要文字色不能太浅（对比度 4.5:1 以上）；背景图参数见 `LESSONS.md` |
| **面板里的输入框比旁边的下拉框宽一截、右边缘压出面板边框** | 玻璃档 **1~3** 的面板是普通 `div.panel`、**不是 `.lg-root`**，拿不到库那条 `.lg-root * { box-sizing: border-box }`（`node_modules/@ttqtt/liquid-glass-react/dist/components.css:7`），而 `<input>` 的 UA 默认是 content-box ⇒ `width:100%` 按内容盒算，宽 26px 高 17px（档 4 自动变好，因为那时才是玻璃根）。**我们自己的 CSS 里 `box-sizing` 声明数原本是 0** —— `.input/.textarea` 必须自己写 `box-sizing: border-box`（`app/web-next/src/index.css`），`.lyrics-result`/`.svsep-drop` 这类 `<button>` 是 UA 默认就 border-box，但也要写明（换元素类型就会踩同一个坑） |
| **分段控件（`GlassSegmentedControl`）在明亮主题 + 玻璃 3 下选中项文字看不见** | 库给 **clear 材质**设了 `--lg-fg: #fff`（`dist/components.css:68`），而亮色主题的胶囊底 `--lg-lens-bg` 是**不透明纯白**（`dist/tokens.css:461`）⇒ 白字压白底。库自己对 opaque 材质有兜底 `--lg-fg: rgb(0 0 0)`（`components.css:102`），我们照抄一条 `[data-lg-theme='light'] .lg-root[data-material='clear'] { --lg-fg: var(--lg-label) }`。⚠️ 覆盖 `--lg-fg` 前先想清楚作用域（只到 clear 材质的玻璃根；带 variant 的按钮走 `--lg-accent-contrast`，不受影响） |
| **启动时白屏 / 一直转圈「正在载入工作台…」（明亮 + 玻璃 3 更明显）** | 两个成因叠加：①遮罩只在 `/api/state` 落定后才揭，而 `/api/state` 过去**每个请求**都现算 `detect_tools` —— 它会真的 spawn `yt-dlp --version`/`python --version` 并逐段扫 PATH，机器忙时 2~7 秒（连跑还越来越慢）；已改成 `AppState::probe_cached` 60 秒缓存 + 启动后台线程预热 + `App.tsx` 里 **1.2 秒兜底揭遮罩**。②玻璃 3 的液态折射：`backdrop-filter` 元素 1→14 个、`<feDisplacementMap>` 0→8 个、模糊面积 6840→1,284,139 px²（188×），明亮主题还多一层全屏 `body::before{filter:blur(5px)}`。数据与整改清单见 **`安全审查.md`** |
| 背景图「压根不显示」 | 触发过两次。图在 `app/web/img/bg/`，被 `index.css` 以 `url()` 引用 —— 别让构建把它当成产物清掉（`emptyOutDir` 必须是 `false`） |
| 网络 | GitHub / Google 要走代理（`curl -x http://127.0.0.1:7890`）；网易云直连；**测试短信接口绝不用真实手机号** |
| 图标 | `components/Icon.tsx` 是一张手写 SVG path 表。**没有图标库**（要离线），加图标往表里加 |
| **Rust 注释里别写 `/*`** | 块注释会**嵌套**：文档注释里写 `` `app/web/js/views/*.js` `` 会让整个注释永不闭合，吞掉后面几十行，rustc 报出**29 条假错**（`prefix 'wav' is unknown`、`unterminated double quote string`）。看到成片的这类错先找「注释没闭合」，别逐个去改字符串 |
| **大文件不能进 git** | `tools/`（288 MB）与 `app/web/vendor/jizura/`（54 MB）都已从 git 移出（`git rm --cached`），靠 `fetch-tools.ps1` 补齐。**别因为「本地看得见」就以为它们在库里** |
| **磁盘** | C 盘很紧，临时大文件放 `H:\工作站\tmp-*` 并即时删。⚠️ **解 4.6 GB 的 runtime 包要 7.5 GB 空间**，默认解到 `%TEMP%`（C 盘）—— 实测把 C 撑到 0 字节，报 `解压失败：磁盘空间不足 (os error 112)`，**看着像解析器坏了**。真包测试用 `VSS_REAL_RUNTIME_DEST` 指到 H 盘 |
| **手写 zip 解析器**（`svsep.rs`） | 只认「压缩后大小」溢出是不够的：**本地头偏移超过 4 GiB 时 `lho` 也是哨兵 `0xFFFFFFFF`**，真值同在 Zip64 扩展块里（排在两个大小之后）。漏了它 → 拿 0xFFFFFFFF 当文件位置 seek → 报 `failed to fill whole buffer`（2.7 万条里查不出来）。现在 `zip64_resolve(extra, big_size, big_off)` 两个哨兵一起处理。⚠️ 报错**必须带条目名**，否则这种错没法定位 |
| **「打包好了」≠「装机装得上」** | 判据是**代码真去找的那几个文件**，不是「包里有一大堆文件」。`runtime_ready()` 要 `runtime/python.exe` **和** `backend/app.py`，而打包脚本第一版用 `CreateFromDirectory` 只能装一个顶层目录 → 只装了 `runtime\`，用户下完 4.5 GB 仍然起不来。改成 `ZipFile.Open` + `CreateEntryFromFile` 手工加条目（`Dirs = @('runtime','backend','bin')`）。⚠️ 用 `ZipArchiveMode` 必须**同时** `Add-Type System.IO.Compression`（`.FileSystem` 里没有这个类型） |
| **后台跑 cargo test 会被 linker 撞** | 两个 `cargo test` 并行会抢同一个输出文件，报 `linking with link.exe failed: exit code: 1104`（**不是代码问题**）。串行跑 |
| **子进程收不住强杀** | 分离引擎是 `python.exe` 子进程。`impl Drop for Svsep` 只覆盖正常退出 —— **任务管理器强杀实测留下孤儿**：它继续监听 17879、占着几 GB 内存，用户看到「关掉了风扇还转」。兜底是 Windows 作业对象（`svsep.rs` 的 `job` 模块）：`CreateJobObjectW` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject`，句柄故意不关（存 `OnceLock`），进程一死句柄被内核回收 → 作业里的进程一起死。⚠️ `windows-sys` 要同时开 `Win32_System_JobObjects`、`Win32_System_Threading`（类型定义在 Threading 下）和 `Win32_Security`（`CreateJobObjectW` 的参数用了 `SECURITY_ATTRIBUTES`） |
| **看着在收、其实没跑** | `main.rs` 的 `RunEvent::Exit` 里写了 `app.try_state::<Arc<AppState>>()`，却没有任何地方 `manage` 过它 —— `try_state` 会**永远回 None**，于是「退出时收尾」那段代码从来没执行过。这种死代码比不写更坏。⚠️ 2026-10-04 之前的旧架构正是这个状态（`AppState` 建在 `serve()` 里）；改成 IPC **之后它才真的被执行** |
| **CI 上 `link.exe` 报 `/usr/bin/link: extra operand`** | **Git for Windows 的 `C:\Program Files\Git\usr\bin` 在 runner 的系统 PATH 里，那个 `link.exe` 是 coreutils 的 `ln` 别名**，rustc 调裸名 `link.exe` 就撞上它。后面那句「build tools may need to be repaired」是**纯误导**。修法在 `build.ps1`：把 `\Git\{usr,mingw64,cmd}` 从 PATH 剔掉、再把 MSVC 的 `bin\Hostx64\x64`（用 `VCToolsInstallDir` 问出来）顶到最前，开跑前用 `where link.exe` 第一行验身份。**本地没有 Git 那套 `usr\bin`，永远复现不了** —— 要复现就自己造个假 `link.exe` 放进 `H:\tmp\faker\Git\usr\bin` 并 prepend 到 PATH |
| **批处理里 `%PATH%` 死活不生效** | **别把命令拼成 `cmd /c "a && b && c"` 长链**：cmd 把整条链**先解析、把 `%VAR%` 全展开**再逐条执行，所以链里 `set "PATH=...;%PATH%"` 拿到的是**启动 cmd 时的原始 PATH**，前面 `set`/`vcvars` 改的全白费（实测：剔掉 Git 段的 PATH 又被原样放回）。**改成写临时 `.cmd` 逐行执行**。另：`set "RUSTFLAGS=-C linker="C:\...\link.exe""` 的引号会原样传给 rustc，报 `os error 123`，**别用这条路**，把链接器目录顶到 PATH 最前就够了 |
| **`npm install` 在 CI 报 `Could not read package.json`** | **`npm install` 只在当前目录找 `package.json`**（不像 vite/tsc 往上找）。`build.ps1` 开头 `Push-Location $here`（= `app\desktop`）后直接 install 就会去找 `app\desktop\package.json`。**开发机永远暴露不了**（`node_modules` 早装好了，这句不跑）—— 改构建脚本后要按「干净 clone」的心智过一遍 |
| **`cargo install tauri-cli` 装完却找不到 `tauri`** | cargo 子命令的可执行文件叫 **`cargo-tauri.exe`**（带 `cargo-` 前缀），缓存 path 与存在性判断都按这个写；验证别猜文件名，直接 `cargo tauri --version` 真调一次 |
| **契约用例在 CI 上红，本地却全绿** | 夹具是**开发机上抓的冻结基准**，凡是记录「**这台机器上有什么**」而不是「**接口返回什么形状**」的用例，换台机器必然对不上。已登记两条：`video-parse-bili`（B 站对匿名/机房 IP 回 HTTP 412）、`fs-list-c`（`C:\` 根目录开发机 13 个、GitHub runner 37 个）。⚠️ 判「有意」的条件要**收得紧** —— 用 `onlyWhenLine` 把原始响应当证据，否则这个清单会变成垃圾桶 |

---

## 五之三、液态玻璃用现成的库，别自己写

**这条原则仍然成立** —— 用户提供过参考项目，就必须先问「能不能直接用」，
别照着原理自己实现（被用户当场指出过）。

⚠️ 具体用哪个库、怎么接，见 **`docs/GLASS-HANDOFF.md`**：现在用的是
`@ttqtt/liquid-glass-react`，不是这节原文写的 `rdev/liquid-glass-react`
（**两个同名包，完全不同的项目**，我装错过）。
**三条血的教训**（用户复报「除了侧栏都没有实现对应的玻璃材质」时定位到的）：
① **材质要写在 `GlassProvider` 上** —— 库的控件（`GlassButton` / `GlassSegmentedControl` /
`TabBar`…）**不接材质参数**、读的是 policy；只给自家包装的面传，切到液态玻璃就只有那两个面
变 `clear`、控件还是 `regular`，看着就是「只有侧栏有材质」。
② **背景自身的 `--bg-blur` 必须小（3~4px）** —— 背景先糊成奶白、玻璃再糊一次等于没糊；
**玻璃的观感来自「背后有东西被它糊掉」**，不是来自玻璃自己。
③ **栏本身不画底**（库的 `GlassToolbar` 注释：「它是一行分组，玻璃是每一组」）——
一整条大玻璃 + 一堆手写平控件 = 看不出材质；平栏要配 `ScrollEdge`，否则内容从文字下面穿过。
4. **玻璃是设置里的 1~4 级滑块**（库的 `GlassSlider`，键 `qingmu.glassLevel`）：
   1 级不透明、2 级毛玻璃（这两级内容面板用轻量材质）、**3 级液态 = 只有栏/侧栏/控件折射**
   （= `backup-pre-global-glass` 那一版，用户报过「一半液态玻璃的效果没了」，就是这档）、
   4 级连内容面板也折射。材质 / 透明度 / 面板要不要玻璃三件事全由这一档派生
   （`lib/useGlass.ts` 的 `level*()`），`Panel.tsx` 只看 `level >= 4`。

**上游源码仓库就在本机磁盘上**：`C:\Users\Administrator\Desktop\工作站素材\`，
含 `docs/design-system.md`（小玻璃/大玻璃、regular/clear 的适用条件）、组件源码、材质参数表。
**判断库的行为以它为准，别对着 `node_modules/dist` 猜。**
验证工具：`tests/manual/glass-probe.mjs`（取计算值 + 截图 + 高亮块逐帧/首帧采样）。

---

## 五之四 / 五之五、把一个外部 AI 项目嵌进来（两套完整经验）

**在 `docs/INTEGRATIONS.md`。** 这里只留结论索引 —— 那两节加起来 17 KB，
放进这一份会把它顶破 64 KB 的工作区指令预算（超了就被截断，**后面的节会直接看不见**）。

| | 音轨分离（五之四） | 人声转 MIDI（五之五） |
|---|---|---|
| 做法 | **包一个别人的 Python 服务**：子进程 + 固定端口 + 健康检查 + 作业对象收尸 | **把算法重写进 Rust**：进程内、零子进程、零端口 |
| 大件 | 运行时 4.7 GB + 模型 462 MB | 模型包 364 MB（解开 376 MB；运行时**能借**音轨分离那份 ORT —— 包括 GPU） |
| 代码 | `src/svsep.rs` `src/ipc/svsep.rs` `src/pages/Svsep.tsx` | `src/game/**` `src/midi_transcribe.rs` `src/ipc/midi.rs` `src/pages/Midi.tsx` |

**要嵌新东西之前，先读那两节里这几条**（它们是血换来的，别重走）：

1. **⛔ 先确认上游在 Windows 上真能跑。** 人声转 MIDI 的 MLX 版依赖
   `features = ["accelerate","metal"]`（macOS 专有框架），**在 Windows 上编不出来** ——
   光看仓库名看不出来，要读它的 `Cargo.toml`。
2. **⛔ 别把「推理图」想当然当成确定性的。** GAME 的 `segmenter.onnx` 里藏着一个
   `RandomUniformLike`，**它不能直接当数值 oracle**。先用 `RandomSpect…` 那类节点自查。
3. **⛔ 别靠形状推理判权重布局。** checkpoint 可能是 NHWC 风格（中间轴都是 1，
   `permute` 前后内存布局相同）—— 只能建两版模型做**数值比对**。
4. **⛔ 判「装没装」看代码真去找的那几个文件**，不是「包里文件挺多」。
5. **数值验证靠逐位比对，不靠读代码推演**：PyTorch 生成 golden → Rust 逐项比
   `max|diff|`；合成输入要选**已知物理量**（220 Hz 正弦解出 MIDI A3 = 57 半音，
   一条就证明了量纲与整条链路）。
6. **新功能一律要带三样界面**：装没装（自检）、怎么装（带进度的下载）、怎么卸（一键删除）。

---

## 六、资源库数据（`app/data/resources.json`）

前端资源库**唯一的数据源**，当前 4 个分组 / 27 条。字段含义看文件本身（自解释），
功能侧的读法见 `docs/FEATURES.md`。要记住的是**三条规则**：

1. **`verified.verdict` 三态**：`ok`(200/301/302/307/308) / `warn`(403/401/405/429) /
   `dead`(404/410/超时/DNS 失败)；其它 5xx 存疑、要人工确认。
   **403 不算失效** —— Musopen、Pixabay、Dreamtonics、爱给网、Booth 这类**完全正常**的站点
   会对脚本请求返 403 拦爬虫，把 403 当死链会误删大量好资源。
   重新校验：`node tests\manual\check-resources.mjs [--write]`（超时/连接错误自动重试 3 次）。
2. **收录原则**：只收官方 / 开源 / 免费或官方试用渠道，**绝不收破解版、学习版、激活器、
   注册机**，也不收网盘转载的盗版声库与编辑器。永久黑名单：瑟狐下载站、
   `pan.vocaloid.world`、`vocakey`（vocakey.wikidot.com）—— 收录即等于协助侵权，
   且这类来源无法验证安全性。
3. **`desc` 必须写实际价值**（对翻调工作流的用处），不写「这是一个音乐网站」这类空话。
   宁缺毋滥：校验不过的条目直接删，宁可 15 条真的，不要 30 条假的；**不伪造状态**。

商业产品**一个公司一条**（收了 CeVIO 就不再单列 KAFU），开源/免费项目**有一个收一个**。git log --all -- app/data/resources.json 里有 2026-09 从 7 组 121 条精简到 4 组 27 条的过程。

---

## 七、已知问题与未完成

| 事项 | 状态 |
|---|---|
| **打包 MSI** | ✅ **已真机装过并验证通过（2026-10-02）**：界面能开、设置改了重启还在、工程转换能跑、文字 PV 能开。安装布局与 `resolve_paths()` 对齐也已用 MSI 表核实 —— 见下 |
| UTAU Shift-JIS | 纯 Rust 侧不生成 Shift-JIS，默认写 UTF-8 |
| YouTube | 境内不可达，相关功能要走代理（设置页可配） |
| `mime_of` | 已补齐（2026-10-02）：`.jpg/.jpeg/.webp/.gif/.woff/.ttf/.mp3/.wav/.mp4/.txt/.map` 都有映射 |
| `backdrop-filter` 降级 | 无该特性环境的降级方案没做视觉验证 |
| `audio.rs` 顶部注释 | 写着「ffmpeg 不随程序分发」，与事实相反（注释是旧的） |
| Rust 代码行数 | README 里那些「约 5,900 行 / 31 条路由 / 56 条路由」**都是旧数字**。现为 `src/` 约 16,900 行、**70 条 IPC 命令**（`src/ipc/` 约 4,500 行） |
| **工程转换** | 选项键已改用 LibreSVIP **官方选项名**，VSQX 参数曲线崩溃已**自动降级**（16 个真样本 15 通过，剩下 1 个是源工程自身音符重叠）。⚠️ `音高信息输入模式` 默认档是官方 `plain`（≈ 只带"已编辑"部分），要完整保留手画音高就在选项面板选「完整」。选项表与实现见 `docs/FEATURES.md` §3.1 |

### 打包：CI 出 MSI，真机装过、验证通过

**2026-10-02**：`build.ps1 -Release -Bundle` 与 CI 都通了 —— 打版本 tag（`v[0-9]*`）就由
`.github/workflows/build-msi.yml` 自动出 MSI 并传 Release。成品
`V-Synth-Studio_1.2.0_x64_zh-CN.msi`（182.85 MB）在 <https://github.com/QingMu39-Gao/V-Synth-Studio/releases/latest>
上，**可以直接发给别人装**。

`tauri.conf.json` 的 4 条映射**写什么就落在哪（相对 INSTALLDIR）**：`../../app/web` →
`app/web`、`../../app/data/{resources,pinyin}.json` → 同名、`../../tools` → `tools`。

> ✅ **2026-10-02 读 Release `v1.2.0` 的 MSI 表核实过（不用真安装）：装出来就是上面那个布局。**
> `C:\Program Files\V-Synth-Studio\` 下直属子目录只有 `app\` 与 `tools\`；resolve_paths 找的
> 哨兵 `app\web\index.html`、`app\data\*.json`、`tools\{ffmpeg\bin\ffmpeg.exe,ffprobe.exe,
> yt-dlp.exe,libresvip\...\libresvip-cli.exe}` 都在。
> 两条关键事实：① **Tauri v2 不再加 `resources\` 前缀**（此前写「会平铺到
> `<安装目录>/resources/` 下」是 **Tauri v1 的行为，错的**）；② **`resource_dir()` 在 Windows 上
> 就是 exe 所在目录**，所以第 1 步 `has_web(resource_dir/app/web/index.html)` 直接命中，
> `is_writable` 在 Program Files 下为假 → 正确判成**安装版**，可写目录落到 `%APPDATA%`。
> ⚠️ 唯一取舍：**管理员装完又用管理员运行**时 `is_writable(安装目录\app\data)` 会真为 true →
> 当成绿色版、配置写进安装目录。
>
> 读 MSI 表的姿势：SQL 列名要**反引号**；`LIKE '%x%'` 一律报 `OpenView,Sql`（要过滤就整表取回
> 再用 `-match`）；`InvokeMember` 的返回值**每个都要 `$null =` 接住**，否则混进函数输出把
> `@(...)` 撑成假的 1-2 个元素（`Directory=1` 这种假数量就是这么来的）。

**实机安装验证（2026-10-02，用户反馈正常）**：界面能开 ✅；改设置 → 重启 → 还在 ✅（可写目录正确
落在 `%APPDATA%`）；工程转换 ✅；文字 PV ✅；ffmpeg 没单独验 ⬜（它与转换走同一套
`resolve_paths()`，转换能跑基本说明 `tools/` 定位没问题）。**安装版这条路是通的。**
⚠️ 万一安装版一启动就挂，看 `%APPDATA%\com.qingmu.vocalworkstation\desktop-error.log`
（`main.rs::error_log_hint()` 会把真实路径打在错误提示里）。

其余形态：Windows 绿色版也命中（同样靠 `is_writable` 区分）；开发机与 CI 靠「往上找」也成立 —— 所以「换掉往上找」不再是 Windows 的待办，它只为 macOS bundle 而做。

---

## 八、平台移植

平台相关代码**全部集中在 `app/desktop/src/platform.rs`** —— 移植时主要改这一个文件。

| 功能 | 现在（Windows） | macOS 需要 |
|---|---|---|
| 下载目录 | 读注册表 `User Shell Folders`，退回 `USERPROFILE\Downloads` | `$HOME/Downloads`（更简单，删掉注册表那段） |
| 文件管理器定位 | `explorer /select,` | `open -R`（**已写好 cfg 分支**） |
| 打开文件 | `explorer` | `open`（同上，已分支） |
| 回收站 | PowerShell `Shell.Application` | `trash` 命令或 `NSFileManager` |
| 平台名 | `node_platform_name()` 返回 `win32` | 已按 Node 命名返回 `darwin`，不用改 |
| 路径规范化 | 剥 `\\?\` 前缀 | `clean_path()` 里的 `#[cfg(windows)]` 块自然跳过 |
| 无窗口子进程 | `quiet_command()` 设 `CREATE_NO_WINDOW` | 该标志不存在，函数已有 cfg 分支 |
| 注册表读取 | 下载目录读取用 `#[cfg(windows)]` 隔离 | 自动跳过 |

**外部工具**（ffmpeg / yt-dlp）走 `find_binary()` + PATH，逻辑本身跨平台。
但它们现在**随包分发**，macOS 版需要换成对应平台的二进制。

**两处要注意**：

1. `src/tools.rs` 的编辑器路径表**已经清空**（2026-10-02：离线分离改成内嵌引擎，UVR 那条
   候选删了，`candidates()` 现在返回 `Vec::new()`）。要重新加编辑器探测时，别照 Windows
   专有路径写死 —— macOS 上要么换成 `/Applications/*.app` 扫描，要么直接去掉。
   这是**数据**不是逻辑，改动很小。
2. 打包见第七节。

**构建脚本不跨平台**：`build.ps1` 是 PowerShell，只能在 Windows 跑。
macOS 需要另写一个薄壳（`vite build` 那类跨平台步骤两边一样，但 vcvars 那步 Mac 上没有）。
**不要为了「一个构建入口」去引 task runner** —— 两个平台两个壳，各十来行。

**Android 是另一回事**：入口是 `tauri android build`（要 SDK + NDK + Gradle），**十节的表已经把依赖出路查清了**。真正的坎是后端靠「起外部进程」（`Command::new` 调 ffmpeg / yt-dlp / LibreSVIP），而 Android 数据目录 noexec、装了也跑不起来 —— 第一步必须**把媒体能力抽成抽象层**（桌面 `Command`、移动 JNI），否则就是复制一整个后端（Node 后端那 19,019 行就是这么来的）。

---

## 九、历史（留档）

- Node 后端（`app/server/`，19,019 行）已整体删除，其中包含 12 个格式模块和一套平台探测代码。
- 格式转换原取自 UtaFormatix3 的模板，现已全部移除（相关代码与参考文件一并删除）。
- 声库探测（784 行）已删 —— 只被用来「显示装了什么」，转换路径从没调用过。
- 编辑器探测从 16 个砍到只剩 UVR，2026-10-02 连 UVR 那条也删了（分离改成内嵌引擎）。
- 网易云扫码登录已移除：服务端返回 `8821 请切换其他登录方式`，按官方 JS 逐字节对齐
  三处仍失败，判断是服务端风控。**别再试图修它**，留了手机号验证码 + Cookie 两条路。

移植历史与旧实现见 git：

```
git log --all -- app/server
git log --all -- app/data/resources.json
```

### ⚠️ 2026-10-02 重写过一次历史（有大件被从历史里剔掉）

仓库从 **83 MB 缩到 6.7 MB**：`app/web/vendor/`（JIZURA 字体，2338 个文件 / 50.7 MB）、
`tests/manual/out/`（探针截图 78 个 / 23.7 MB）、`app/shell/`（旧 WebView2 dll）
用 `git filter-branch --index-filter` **从全部历史里删掉了**（本就不在 HEAD 上）。

**三条要记住的**：① **所有提交 SHA 都变了**（`v1.2.0` 从 `48049c4` 变 `ca0ff84`、
`assets-v1` 从 `341d234` 变 `9f03e02`）—— 旧 SHA 已不存在，别照着 `git show`；
tag 是 force push 的，Release 靠 tag 名绑定、**附件没丢**。② `git log --all -- app/web/vendor`
（或 `tests/manual/out`、`app/shell`）**现在恒为空** —— 不是命令写错，是历史里没有了；
`app/server`、`app/web/js` 这些小体积历史**故意留着**。③ **`vendor/jizura/` 磁盘上还在**
（靠 `fetch-tools.ps1` 补），只是不入库。

---

## 十、下一步

**已定的方向**（用户已拍板，不要再问）：

1. **三端适配**：Windows + macOS + Android，**功能对等**。但**优先 Windows**，
   Android 是后续；唯一要求是「别把门焊死」。
2. **前端换 React + Vite + Tailwind**，UI 控件层全部替换，**保留离线运行**。
   ⚠️ 原计划的 **shadcn/ui 不再引**：`@ttqtt/liquid-glass-react` 自带 60+ 个控件，
   再叠一层 UI 库只会打架，而且 shadcn 初始化要联网，与离线目标相冲。
   **不用 React Native** —— WebView + 同一套 React，三端共用一份 UI。
3. **全局毛玻璃材质** + **设置里可开关性能模式**（开启即全局取消毛玻璃）。

### 已完成（2026-09 起）

| 事项 | 说明 |
|---|---|
| 改名 V-Synth-Studio | 界面可见处全改；exe 变 `v-synth-studio.exe`；存储键与配置目录**刻意不动** |
| 图标全套 | 由 `图标.png` 生成（Win/macOS/Android/iOS），`cargo tauri icon --fit contain` |
| 前端脚手架 | `app/web-next/`（React 19 + Vite 8 + TS 7 + Tailwind 4），产物落 `app/web/`，访问 `/` |
| **玻璃材质修好**（2026-10-01） | 默认毛玻璃、侧栏 `size="large"`、面板 `thin`、高亮块用库的透镜、补回 `corner-shape: squircle`，细节在 `GLASS-HANDOFF.md` §2 |
| **顶栏只留品牌 + 改回吸顶**（2026-10-01/02） | 右上角控件（材质分段 / 重新检测 / 状态文字）移除、换真图标、材质切换挪到设置页；用户报「往下滚动品牌会跟着跑」后顶栏 `absolute` → `sticky`。⚠️ **`inset-inline` 必须写 `var(--lg-margin)`**（绝对定位的包含块是内边距盒，写 0 偏左 20px）、**侧栏 `top` 写多少吸顶后 `y` 就是多少**（实测表在 `index.css` 注释）。有意副作用：顶栏真占位，下方内容整体下移 42px |
| **`glass-probe.mjs`** | 玻璃专项探针：计算值 + 截图 + 高亮块逐帧/首帧采样 |
| **`app/web-next` 入库** | 首次提交 `83320cd` —— 在此之前它一个 commit 都没有 |
| **启动加载画面**（2026-10-02） | `#boot` + `lib/boot.ts`；遮罩淡出与界面入场**交叉**（见 `GLASS-HANDOFF` §4.1） |
| **玻璃等级 1~4 滑块** | 材质 / 透明度 / 面板要不要玻璃全由这一档派生（`lib/useGlass.ts`），键 `qingmu.glassLevel` |
| **滑条动画 + `Panel` 只换材质不重建**（2026-10-02） | ① 库没给 `.lg-slider-lens` 做 transition（`index.css` 补，含为什么必须 `!important`）；② **主因**：`Panel` 原在两个组件**类型**间切换（`MaterialView` ↔ `GlassLayer`），React 到类型边界整棵重建，新 lens 一出生就带终态 —— 改成两档都渲染 `MaterialView`、只用类名切材质，⚠️ **子树形状也要一样**。根因在 **`GLASS-HANDOFF.md` §2.5** |
| **设置页小节导航复用主侧栏那套** | `lib/useNavLens.ts` + `.app-nav` / `.nav-row` / `.nav-lens`，两处外框参数逐项相同 |
| **9 页全部搬到 React**（2026-10-02） | 旧 `views/*.js` → `pages/*.tsx`（约 7,200 行）；顺带修掉旧前端 4 处接口契约错误（`docs/FRONTEND.md` 第 5 节） |
| **`next-smoke.mjs`** | 逐页冒烟：控制台报错 / 占位页 / 玻璃面 / 该页文案，9/9 全绿 |
| **音轨分离一页**（2026-10-02） | 在线 MVSEP + 离线内嵌引擎（Python 子进程，`docs/FEATURES.md` §3.11）；引擎 / 模型不随包分发，按需下载、可暂停续传、可一键删除 |
| **移植 playbook：五之四**（2026-10-02） | 音轨分离「怎么嵌外部 AI 项目」的完整经验写成 **五之四**（自包含，可直接交给下一个对话去做「AI 自动扒谱」） |

### ✅ 已完成：歌词页做成「网易云专栏」（2026-10-02 落地）

用户原话：「我打算 把歌词页面做成网易云专区 让用户可以搜索歌曲后直链下载歌曲
甚至是歌曲封面（记得把填写QQ音乐cookie的功能删掉）」

四件事全部落地 —— **实现细节与实测证据全在 `docs/FEATURES.md` §3.4，改这块先读它**：
① `source` 概念整体删掉（前端分段与 `SOURCES`/`isQq`/`loginKey`/`sourceLabel` 全没，
后端 `normalize_source()` 删除、`ipc/lyrics.rs::source_of()` 永远返回 `"netease"`，
QQ 那整条链 `QQ_UA`/`qq_search`/`qq_fetch`/`html_unescape`/songmid 解析/相关单测全删、
**应为 0 命中**；回包里的 `"source"` 字段**保留**，形状冻结）；
② 歌曲直链下载 `lyrics_song`（`{id,outDir?,name?}` → `{path,name,size,level,format}`，
**拿不到直链回 400 不回 500**）；
③ 封面修掉 7 MB 原图（`cover_url()` 统一拼 `?param=500y500`，3000×3000 / 7.1 MB → 249,916 B）；
④ 删掉「填写 QQ 音乐 cookie」。

**三条容易再踩的**（详情在 §3.4）：

- **`level` 与 `encodeType` 一个都不能少**：老接口 `player/url?id=X&ids=[X]&br=320000` 七首里只
  1 首给 url，换成 `enhance/player/url/**v1**?ids=[X]&level=exhigh&encodeType=mp3` 后 6 首全通。
  **不存在第三个端点**。⚠️ **判据只有「接口有没有给到 url」，`fee` 只当标签。**
- **音频下载超时要单独给**：`client()` 的 20 秒总超时太短（实测 9.8 MB / 320 kbps 要 96 秒，
  掐断后报含糊的 `error decoding response body`）→ 音频走 `media_client()`（600 秒 + read_timeout 60）。
- **封面字节是 PNG**（URL 却叫 `.jpg`）、`picUrl` 是 **`http://`** 开头 —— **别改成只认 https**，
  存文件时嗅探魔数或固定存 `.png`。⚠️ `default_config()` 里删 `qqCookie` 是安全的：
  `load_config()` 只认默认值里有的键，老用户残留会在下次 `save_config` 整份回写时清掉。

**别重走的死路**：`eapi/*` 回 `Content-Length: 0`；`weapi/*` 不带 `encSecKey` 回空 body；
第三方解析站（侵权灰产）。**匿名会话不触发网页取链** —— 未登录打开歌曲页时
`enhance/player/url` 这个请求根本不发出，「抓包学网页」对未登录态没用。

### 待办，按优先级

1. **`resolve_paths()` 改用 `resource_dir()`** —— ⚠️ **优先级已下调**：Windows 安装版与绿色版
   都已核实没问题（见第七节），现在这条只为 **macOS bundle** 和「省掉构建脚本复制 exe 那步」而做。
2. **把剩下几处手写控件换成库的**：`Button`（已是 `GlassButton`）和 `List`/`Dialog`/`Slider`/
   `Progress`/`Badge`/`Switch` 都在用了，但 `Field.tsx` 的输入框、页面里的 `.seg` / `.input`
   还是手写的（只有材质走库）。库有 `TextField`（`multiline`）/ `Picker` / `RadioGroup` /
   `GlassSegmentedControl`（胶囊、可拖、拖动中实时更新）。**换的时候注意**：库的分段控件是
   `<label class="lg-segment"><input type=radio>`，`aria-label` 挂在内层 `.lg-segmented-track`
   上 —— 别在外层 `.lg-segmented` 上取它。
3. **给前端补点击穿透测试** —— 自动化现在只到「渲染 + 文案 + 控制台」，真实操作链路（选文件 → 预检 → 提交任务）靠人工 + 探针截图。
4. **侧栏形态要不要换成库的 `TabBar`？** 它自带透镜、拖拽换页、窄屏自动变底部胶囊栏，
   但它的侧栏形态是 `position: fixed` 的整列贴窗口左边，而且**没有分组标题**
   （现在的「工作台 / 素材获取 / 系统」是手写的）。两条路都成立，**属于要用户拍板的结构选择**。
5. **CI**：已落地（`.github/workflows/build-msi.yml`，Windows 单平台出 MSI + 冒烟）。matrix 以后再谈：macOS 那格要等第八节的构建壳，Linux 编不出 Windows / macOS 的 GUI 包。

### 关于 Android（已核实，不用再查）

| 依赖 | Android 出路 | 状态 |
|---|---|---|
| ffmpeg | [ffmpeg-kit-maintained](https://github.com/ffmpegkit-maintained/ffmpeg-kit)（FFmpegKit 退役后的社区续作，改 group ID 即迁移） | 可用 |
| yt-dlp | [yt-dlp-android](https://github.com/ffmpegkit-maintained/yt-dlp-android)（Chaquopy 内嵌 CPython 3.13，进程内跑纯 Python） | 可用；AAR 60–80 MB |
| LibreSVIP | 未验证 | **不构成风险** —— 用户已明确「工程转换实现方式有很多」 |

约束：Android 数据目录 noexec，必须走 JNI 从 `.so` 加载；调用链的 `tools_dir` 形参已一路穿好，将来是机械替换而非重写。
