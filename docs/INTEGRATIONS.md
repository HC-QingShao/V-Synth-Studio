# 嵌外部 AI 项目：两套完整经验（音轨分离 · 人声转 MIDI）

**这一份从 `AGENTS.md` 第五节里拆出来的。** 那边太长，破了工作区指令的 64 KB 预算
（超了就会被截断，后面的节会看不见）—— 所以把这两节放到这里，`AGENTS.md` 只留索引。

**什么时候读**：要往工作站里再塞一个外部 AI 项目（模型 / 推理引擎 / 别人的服务）时。
两节是姊妹篇，正好是**两种做法**：

- **五之四 = 包一个别人的 Python 服务**（音轨分离：子进程 + 端口 + 健康检查 + 收尸）。
- **五之五 = 把算法重写进 Rust**（人声转 MIDI：进程内、零子进程、零端口）。

先读 `AGENTS.md` 的「零、硬规矩」，再读这里。

---
## 五之四、把一个外部 AI 软件嵌进来（音轨分离的完整经验）

**这一节是给「要往工作站里再塞一个外部 AI 项目」的人写的** —— 比如自动扒谱。
音轨分离（`svsep.rs` + `server/svsep.rs` + `tools/svsep-pack.ps1` + 分离页）
踩过的坑与设计取舍全在这儿，照着走能省几天。**先读「零、硬规矩」。**

### 动手之前（半天，不许跳过）

1. **先把上游跑通再动 Rust。** 在 `%TEMP%` 里手动把包解开、手动起进程、手动发一次
   请求，直到它真的返回一轨音频。**看不懂它的入参出参就不要开始写包装。**
2. **问一句「能不能直接用」。** 用户明说过：优先直接调用现成的，别自己重写
   （分离用的是现成的离线引擎 UVR / RoFormer，不是自己实现的分离算法）。
3. **列「进程 + 端口 + 路径 + 环境变量」四张清单。** 它要几个进程、监听哪个端口、
   从哪读模型、靠哪些环境变量找东西 —— 这四样决定了后面全部设计。
4. **先用界面画清「要什么」再动手。** 每加一个外部引擎，界面都要多出三样：
   装没装（自检）、怎么装（带进度的下载）、怎么卸（一键删除）。少一样都要返工。

### 磁盘布局：两层，别混

| 放哪 | 内容 | 谁写 |
|---|---|---|
| `<root>/app/data/svsep/`（**也可能不可写**） | `runtime/`（python.exe 与 site-packages）、`backend/*.py`、`bin/ffmpeg.exe` | **随包**，只读 |
| 可写目录（绿色版 = `app/data/`，安装版 = `%APPDATA%`） | `models/`、运行期 `data/ logs/ outputs/ uploads/`、下载中的 `.part` | **按需下载**与运行产生 |

依据是 `app/desktop/src/svsep.rs` 的 `Bundle`（`models` / `runtime` 两种）。
⚠️ **子进程的「运行目录」（cwd）必须是那个 svsep 根**：`backend/config.py:12` 是
`BASE_DIR = Path(__file__).resolve().parent.parent`，一切相对路径都挂在它身上。
⚠️ 可写目录不是 `.exe` 所在目录 —— 见第二节「路径模型」。

### 改上游要小到极致

| 需求 | 做法 | 为什么不用另一条路 |
|---|---|---|
| 让引擎认磁盘 | **加一个环境变量覆盖**（`VSS_SVSEP_*`），别改它的硬编码常量 | 软链接 / junction 要提权，且目标不存在时建不出来。一个变量就解决 |
| 出参改形状 | **别改后端**。工作站只做多部分表单转发，不拆包也不重打 | 前端已按上游的字段名拼好；自己拆开再拼要处理文件名转义与大文件缓冲 |
| 加认证 | **不加**。只监听 127.0.0.1 + 固定端口 | 上游自己的 `port_bind.py` 够用；加 auth 既改上游又改前端 |

