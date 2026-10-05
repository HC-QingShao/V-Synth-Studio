# V-Synth-Studio —— **一个专为P主制作的工作站。**

> ## ⚠️ 完全测试版（Beta）
>
> **全部功能都还在测试与调整中**：可能有 bug、接口可能改动、极端情况下可能丢数据。
> **用之前请先备份工程文件**，别把它当成唯一的工作副本。
> 遇到问题欢迎到 Q 群（设置 → 关于）或 [Issues](https://github.com/QingMu39-Gao/V-Synth-Studio/issues) 反馈。

- **工程转换**：40 种工程格式互转。内置 LibreSVIP CLI。
- **视频解析**：B 站原生解析 + yt-dlp 兜底。可选下封面 / 弹幕 / 字幕；多线程分块下载；试听先缓存到本机再播（同一支看第二次瞬时）。
- **音轨分离**：拆人声 / 伴奏 / 鼓 / 贝斯 / 钢琴 / 其它。在线 MVSEP 或内嵌引擎。
- **人声转 MIDI**：干声扒谱，导出 `.mid`。内置 ONNX 推理，进程内、无子进程。
- **音频工具**：格式转换、变调变速、裁剪、响度归一化、波形编辑。内置 ffmpeg。
- **网易云专栏**：搜歌、取词、导 LRC/SRT、下封面、下歌曲。网易云官方接口，可选登录（手机号验证码或 Cookie）。
- **文字 PV**：歌词做成动态歌词视频 / PNG 序列。内置 JIZURA。
- **资源库**：4 组 27 条：工程分享、免费音源、编辑器官网、UTAU 系开源。只收录链接，不转载文件。

---

## 安装

到 **[Releases](https://github.com/QingMu39-Gao/V-Synth-Studio/releases/latest)** 下载最新的
`V-Synth-Studio_x.y.z_x64_zh-CN.msi`，双击安装。装完直接能用，不联网也行。

> 安装包没有代码签名，第一次运行会看到一次 SmartScreen 提示：
> 点「更多信息」→「仍要运行」即可。

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

---

## 许可

**GPL-3.0**（GNU 通用公共许可证第 3 版）—— 全文见 [`LICENSE`](LICENSE)。

可以自由使用、修改、再分发；**再分发（含修改版）时必须同样以 GPL-3.0 开放源码**。

随包分发的外部工具（FFmpeg / LibreSVIP / yt-dlp / JIZURA 与字体）**各自独立授权**、
不属于本许可证覆盖范围，逐项说明见 [`docs/THIRD-PARTY-NOTICES.md`](docs/THIRD-PARTY-NOTICES.md)。

---

## 致谢

感谢所有赞助者与测试者 —— 完整的感谢名单在程序里的「设置 → 关于」。

这个项目由 DeepSeek、Claude 等智能体辅助开发。