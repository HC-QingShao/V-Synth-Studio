# V-Synth-Studio

**翻调用的桌面工作台。** 从「我想翻这首歌」到「工程能交给声库唱了」，中间那些琐碎活儿
—— 找伴奏、下 MV、扒工程、转格式、取歌词、做歌词视频 —— 全在这一个窗口里做完。

> 原名「清沐的虚拟歌姬工作站」，2026-09 更名。作者署名 QingMu39（不是软件名）。

**下载安装包**：<https://github.com/QingMu39-Gao/V-Synth-Studio/releases/latest>

---

## 目录

- [这是什么](#这是什么)
- [安装](#安装)
- [上手：一条完整的翻调流水线](#上手一条完整的翻调流水线)
- [十页功能](#十页功能)
- [支持的格式](#支持的格式)
- [运行要求与体积](#运行要求与体积)
- [关于「破解版 / 学习版」](#关于破解版--学习版)
- [从源码编译](#从源码编译)
- [开发与验证](#开发与验证)
- [目录说明](#目录说明)
- [分发与授权](#分发与授权)

---

## 这是什么

一个**完全离线**的 Windows 桌面程序。所有处理都在你本机跑，工程文件、音频、歌词
都不上传到任何地方。

需要联网的只有三件事，用不用由你决定：**视频解析下载**（去 B 站 / YouTube 取东西）、
**在线音轨分离**（把音频传到 MVSEP，不想传就走离线那条路）、以及**第一次使用某些功能时
下载依赖**（音轨分离的引擎与模型、人声转 MIDI 的模型，装完就一直在了）。

技术上是一个 Tauri 2 程序：原生窗口 + 同进程的 Rust 后端，界面是 React（构建时打包进 exe）。
前端调后端走 **Tauri IPC**（70 条命令，没有 HTTP 服务、不监听任何端口）；
本地音视频播放走 Tauri 内置的 **asset 协议**（拖进度条能用，因为 Range 是内置的）。

**运行时不需要 Node、Python、VC++ 运行库** —— 唯一的前置条件是 WebView2
（Win11 和较新的 Win10 都自带）。

---

## 安装

1. 打开 <https://github.com/QingMu39-Gao/V-Synth-Studio/releases/latest>
2. 下载 `V-Synth-Studio_x.y.z_x64_zh-CN.msi`（约 180 MB），双击安装。

装完直接能用，不联网也行。

**配置存在哪儿？** 安装版放在 `%APPDATA%\com.qingmu.vocalworkstation\`。
如果你把程序整个文件夹拷到 U 盘或别的机器上跑（绿色版），配置就跟着文件夹走
（`app\data\`）—— 判断依据是「那个目录能不能写」，你不用管。

> 万一启动就闪退，去看 `%APPDATA%\com.qingmu.vocalworkstation\desktop-error.log`，
> 错误提示里会写出真实路径。

---

## 上手：一条完整的翻调流水线

下面是一条真实会走的路，每一站对应左侧栏的一页。你不必全走，挑需要的用。

**① 找到伴奏** —— 打开「音轨分离」，把原曲拖进去，选「二轨（UVR MDX）」，
十几分钟后拿到人声和伴奏两条 WAV。不想把音频传到网上就用这个（第一次要先下
引擎和模型，见下）；不介意上传的话，同一页右边还有「在线 MVSEP」那条路。

> 第一次用离线分离要下两包东西：**运行时**（压缩包 4.7 GB，解压 7.4 GB）和
> **模型**（压缩包 462 MB，解压 730 MB）。下完就一直在了，升级程序也不用重下。
> 网慢的话可以中途「暂停」，它会把已经下好的部分留在磁盘上，下次**接着下**
> （关掉程序再打开也认）。不想留了就点「删除全部依赖」一次清干净。

**② 顺手拿素材** —— 「视频解析」页贴 B 站链接，下 MV 画面、封面、弹幕、字幕。
「音频工具」页拿 ffmpeg 转格式、裁剪、变调变速、响度归一化。

**③ 弄到工程文件** —— 两条路：
- 手上有别人的工程（`.vsqx` / `.ustx` / `.svp` …）→「工程转换」直接转成你要的编辑器格式；
- 只有 MIDI 或没有工程 →「资源库」里的 MIDIshow 之类下个 `.mid` 回来，一样能转成工程。

**④ 取歌词** —— 「网易云专栏」搜歌名，选中就能拿到歌词，双语一起导；
顺带能把**封面**和**歌曲音频**（mp3 直链）一起下下来。本地 `.lrc` 也能导入
（GBK 老文件自动识别）。导出 LRC 或 SRT。

**⑤ 做歌词视频** —— 「文字 PV」把上一步的歌词一键带进来，套模板、挑字体，
导出 MP4 或 PNG 序列。字体已经离线打包好了，不用联网。

**⑥ 干声扒谱（可选）** —— 上一步拆出来的人声可以丢进「人声转 MIDI」，
直接扒成音符导出 `.mid`。⚠️ **它很慢**：本机纯 CPU 实测约 **10 秒墙钟换 1 秒音频**
（3 分钟干声就是半小时），耗时几乎与「去噪步数」成正比。第一次用要下模型
（压缩包 364 MB，解开 376 MB）；装了音轨分离的话 ONNX Runtime 直接借它那份，不用另下。

**⑦ 开始调** —— 工程丢进你自己的编辑器（VOCALOID / SynthV / OpenUtau / …）。

---

## 十页功能

| 页 | 干什么 | 用什么做 |
|---|---|---|
| **总览** | 环境自检 + 常用入口 | — |
| **工程转换** | 40 种工程格式互转 | 内置 LibreSVIP CLI，纯离线 |
| **视频解析** | B 站原生解析 + yt-dlp 兜底（上千站点） | 可选下封面 / 弹幕 / 字幕；多线程分块下载；试听先缓存到本机再播（同一支看第二次瞬时） |
| **音轨分离** | 拆人声 / 伴奏 / 鼓 / 贝斯 / 钢琴 / 其它 | 在线 MVSEP（要上传）**或**离线内嵌引擎（不出本机） |
| **人声转 MIDI** | 干声扒谱，导出 `.mid` | 内置 ONNX 推理（GAME 的 Rust 移植），进程内、无子进程 |
| **音频工具** | 格式转换、变调变速、裁剪、响度归一化、波形编辑 | 内置 ffmpeg |
| **网易云专栏** | 搜歌、取词、导 LRC/SRT、下封面、下歌曲 | 网易云官方接口，可选登录（手机号验证码或 Cookie） |
| **文字 PV** | 歌词做成动态歌词视频 / PNG 序列 | 内置 JIZURA，字体全离线 |
| **资源库** | 4 组 27 条：工程分享、免费音源、编辑器官网、UTAU 系开源 | 只收录链接，不转载文件 |
| **设置** | 外观与材质、路径、外部工具 | — |

**几个容易踩的点**：

- **网易云专栏**：搜索结果里标了「能下载 / 不能下载」。标 `VIP` 只是收费标签，
  **不等于下不了** —— 真正能不能下，按下「下载歌曲」那一刻才知道。碰上不能下的，
  换一条同名的再试（列表里那个「换一个能下的版本」就是干这个的）。
- **音轨分离的进度是估的**：百分比按本机纯 CPU 实测线性外推，
  会长时间停在 90% 再跳到 100%。看到不动不用重试，底下有「已用时 N 秒」。
  装了 NVIDIA 显卡会快很多（人声转 MIDI 也能借上这份 CUDA）。
- **人声转 MIDI 很慢**（见上），去噪步数是那个「一定有效」的旋钮。
- **在线 MVSEP 会上传你的音频**，页面里写明了。不想上传就走离线那条。
- **视频解析受代理影响**：B 站对境外出口 IP 会回 `HTTP 412`（风控）。
  如果挂着全局/TUN 模式的代理，把 `bilibili.com` / `bilivideo.cn` 加进直连规则。

---

## 支持的格式

工程转换支持的 **40 种格式**：

| 类别 | 格式 |
|---|---|
| VOCALOID | `.vsqx` `.vsq` `.vpr` `.vog` `.vspx` |
| Synthesizer V | `.svp` `.s5p` |
| UTAU / OpenUtau | `.ust` `.ustx` |
| CeVIO / ACE / DeepVocal | `.ccs` `.acep` `.dv` `.dspx` |
| 通用交换 | `.mid` `.musicxml` `.ufdata` |
| 歌词字幕 | `.lrc` `.ass` `.srt` `.svg` |

> 读工程是借道 LibreSVIP 导出的 `ufdata`（一种 JSON 中间格式）——
> 这样它支持的格式我们都能读，只需要解析一种结构。

**转换前会预检**：先告诉你目标格式装不下哪些数据（音高曲线、多轨、歌词），
再让你决定要不要转。

---

## 运行要求与体积

用户只需要 **WebView2 运行时** —— Win11 和较新的 Win10 都预装。

| 依赖 | 用户要装吗 | 说明 |
|---|---|---|
| WebView2 | **要**（多半已自带） | 唯一的硬依赖 |
| VC++ 运行库 | 不要 | 已静态链接 |
| Node.js | 不要 | 只在开发者编译前端时用 |
| Python | 不要 | 离线分离引擎自带一份便携运行时 |

**为什么安装包约 180 MB？** 因为 ffmpeg、yt-dlp、LibreSVIP、JIZURA 与 2335 个字体
（合计约 340 MB 未压缩）都**随包分发**，装完不联网也能用。主要在国内使用，
让用户自己去 GitHub 下 ffmpeg 基本下不动。

**音轨分离与人声转 MIDI 的引擎 / 模型不在安装包里**，第一次用时按需下载：

| 依赖 | 压缩包 | 解压后 | 谁用 |
|---|---|---|---|
| 分离运行时（Python + torch） | 4.7 GB | 7.4 GB | 离线音轨分离 |
| 分离模型（BS-RoFormer / UVR MDX） | 462 MB | 730 MB | 离线音轨分离 |
| 人声转 MIDI 模型（GAME ONNX） | 364 MB | 376 MB | 人声转 MIDI |
| ONNX Runtime | 78 MB | 只用其中 15 MB 的 dll | 人声转 MIDI（**装了分离就能借，不用下**） |

这不是偷懒：加上它们安装包会到 5 GB 以上，而且大部分人用不到。
⚠️ 人声转 MIDI 的权重许可是 **CC BY-NC-SA 4.0（非商业）**，所以它只按需下载、
不随包分发。

---

## 关于「破解版 / 学习版」

资源库**不收录**任何破解、激活器、注册机、网盘转载的盗版声库或编辑器。

原因不是保守，而是这类东西在原理上无法验证安全性：没有数字签名、二次打包、
经常捆绑启动器，是木马和挖矿程序的高发区。而且翻调本来就有很多正经的免费选择
—— UTAU、OpenUtau、DiffSinger、NNSVS、VOICEVOX、NEUTRINO 这一批开源项目
资源库里都收了，零成本就能起步。

只收官方、开源与免费试用渠道。有永久黑名单，收录即等于侵权来源。

---

## 从源码编译

需要 **Rust + MSVC 工具链 + Node**（Node 只在编译前端时用）。

```powershell
# ① 补齐两个大件（从本仓库 Release 的 assets-v1 附件下载，约 340 MB）
powershell -ExecutionPolicy Bypass -File app\desktop\fetch-tools.ps1

# ② 编译（第一步 npm run build 编前端 → cargo build → 把 exe 复制到根目录）
powershell -ExecutionPolicy Bypass -File app\desktop\build.ps1

# ③ 双击运行
启动工作站.bat
```

`build.ps1` 是**唯一的构建入口**。常用参数：

| 参数 | 作用 |
|---|---|
| `-Release` | release 版（体积小、跑得快、无控制台窗口） |
| `-Bundle` | 出 MSI 安装包，要和 `-Release` 一起用 |
| `-SkipWeb` | 只编后端，跳过前端（改 Rust 时省几秒） |
| `-FetchTools` | 先补齐 `tools/` 与 JIZURA 字体 |
| `-NoCopy` | 编完不复制 exe 到根目录（CI 用） |

> ⚠️ **别直接用 `cargo build`** —— 它只编译、**不复制** exe，
> 根目录那个 `v-synth-studio.exe` 会悄悄停在旧版本，你会以为改动没生效。

> ⚠️ **改前端也要重跑 `build.ps1`。** 前端产物（`app/web/`）是在编译时**打包进 exe** 的，
> 所以 `npm run watch` 只更新磁盘上那份、不影响你打开的窗口 —— 想看到效果必须重编。

> ⚠️ **`-Bundle` 之前先确认 `tools/`、`app/web/vendor/jizura/`、`app/web/index.html` 都在**，
> 否则 `bundle.resources` 会**静默少打包**，缺的东西要到用户手里才现形（脚本里有防线）。

打 tag 推上去会由 GitHub Actions 自动出 MSI：

```
git tag v1.3.0 && git push origin v1.3.0
```

### 仓库只放源码，两个大件编译前补齐

仓库约 7 MB，是有意为之。下面两块不属于源码，但程序要离线可用就必须在打包前到位：

| 大件 | 体积 | 补齐后落在 | 谁在用 |
|---|---|---|---|
| ffmpeg + yt-dlp + LibreSVIP CLI | 约 288 MB | `tools/` | 转换 / 下载 / 音频处理 |
| JIZURA 与 2335 个 woff2 字体 | 约 54 MB | `app/web/vendor/jizura/` | 文字 PV 页（**离线可用靠它**） |

`fetch-tools.ps1` 优先从本仓库 Release（tag `assets-v1`）的附件取
`tools.zip` 与 `jizura.zip`；拿不到存档时退到三个上游官方地址现下
（gyan.dev / yt-dlp release / LibreSVIP release）—— 慢，但不用人去别处找。
想用本地已有的包：`fetch-tools.ps1 -Local 'D:\存着两个 zip 的目录'`。

---

## 开发与验证

```powershell
# Rust 单测（当前 93 passed / 0 failed）—— 这是主验证
cd app\desktop; cargo test --bins

# 前端类型检查 + 构建
cd app\web-next; npm install; npm run build
```

**起一个真窗口并用 CDP 探针问它**（表达式都写在探针文件里，命令只传端口）：

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9349'
$app = Start-Process -FilePath '.\v-synth-studio.exe' -PassThru
Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
Start-Sleep -Seconds 9

node tests\manual\window-check.js 9349          # origin / IPC 在不在 / 渲染了多少
node tests\manual\cdp-window-check.mjs 9349     # 10 页冒烟：控制台报错 / 玻璃面 / 文案
node tests\manual\ipc-window-data-check.mjs 9349 # 逐页验「正文里有没有真数据」

Stop-Process -Id $app.Id -Force                 # 收尾：只能按精确 PID
```

> ⚠️ **别再按进程名或命令行子串杀进程** —— 曾经误杀过别的工具；也**别用
> 「杀掉所有无窗口的 msedge」那招**，用户开着 Edge 时那会连他浏览器的后台进程一起收。

几处约定（改代码前值得知道的）：

- **`build.ps1` 是唯一构建入口**，直接 `cargo build` 不算。
- **前端只能通过 `invoke('命令名')` 找后端**。命令清单的唯一登记处是
  `app/desktop/src/main.rs` 的 `generate_handler!`（70 条）。**没有 HTTP 服务、没有路由表。**
- **本地媒体一律 `convertFileSrc(path)`**（asset 协议），别自己起服务、也别手搓自定义协议。
  新目录要先放行（`allow_path`），否则 `<video>` / `<audio>` 会**静默** 403。
- **配置只有一份真相**：`config.json`（绿色版 `app\data\`，安装版 `%APPDATA%`）。
  前端不再往浏览器存储里写设置。
- 仓库里**有意不放开发笔记** —— 这个项目的约定与踩过的坑都写在源码注释里
  （每个文件头部都会说「为什么长这样」「哪些做法会静默出错」），改动前先读那个文件的注释。

---

## 目录说明

| 路径 | 说明 |
|---|---|
| `app/desktop/` | Tauri 外壳 + Rust 后端（构建脚本也在这里） |
| `app/desktop/src/ipc/` | **前端唯一能碰到后端的入口**：70 条 IPC 命令 |
| `app/web-next/` | 前端源码（React + Vite + TS + Tailwind） |
| `app/web/` | 前端产物 + 随包静态资源（`index.html` 也是程序根目录的哨兵文件） |
| `app/data/` | 资源库、拼音词典；绿色版的配置也落在这儿 |
| `tools/` | 随包分发：ffmpeg / LibreSVIP / yt-dlp（**不入库**，用 `fetch-tools.ps1` 补） |
| `tests/manual/` | 探针脚本（CDP 问真窗口） |
| `tests/samples/` | 转换用的合成工程夹具 |
| `docs/` | `THIRD-PARTY-NOTICES.md`（第三方许可与合规） |
| `.github/workflows/` | CI：编译 + 打 MSI + 冒烟 |
| `资料归档/` | 上传 Release 用的大存档（**不入库**） |

---

## 分发与授权

随包分发了几个独立的外部程序（FFmpeg / LibreSVIP / yt-dlp / JIZURA / 字体），
各自的许可与合规要求见 **[`docs/THIRD-PARTY-NOTICES.md`](docs/THIRD-PARTY-NOTICES.md)**。

> ⚠️ 当前 `tools/ffmpeg/` 是 **GPL v3** 构建。分发前请先读那份文档 ——
> 换成 LGPL 构建可以省掉大部分合规负担，而且不影响本程序的功能。

程序本身由 QingMu39 开发，界面素材用 [@ttqtt/liquid-glass-react](https://github.com/Tsdsj/liquid-glass-react)
（MIT，按 Apple 设计语言做的独立组件库，不是 Apple 官方产品）。

---

## 致谢

感谢所有赞助者与测试者 —— 完整的感谢名单在程序里的「设置 → 关于」。

这个项目由 DeepSeek、Claude 等智能体辅助开发。