### 打包：包里的东西必须等于「代码真会去找的东西」

- 两个包：**runtime**（python 3.11 + CUDA 版 torch）与 **models**（模型 + 索引）。
  分开发 —— 「只想换模型」和「重装运行时」是两件事，用户网慢时也能分开下。
- ★ **判据不是「包里文件挺多」，是「`runtime/python.exe` 和 `backend/app.py` 在不在」。**
  引擎还要 `bin/ffmpeg.exe`：`backend/config.py:54-66` 的 `_ensure_ffmpeg_on_path()`
  把 `BASE_DIR/bin` 或 `BASE_DIR/runtime` 塞进 PATH。
  第一版用 `CreateFromDirectory` 打**只能有一个顶层目录**，结果只装了 `runtime\` ——
  用户下完 4.5 GB 还是起不来。改成 `ZipFile.Open` + `CreateEntryFromFile` 手工加
  `Dirs = @('runtime','backend','bin')`（`tools/svsep-pack.ps1`）。
- 包打出来后**立刻点名断言**那三个文件，每次改打包脚本都重跑一次解压测试。

### 下载 / 解压：一条管子

一条代码路径（`fetch_bundle`）里同时做完**下载 + 解压 + 进度 + 暂停 + 续传**：

- **顺序是判据**：`fetch_bundle` 在拉数据**之前**先回调一次 `on_progress(already, total)`；
  续传的第一次回调必须是 `(已有字节, 整包)`。没带 Range 时第一次是 `(0, 整包)` ——
  这条断言能一眼看出「到底续上了没」。
- 服务端回 **206 才追加、200 就归零**（有的静态服务器不认 `Range`，回 200 整包，
  那时接着追加就会拼出坏 zip）。用 `tmp-rangesrv.mjs` 那种本地 Range 服务器验。
- **暂停留 `.part`、停止删 `.part`**；zip 要存在 `.part` 而不是直接 `.zip`，
  否则「下到一半点了分离」会被 `models_status()` 当成完整包。
- ★ **续传点记在盘上（`.part` 旁边的 `.part.url`），别只记在内存。** 内存里的记号
  一重启就没了 —— 磁盘上躺着 4 GB 的半个包，界面却说「没下过」，用户一点就从零开始。
  第一版就是这么错的。
- **为什么必须记链接**：换了下载服务器后，拿旧半个包接新链接的 `Range`，会拼出
  「旧包前半段 + 新包后半段」的坏 zip，**要到解压才炸**，看着像解析器坏了。
- 判「能不能续」分三种（`stored_resume`）：**没有 `.part` → 不能**；
  **有 `.part` 没记号 → 能**（记号的用处是「证明这半个包属于哪个链接」，缺失不该让几个 GB 报废）；
  **记号在且写着别的链接 → 不能**。
- **超时要单独给**：大包按小时算，默认 30 秒超时会在几小时处断开、白下。
- 进度回调**限流**（约每 32 MB / 每半秒一次），否则几万次回调能把渲染队列打满。
- 解压要自己写 zip 解析器（只有 `flate2`，**没有 `zip` crate**）。⚠️ Zip64 的坑见
  「五之二」：`csize` 与 **`lho`** 都可能是 `0xFFFFFFFF` 哨兵，且**报错必须带条目名**。
- **删依赖逐文件 `remove_file`，绝不用 `remove_dir_all`**：几万个文件里总有被占用的，
  一个失败就全放弃最糟。删不掉的收进 `locked[]`（最多 8 条）返回，界面告诉用户
  「关掉占用的程序再删」。只删 `models/`、`runtime/`、`bin/` —— **别碰 `backend/` 的 .py**。

### 子进程：它必须跟着主进程一起死

- 正常退出靠 `impl Drop`，**强杀靠不住** —— 任务管理器结束进程时 `Drop` 不跑，
  留下孤儿 `python.exe` 继续监听端口、占几 GB 内存，用户看到「关掉了风扇还转」。
- 兜底是 Windows **作业对象**（`svsep.rs` 的 `job` 模块）：`CreateJobObjectW` +
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject`，
  **句柄故意不关**（存 `OnceLock`）—— 进程一死句柄被内核回收，作业里的子进程一起死。
