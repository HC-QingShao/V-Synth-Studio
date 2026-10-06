# V-Synth-Studio —— **一个专为 P 主制作的本地工作站。**

> ## ⚠️ 完全测试版（Beta）
>
> **全部功能都还在测试与调整中**：可能有 bug、接口可能改动、极端情况下可能丢数据。
> **用之前请先备份工程文件**，别把它当成唯一的工作副本。
> 遇到问题欢迎到 Q 群（设置 → 关于）或 [Issues](https://github.com/QingMu39-Gao/V-Synth-Studio/issues) 反馈。

**全部离线运行**：工程文件、音频、歌词都不出本机；程序不开端口、不起本地服务、运行时不依赖
Node。需要联网的只有那几件本来就要网的事（视频解析、歌词搜索、在线音轨分离），以及第一次
按需下载的两个大件（见「安装」）。

## 功能

| 页面 | 做什么 |
|---|---|
| 总览 | 环境检测（外部工具、扩展包装没装）与常用入口 |
| 工程转换 | **40 种工程格式互转**，批量、可选输出目录，转换前先告诉你哪些数据会丢 |
| 视频解析 | B 站原生解析 + yt-dlp 兜底（YouTube 等上千站点）；封面 / 弹幕 / 字幕，多线程分块下载，试听先缓存 |
| 音轨分离 | 拆人声 / 伴奏 / 鼓 / 贝斯 / 钢琴 / 其它；在线 MVSEP 或本地引擎。**A 卡 / 核显走 DirectML，N 卡走 CUDA** |
| 人声转 MIDI | 干声扒谱导出 `.mid`。GAME 的算法重写进 Rust，进程内推理、无子进程；有 N 卡时自动走 GPU |
| 音频工具 | 格式转换、变调变速、裁剪、响度归一化、波形编辑（内置 ffmpeg） |
| 网易云专栏 | 搜歌、取词、导 LRC/SRT、下封面、下歌曲（可选登录：手机验证码或 Cookie） |
| 文字 PV | 歌词做成动态歌词视频 / PNG 序列（内置 JIZURA，离线可用） |
| 资源库 | 工程分享、免费音源、编辑器官网、UTAU 系开源 —— **只收录链接**，不转载文件 |
| 设置 | 玻璃材质 / 主题 / **背景壁纸** / 路径 / 外部工具 / 关于 |

**背景壁纸**：可以把你自己 Steam 库里 Wallpaper Engine 的壁纸当界面背景（场景 / 视频 / 图片
都行）。只读你本机已有的文件 —— 不下载、不打包、不上传任何壁纸；场景壁纸在沙箱 iframe 里
渲染，壁纸自带的脚本碰不到程序本体。

## 安装

到 **[Releases](https://github.com/QingMu39-Gao/V-Synth-Studio/releases/latest)** 下载：

- `v-synth-studio_x.y.zbeta_x64_zh-CN.msi` —— 双击安装（推荐）
- `v-synth-studio_x.y.z_x64-setup.exe` —— 同一个程序的 exe 安装程序，按用户安装、不需要管理员

也可以直接用绿色版：把 `v-synth-studio.exe` 和 `data/`、`tools/` 放在同一个目录里，双击就行，
配置写在 `data/config.json`，整个目录可以拷着走。

> 文件名里的 `beta` 表示这是测试版。界面「设置 → 关于」里也写着版本号（如 `1.3.2beta`）。
>
> 安装包**没有代码签名**，第一次运行会看到一次 SmartScreen 提示：点「更多信息」→「仍要运行」。
>
> 装完只有「外部工具」是齐的。**音轨分离**和**人声转 MIDI** 的引擎与模型不随包分发（合计约
> 8 GB，其中模型权重是非商业许可、不允许随源码分发）——在对应页面点「安装扩展包」按需下载，
> 支持暂停与续传，也能一键删除。

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

## 自己编译

需要 **Node ≥ 20.19**（Vite 8 的要求）、**pnpm**、**Rust（MSVC 工具链）**。

```bash
pnpm install
pnpm exec tauri build          # 出安装包；只要 exe 就加 --no-bundle
```

两块**不入库**的大件（`tools/` 约 183 MB、`public/vendor/jizura/` 约 54 MB）由
`pnpm exec tauri build` 的 `beforeBuildCommand`（= `pnpm prepare:assets`）自动补齐，顺序不能反：
字体要先进 `public/`，Vite 才可能把它拷进 `dist/`，`dist/` 才可能被嵌进 exe。**反过来不报错**，
只会打出一个缺字体、缺工具的包。

补件有三条来源，按顺序试：`--local <目录>`（完全不联网）→ 自建归档（`tools/assets.mjs` 的
`SELF_HOST_BASE`）→ 上游（GitHub / Google Fonts）。别人给你的两份归档这么用：

```bash
VSS_ASSETS_LOCAL=<放 tools.zip 与 jizura.zip 的目录> pnpm prepare:assets
```

产物在 `src-tauri/target/release/bundle/{msi,nsis}/`。仓库里有两个自检脚本，出问题先跑它们：

```bash
node tools/check-artifact-paths.mjs   # 产物路径只有一处真源 + 与 build.rs 对照
node tools/check_assets_agree.mjs     # workflow 与产物表对得上吗
```

完整流程（CI 出包、打包契约、换 ffmpeg / onnxruntime 版本、出问题了查哪里）见
[`如何编译打包.md`](如何编译打包.md)；给接手代码的人看的东西（架构、路径模型、踩过的坑）见
[`AGENTS.md`](AGENTS.md)。

## 许可

**GPL-3.0**（GNU 通用公共许可证第 3 版）—— 全文见 [`LICENSE`](LICENSE)。

可以自由使用、修改、再分发；**再分发（含修改版）时必须同样以 GPL-3.0 开放源码**。

随包分发的外部工具与库（FFmpeg / LibreSVIP / yt-dlp / JIZURA 与字体 / webwallgl 等）
**各自独立授权**、不属于本许可证覆盖范围，逐项说明见
[`docs/THIRD-PARTY-NOTICES.md`](docs/THIRD-PARTY-NOTICES.md)。

## 致谢

感谢所有赞助者与测试者 —— 完整的感谢名单在程序里的「设置 → 关于」。

这个项目由 DeepSeek、Claude 等智能体辅助开发。