- 起进程、等它就绪（轮询健康接口）、转发请求、退出收尸 —— 四步都要有超时与明确报错，
  别让界面永远转圈。

### 验证清单

1. `cargo test --bins`（含 `zip64_resolve` 的组合单测、删除依赖的文件数断言）。
2. **真包解压测试**：跑完点名那三个文件，并抽查一个**偏移超 4 GiB** 的条目。
3. **HTTP 层实测**（临时安装 + 测试端口）：暂停 → 看 `.part` 大小 → **重启进程** →
   状态里 `resumable` 仍为 true → 继续下载 → 起始字节正是暂停点。
   （**断线重试**也能零流量验，做法见 `docs/FEATURES.md`「断线自动重试」；⛔ 别拿真链接验，流量按 GB 算钱。）
4. **强杀进程**：任务管理器结束工作站，确认 `python.exe` 也死了。
5. 前端 `next-smoke.mjs` 逐页冒烟 + `tests/contract/verify.mjs` 契约测试。

### 别重走的死路

**软链接省布局**（要提权、目标不存在时建不出来，用环境变量覆盖）；
**多线程分块下载**（实测瓶颈在出口带宽 —— 四条并行连接总量不变，单条只有 0.11~0.18 MB/s）；
**用「文件数」判装没装**（看代码真去找的那几个文件）；
**把 `(os error 112)` 当解析器 bug**（那是磁盘满 —— 见「五之二」磁盘那条）。

---

## 五之五、人声转 MIDI（GAME）—— 一次「不嵌子进程」的集成

2026-10-03 落地。**这一节和「五之四」是姊妹篇**：同样是嵌一个外部 AI 模型，
但做法**反过来** —— 那边是包一个别人的 Python 服务，这边是把算法重写进 Rust。

| | 音轨分离 | 人声转 MIDI |
|---|---|---|
| 上游 | 炽小阳音轨分离站离线版（Python HTTP 服务） | openvpi/GAME（PyTorch 训练 + ONNX 导出） |
| 形态 | **子进程** + 固定端口 + 健康检查 + 收尸 | **进程内**，零子进程、零端口 |
| 大件 | 运行时 4.7 GB + 模型 462 MB | 模型包 364 MB（解开 376 MB；运行时**能借**音轨分离那份 ORT） |
| 权重许可 | 随上游 | **CC BY-NC-SA 4.0（非商业）** —— 必须界面上署名 |
| 代码 | `src/svsep.rs` `src/server/svsep.rs` `src/pages/Svsep.tsx` | `src/game/**` `src/midi_transcribe.rs` `src/server/midi.rs` `src/pages/Midi.tsx` |

### 先记住三条硬结论（省得重走）

1. **⛔ MLX 版在 Windows 上编不出来。** 用户最初给的是
   [`Da1sypetals/game-mlx-rs`](https://github.com/Da1sypetals/game-mlx-rs/)（二次构建的 MLX 版），
   它依赖 `mlx-rs` 的 `features = ["accelerate","metal"]` —— Accelerate 与 Metal 都是
   **macOS 专有框架**。它的 README 里「测试必须 `--test-threads=1`，否则 Metal 驱动层
   竞争导致 SIGSEGV」是第二个证据。**这条路别再看第二眼。**
2. **✅ 官方有现成的 ONNX 权重**：release `v1.0.3` 的 `GAME-1.0.3-large-onnx.zip`
   （361,619,205 B）。⚠️ **注意 `GAME-1.0-large.zip` 里是 PyTorch 的 `model.pt`，不是 ONNX** ——
   两个 release 差一个后缀，认错了会白下 366 MB。
3. **本机没有可用的 GPU 推理后端**（AMD RX 580；`CUDAExecutionProvider` 不可用，
   DirectML 试过 —— `estimator.onnx` 里有一条 Reshape 它处理不了，一推理就抛
   `MLOperatorAuthorImpl.cpp(2597)`）。**纯 CPU 约 10 秒墙钟换 1 秒音频**，
   3 分钟干声 ≈ 半小时。界面必须在按钮**上方**就把这句话说清。

### 导出 ONNX 时的三个坑（`deployment/exporter.py`）

1. **导出总开关是 `deployment/context.py` 的 `export_mode()`**，不是 `eval()`。
   入口必须是 `from deployment.api import deploy_model`（`with torch.no_grad(): with
   export_mode(): Exporter(...).export()`）。不走它就会撞
   `GuardOnDataDependentSymNode: Could not guard on data-dependent expression Eq(u0, 1)`
   （`lib/feature/mel.py:48` 的 `if torch.min(y) < -1.`）——
   **那是设计如此，不是上游 bug**。
   ⛔ 不能 `from inference.api import ...`：`inference/api.py:5` 会拖进 `lightning`。
2. **⛔ 160 个卷积张量的轴顺序在 checkpoint 里是 NHWC 风格**（MLX/TF 遗留），
   PyTorch 要 NCHW。**形状推理分辨不出来**（中间轴都是 1，`permute` 后内存布局相同），
   只能靠数值实验判定：`[工作站临时工程]\export\check_layout.py` 的做法是
   建两版模型、各自与官方 ONNX 比 `max|x_seg diff|`，取偏差小者
   （实测 permute 版相对偏差 1.08e-4，另一版根本装不进模型）。
   `export_onnx.py` 里那个 `--permute-conv` **默认必须开**。
3. 权重加载需要一份键名映射（checkpoint 是裸名、模型侧带 `model.` 前缀）：
   `time_embedding.layers.{0,2}` → `time_embedding.{0,2}`，其余一律加 `model.` 前缀。

### ⛔ `segmenter.onnx` 里的 D3PM 采样是随机的

图里有一个 `RandomUniformLike`（**无 seed**）。同一 session 连跑三次结果都不完全相同。
⇒ **它不能直接当数值 oracle**（分不清「真错」还是「随机不同」）。
验证时必须先把随机数改成图输入：`patch_segmenter_rnd.py` 把
`RandomUniformLike(bitwise_and)` 改写成 `Identity(rnd)` 并新增图输入 `rnd` float `[B,T]`，
由宿主喂抽好的随机数。**发布版保留 `RandomUniformLike`**（真推理要真随机）。
`engine.rs` 用 `let seg_takes_rnd = seg.inputs().iter().any(|i| i.name() == "rnd");`
自动识别两种情况 —— **别在宿主侧噪化两遍**。

### ONNX 的 I/O 契约（写宿主代码时唯一依据）

| 文件 | 输入 | 输出 |
|---|---|---|
| `encoder.onnx` | `waveform` f32 `[B,L]`、`duration` f32 `[B]` | `x_seg`/`x_est` `[B, L/441, 256]`、`maskT` bool |
| `segmenter.onnx` | `x_seg`、`language` int64 `[B]`、`known_boundaries`、`prev_boundaries`、`t`、`maskT`、`threshold`、`radius` | `boundaries` bool |
| `estimator.onnx` | `x_est`、`boundaries`、`maskT`、`maskN`、`threshold` | `presence` bool、`scores` f32（**已经是半音值**） |

- **`t` / `threshold` / `radius` 是 0 维标量 `shape []`，不是 `[B]`。** 喂法必须是
  `np.array(0.5, dtype=np.float32)`；`np.float32(0.5)` 会被 ORT 拒
  （`Unable to handle object of type <class 'numpy.int64'>`），`np.array([0.5])` 会在图内
  报 `onnxruntime::MatMulComputeHelper::Compute MatMul dimension mismatch`。
- **帧数是 `L // 441` 整数除法**，不是教科书 STFT 的 `1 + L/hop`。
  **mel/STFT 在 `encoder.onnx` 图内部完成** —— 宿主不必复刻 librosa 的滤波器组。
- `dur2bd` / `bd2dur` 是纯算子无学习权重（1,967 / 4,612 B），**Rust 手写就行**，别吃 ONNX。
- 半音换算：`midi_num_bins=257`、`midi_min=0`、`midi_max=128` ⇒ **1 个半音 = 2 个 bin**，
  `decode_gaussian_blurred_probs` 返回的**已经是半音值**，不要再乘系数。
- 官方推荐参数：D3PM `t0=0.0` / `nsteps=8`（`ts = [0, .125, …, .875]`）、
  边界阈值 `0.2`、局部极大半径 `0.02 秒`、音符存在阈值 `0.2`。

### 数值验证怎么做（这是这次集成最有价值的部分）

**别靠读代码推演，靠逐位比对。** 做法是三层：

1. PyTorch 侧生成 golden 向量（固定种子 + 合成信号），dump 每一级的中间量；
   再补一份**真实人声**的（`real_port_ref.json` + `real_port_ref.rnd.npy` 记录 8 步的随机数）。
2. Rust 侧对着 golden 跑，逐项打印 `max|diff|` / `rel`。本项目的判据是
   **21 项全 ok 才算过**；任何一项 FAIL 都要查到根因。
3. **合成输入要选已知物理量**：用 220 Hz 正弦，模型解出 **57.01 半音 = MIDI A3** ——
   这一条比逐层读代码快得多地证明了量纲与整条链路都对。

最后的生产形态也是这么验的：用 `#[path]` 把**生产文件本身**（不是副本）挂进一个独立
cargo 工程（`[工作站临时工程]\verify\`），喂真实 10.68 秒干声 + 录好的随机数，
比出土的 41 个音符**音高逐位相同**（`max |Δpitch| = 0.000000000`）。

### 已知的良性偏差

- 音符起止时间比 Python 参考实现大 **2.34e-7 秒**。原因：生产代码走
  `frames as f64 * TIMESTEP as f64`（f32 的 0.01 提升到 f64），
  而参考实现走「时值直接乘」。**这是更准的一边**（帧本来就是精确的 10 ms），不要「修」它。
- 权重许可 **CC BY-NC-SA 4.0**：模型不进仓库、不随包分发，由界面按需从官方 release 下。
  界面上必须写明许可与出处 —— 和资源库的收录原则（第六节）是同一条规矩。

### Node 端的 `ort` 用法（写的时候会撞上的）

- `ort` 目前只有 `2.0.0-rc.13`；用 `features = ["ndarray","load-dynamic"]`，
  `ndarray` **必须与它自带的那份同版本（0.17）**，否则报
  `OwnedTensorArrayData<_> is not satisfied`。
- `load-dynamic` 下用 `ort::init_from(<绝对路径的 dll>)?.commit()`。Windows 走
  `LoadLibraryExW(..., LOAD_WITH_ALTERED_SEARCH_PATH)`，所以**同目录的
  `onnxruntime_providers_shared.dll` 能自动解析** —— 直接指向它就行。
  **本机已经在 `app/data/svsep/runtime/Lib/site-packages/onnxruntime/capi/` 有一份
  ORT 1.23.2（15.55 MB）**，所以这个功能**可以完全不下运行时**。
- 要增删可选输入必须用 `SessionInputs::from(Vec<(Cow<str>, Value)>)`，`ort::inputs![]` 做不到。
- `Session::inputs()` / `Outlet::name()` 是**方法**不是字段。CPU EP 是
  `ort::ep::CPU::default().build()`。
- **线程数钉 4**：实测每步 segmenter 1 线程 2.66 s / **4 线程 1.04 s** / 8 线程 3.30 s
  （图窄，线程多了被超订拖慢）。

### 验证清单（这一次实际跑过的）

1. `cargo test --release` —— 11 个单测全过（D3PM 时间表、`round_half_even`、
   切片器、MIDI 写出、量纲）。
2. **`ort` 全流程对 golden**：21 项全等；真实人声 41 音符音高逐位相同。
3. **真机 HTTP 层**：起测试实例 → `POST /api/midi/transcribe` → 轮询 `/api/jobs/get`
   → 落到 `.mid` / `.csv` / `.json`。实测 10.68 秒干声 **100.9 秒**跑完
   （encoder 5.2 / segmenter 75.9 / estimator 10.1），42 个音符，
   MIDI 里 42 个 `note_on`、`division=480`（= 960 ticks/秒）。
4. 前端 `tsc -b` + `vite build`（**`npm run build` 是 `tsc -b && vite build`，
   类型错会直接挡住构建**），再确认产物 JS 里能找到新页关键字。

### 别重走的死路（这一次新增的）

**MLX**（见上）；**让 ONNX 的 `segmenter` 兼作数值 oracle**（随机）；
**靠形状推理判卷积轴顺序**（`permute` 前后内存布局相同）；
**给 `fetch_to_file` 传手动阶段**（它的回调本来就是三参 `got, total, Stage`，
自己塞 `Stage::Download` 会让解压那一段没有进度）；
**在子进程里靠 `PYTHONPATH` 找模块**（本环境不传播，必须在脚本里 `sys.path.insert`）；
**把 Python 脚本里 `H:\工作站` 显示成 `H:\?????` 当编码 bug**（只是显示，
文件读写正常；但**经 PowerShell here-string 传给 `python -c` 会真乱码**，
必须写成 `.py` 文件）。

### 模型包自己托管（2026-10-03 补）

上游 release 里本来就有同内容的 `GAME-1.0.3-large-onnx.zip`（361,619,205 B），
**重新打一份的唯一理由是托管** —— 这台机器上 GitHub 的 release 资产根本下不动：
`github.com` / `api.github.com` 直连都通（`curl -I` 200），但资产会 302 跳到
`objects.githubusercontent.com`，跟过去 **TLS 握手直接失败**
（`curl: (35) schannel: failed to receive handshake`），带 `-L` 则是
`Failed to connect to github.com port 443: Timed out`（`final_code=000`）。
没有代理（`127.0.0.1:7890` refused）、没有 `*_PROXY`、`hosts` 文件都不存在。
音轨分离的运行时与模型早就在 123 云盘 CDN 上，模型包跟着走同一条路。

| | 值 |
|---|---|
| 打包脚本 | `tools/game-pack.ps1`（白名单 4 个文件 + **逐个校验实测字节数**） |
| 产物 | `资料归档\GAME-1.0.3-large-onnx.zip` = **364,093,888 B**（`Optimal` 档 48 s） |
| 为什么大 2.4 MB | .NET 的 deflate 比上游压得松，正常 |
| 托管 | `midi_transcribe.rs::MODEL_URL`（123 云盘 CDN，**末尾 `#` 不能删**） |
| 常量 | `MODEL_ZIP_BYTES = 364_093_888`（**换包必须同步改**，它就是「下完没有」的判据）、`MODEL_BYTES = 393_794_532` |

**四条要记住的：**

1. **包必须带顶层目录 `GAME-1.0.3-large-onnx/`，不能打成平铺。**
   `download_models` 里 `strip` 是硬编码的 `"GAME-1.0.3-large-onnx/"`，而
   `extract_zip` 剥不中前缀时**不报错、原样保留** ⇒ 文件会落到
   `game\models\GAME-1.0.3-large-onnx\`，`missing_models` 找不到，
   **状态页显示「就绪」、点「开始扒谱」才报「模型没下全」**。
   带上这一层之后自建包与官方包**可以互换**。
2. **判据是内容不是文件名**：`MODEL_FILES` 只看三个 `.onnx` 在不在，
   **注入版与官方版 `segmenter` 只差 37 字节**（160,372,991 / 160,373,028）——
   光看文件名分不出来，所以 `game-pack.ps1` 才要逐个校验字节数。
3. **`RUNTIME_URL` 那条没改托管**（还是 onnxruntime 官方 GitHub release）——
   它只在「没装音轨分离」时才走到，装了的话 `runtime_dll` 直接借运行时里那份。
4. **debug 覆盖口子**：`VSS_MIDI_MODEL_URL` / `VSS_MIDI_RUNTIME_URL`
   （照 `svsep.rs::url_override` 那套，`#[cfg(debug_assertions)]`）。
   发布版必须为「编译进去的常量」——实测 release 二进制里这三个变量名
   **一个都搜不到**，说明确实被 cfg 掉了。

### ⛔ 「删除依赖」必须删 `<可写>/game/models`，而且绿色版判不出「谁放的」

**这一条真被用户报过**：点了「删掉下好的依赖」，模型一个字节没少、状态还是「就绪」，
于是**下载按钮再也不出现**（前端 `Midi.tsx` 只在 `!ready` 时渲染那个按钮）。

根因是**绿色版两层同路径**：`resolve_paths()` 判可写之后 `writable = <root>/app/data`，
而模型的两个候选目录是 `[<可写>/game/models, <root>/app/data/game/models]` ——
**绿色版下它们是同一个绝对路径**，一个目录同时扮演「下载落点」和「随包只读层」。
（安装版才真是两个目录：可写那份在 `%APPDATA%`。）

所以：

- `delete_deps` 现在删**两处** —— `<可写>/midi/`（`.part` / `ort-unpack\` / 下下来的 dll）
  与 **`<可写>/game/models/`**；随包那一层一律不碰。签名是
  `delete_deps(root, writable) -> (u64, u64, String)`，第三项 `note` 直接给界面显示。
- ⛔ **别想区分「这份模型是下下来的还是随手放进去的」—— 物理上分不出来。**
  状态里那个 `models.origin` 报的是「引擎这一刻在用**哪个目录**」：
  `"downloaded"`（= `<可写>/game/models`）/ `"bundled"`（= 随包层，**且它不是下载落点**，
  只可能出现在安装版）/ `"local"`（两层同一路径）。
  前端据此说人话：只有 `bundled` 时才写「这是随包自带的，删除按钮不会动它」，
  **`local` 绝不能说这句**（删了就是真没了，没有第二层可回落）。
- ⛔ **别在前端按 `dir` 的尾巴猜**（`/game[\\/]models$/` 那类）：
  三种情况的路径都以 `\game\models` 结尾，判不出来。我写错过一次，撤掉了。
- 判「模型可用」用 `engine::missing_models(d).is_empty()`（**目录不齐就得跳过**），
  **别改成 `d.is_dir()`** —— 空目录必须当成没有。
- 这个包**不支持续传**，`resumable` 恒 false。`/api/midi/download/pause` 这条路由留着
  只是为了和音轨分离接口形状一致，**真语义是「停下、下次重下」，界面上别加「暂停/继续」**。

**实测（2026-10-03，起 8891 测试实例、全程没碰用户的 17878）：**

| 项 | 结果 |
|---|---|
| 删除依赖 | `files=4`、`bytes=394164523`（三个 onnx + `config.json` 逐项对得上）→ 按钮回来了 |
| **完整下载** | **297 秒下载 + 18 秒解压**，约 1.2 MB/s；压缩包**自动删除**、`.part` 残留 0 |
| 解压产物 | 4 个文件 **SHA-256 与删前逐个一致** |
| `origin` 三分支 | 齐全 → `local`、缺文件 → `downloaded`、删完 → `downloaded` |

打包脚本本身有个 PS 5.1 的坑：**数字分隔符 `1_000_000` 不支持**、
`Measure-Object` 对 hashtable 数组会失败。

---
