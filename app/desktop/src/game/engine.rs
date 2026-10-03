//! GAME 推理引擎：把音频跑成音符。
//!
//! 这里是**纯 Rust 实现** —— 不启 Python 子进程、不开本地端口（对比音轨分离
//! 那套 `svsep` 的形态：那边是 Python HTTP 服务 + 固定端口转发，这边是进程内
//! 直接调 ONNX Runtime）。
//!
//! 需要两样东西：
//!
//! * **ONNX Runtime 动态库** —— 直接复用音轨分离运行时里那份
//!   `runtime/Lib/site-packages/onnxruntime/capi/onnxruntime.dll`，不额外下载。
//! * **三个模型图** —— `encoder.onnx` / `segmenter.onnx` / `estimator.onnx`，
//!   权重是 CC BY-NC-SA 4.0（非商业），所以**不进仓库、不随包分发**，
//!   由用户在界面上点一下从官方 release 下载。
//!
//! 数值正确性由 `gameport` 那份验证工程逐位保证（对 golden 向量 21 项全等，
//! 对真实人声 41 个音符逐音符相等）；本文件是从那份移植过来的生产形态。

use std::path::{Path, PathBuf};

use ndarray::{Array1, Array2, ArrayD, IxDyn};
use ort::session::{Session, SessionInputs};
use ort::value::Tensor as OrtTensor;
use ort::value::Value as OrtValue;

use super::algo;
use super::midi::Note;

/// 一个音符。`pitch` 是**半音值**（A4 = 69），写 MIDI 时才取整 —— 保留小数
/// 是为了界面能显示「偏高/偏低多少音分」。
#[derive(Debug, Clone, Copy)]
pub struct NoteOut {
    pub onset: f64,
    pub offset: f64,
    pub pitch: f32,
}

/// 转录参数。默认值就是上游 CLI 的默认值（`infer.py extract`）。
#[derive(Debug, Clone)]
pub struct Options {
    /// D3PM 采样步数。上游默认 8 —— **这是质量与时间的主要旋钮**：
    /// CPU 上全程耗时几乎正比于它。
    pub steps: usize,
    /// 界面语言 id（`config.json` 的 `languages` 映射，0 = 通用）。
    pub language: i64,
    /// 每个 ONNX 会话的 intra-op 线程数。见 `ort_session` 的注释。
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { steps: algo::D3PM_STEPS, language: 4, threads: 4 }
    }
}

/// 进度回调：`(阶段说明, 已完成的占比 0..1)`。
pub type Progress<'a> = dyn Fn(&str, f64) + Send + Sync + 'a;

pub struct Report {
    pub notes: Vec<NoteOut>,
    /// 每个切片的 `(起始秒, 采样点数)`
    pub slices: Vec<(f64, usize)>,
    pub n_samples: usize,
    /// 各阶段耗时（秒），界面拿它估剩余时间
    pub encoder_seconds: f64,
    pub segmenter_seconds: f64,
    pub estimator_seconds: f64,
    /// D3PM 每一步的边界数，出问题时是唯一的现场记录
    pub per_step: Vec<usize>,
}

/// 建一个 ONNX 会话。
///
/// **必须显式指定 CPU provider**：随包的 `onnxruntime.dll` 是 CUDA 构建，
/// 而它所在目录里同时躺着 `onnxruntime_providers_cuda.dll`。不指定的话 ORT
/// 会去加载 CUDA，在没有 NVIDIA 卡的机器上失败或静默降级 —— 两种结果都很难查。
///
/// 线程数也是显式设的：实测（8 核开发机、T=300 的单步 segmenter）
/// 1 线程 2.66 s、4 线程 1.04 s、**8 线程 3.30 s** —— 图本身偏窄，
/// ORT 自己按核数选的那个默认值反而最慢。
fn ort_session(path: &Path, name: &str, threads: usize) -> Result<Session, String> {
    let mut builder = ort::session::Session::builder().map_err(|e| format!("{name}: {e}"))?;
    builder = builder
        .with_execution_providers([ort::ep::CPU::default().build()])
        .map_err(|e| format!("{name}: CPU provider: {e}"))?;
    builder = builder
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
        .map_err(|e| format!("{name}: 优化级别: {e}"))?;
    builder = builder
        .with_intra_threads(threads.max(1))
        .map_err(|e| format!("{name}: 线程数: {e}"))?;
    builder
        .commit_from_file(path)
        .map_err(|e| format!("{name}: 载入 {} 失败：{e}", path.display()))
}

/// 加载 ONNX Runtime 动态库。只需成功一次，重复调用无害。
pub fn load_runtime(dll: &Path) -> Result<(), String> {
    if !dll.is_file() {
        return Err(format!("找不到 ONNX Runtime：{}", dll.display()));
    }
    ort::init_from(dll)
        .map_err(|e| format!("载入 ONNX Runtime 失败（{}）：{e}", dll.display()))?
        .commit();
    Ok(())
}

/// 跑完整条流水线。
///
/// `waveform` 必须是 **44.1 kHz 单声道** —— 重采样交给 ffmpeg，不在这里做，
/// 和音轨分离那边的做法一致（`audio::run_ffmpeg`）。
pub fn transcribe(
    models_dir: &Path,
    dll: &Path,
    waveform: &[f32],
    opts: &Options,
    progress: &Progress<'_>,
) -> Result<Report, String> {
    load_runtime(dll)?;

    progress("正在载入模型…", 0.0);
    let mut enc = ort_session(&models_dir.join("encoder.onnx"), "encoder", opts.threads)?;
    let mut seg = ort_session(&models_dir.join("segmenter.onnx"), "segmenter", opts.threads)?;
    let mut est = ort_session(&models_dir.join("estimator.onnx"), "estimator", opts.threads)?;

    // 官方导出的图里随机数节点是图内自带的（`RandomUniformLike`），但验证用的
    // 注入版把它改成了名为 `rnd` 的图输入。两种都要能跑：有 `rnd` 就自己抽随机
    // 数、先把边界噪声化再喂；没有就让图自己去噪。
    let seg_takes_rnd = seg.inputs().iter().any(|i| i.name() == "rnd");

    let steps = opts.steps.max(1);
    let ts = algo::d3pm_ts(algo::D3PM_T0, steps);
    let radius = (algo::SEG_RADIUS_SEC / algo::TIMESTEP).round() as i64;

    let slices = algo::slice_waveform(waveform, algo::SAMPLE_RATE);
    let mut report = Report {
        notes: Vec::new(),
        slices: slices.iter().map(|(o, w)| (*o, w.len())).collect(),
        n_samples: waveform.len(),
        encoder_seconds: 0.0,
        segmenter_seconds: 0.0,
        estimator_seconds: 0.0,
        per_step: Vec::new(),
    };
    if slices.is_empty() {
        // 全静音：没有切片，也就没有音符。不是错误。
        progress("完成（没有检测到人声）", 1.0);
        return Ok(report);
    }

    // 每个切片在整体进度里的权重：切片越长占得越多（时间基本正比于长度）。
    let total_samples: usize = slices.iter().map(|(_, w)| w.len()).sum();
    let mut done_samples = 0usize;
    let mut rng_state = 0x2026_1003_2026_1003u64;

    for (si, (offset, chunk)) in slices.iter().enumerate() {
        let base = done_samples as f64 / total_samples as f64;
        let share = chunk.len() as f64 / total_samples as f64;
        let tag = if slices.len() > 1 {
            format!("第 {}/{} 段", si + 1, slices.len())
        } else {
            "正在扒谱".to_string()
        };

        let duration = chunk.len() as f32 / algo::SAMPLE_RATE as f32;
        let wf: Array2<f32> =
            Array2::from_shape_vec((1, chunk.len()), chunk.clone()).map_err(|e| e.to_string())?;
        let dur: Array1<f32> = Array1::from_vec(vec![duration]);

        progress(&format!("{tag}：提取特征…"), base);
        let t0 = std::time::Instant::now();
        let enc_out = enc
            .run(ort::inputs![
                OrtTensor::from_array(wf).map_err(|e| e.to_string())?,
                OrtTensor::from_array(dur).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("encoder 推理失败：{e}"))?;
        report.encoder_seconds += t0.elapsed().as_secs_f64();

        let x_seg = enc_out["x_seg"]
            .try_extract_array::<f32>()
            .map_err(|e| format!("x_seg: {e}"))?
            .to_owned();
        let x_est = enc_out["x_est"]
            .try_extract_array::<f32>()
            .map_err(|e| format!("x_est: {e}"))?
            .to_owned();
        let mask_t: Vec<bool> = enc_out["maskT"]
            .try_extract_array::<bool>()
            .map_err(|e| format!("maskT: {e}"))?
            .iter()
            .copied()
            .collect();
        let t_len = mask_t.len();

        // ── D3PM 采样环（宿主侧，见 `algo` 顶部的表）────────────────────
        let known = vec![false; t_len];
        let mut prev = known.clone();
        let t_seg = std::time::Instant::now();
        for (step, tv) in ts.iter().enumerate() {
            let p = algo::d3pm_time_schedule(*tv);
            let rnd: Vec<f32> = (0..t_len)
                .map(|_| algo::next_uniform(&mut rng_state))
                .collect();
            // 没有 `rnd` 输入时不能自己先去噪，否则等于噪化两遍。
            let prev_after = if seg_takes_rnd {
                algo::remove_mutable_boundaries(&prev, &known, p, &rnd)
            } else {
                prev.clone()
            };

            let prev_arr: Array2<bool> =
                Array2::from_shape_vec((1, t_len), prev_after).map_err(|e| e.to_string())?;
            let known_arr: Array2<bool> =
                Array2::from_shape_vec((1, t_len), known.clone()).map_err(|e| e.to_string())?;
            let mask_arr: Array2<bool> =
                Array2::from_shape_vec((1, t_len), mask_t.clone()).map_err(|e| e.to_string())?;
            let lang: Array1<i64> = Array1::from_vec(vec![opts.language]);
            let t_scalar: ArrayD<f32> = ArrayD::from_elem(IxDyn(&[]), *tv);
            let thr: ArrayD<f32> = ArrayD::from_elem(IxDyn(&[]), algo::SEG_THRESHOLD);
            let rad: ArrayD<i64> = ArrayD::from_elem(IxDyn(&[]), radius);

            // 输入要按需增删，所以用 `SessionInputs::from(Vec<(Cow<str>, Value)>)`
            // 而不是 `ort::inputs![]` —— 后者拼不出可选输入。
            let mut ins: Vec<(std::borrow::Cow<'_, str>, OrtValue)> = vec![
                (
                    "x_seg".into(),
                    OrtTensor::from_array(x_seg.clone())
                        .map_err(|e| e.to_string())?
                        .into(),
                ),
                (
                    "language".into(),
                    OrtTensor::from_array(lang).map_err(|e| e.to_string())?.into(),
                ),
                (
                    "known_boundaries".into(),
                    OrtTensor::from_array(known_arr)
                        .map_err(|e| e.to_string())?
                        .into(),
                ),
                (
                    "prev_boundaries".into(),
                    OrtTensor::from_array(prev_arr)
                        .map_err(|e| e.to_string())?
                        .into(),
                ),
                (
                    "t".into(),
                    OrtTensor::from_array(t_scalar)
                        .map_err(|e| e.to_string())?
                        .into(),
                ),
                (
                    "maskT".into(),
                    OrtTensor::from_array(mask_arr)
                        .map_err(|e| e.to_string())?
                        .into(),
                ),
                (
                    "threshold".into(),
                    OrtTensor::from_array(thr).map_err(|e| e.to_string())?.into(),
                ),
                (
                    "radius".into(),
                    OrtTensor::from_array(rad).map_err(|e| e.to_string())?.into(),
                ),
            ];
            if seg_takes_rnd {
                let rnd_arr: Array2<f32> =
                    Array2::from_shape_vec((1, t_len), rnd).map_err(|e| e.to_string())?;
                ins.push((
                    "rnd".into(),
                    OrtTensor::from_array(rnd_arr)
                        .map_err(|e| e.to_string())?
                        .into(),
                ));
            }

            let out = seg
                .run(SessionInputs::from(ins))
                .map_err(|e| format!("segmenter 推理失败（第 {} 步）：{e}", step + 1))?;
            prev = out["boundaries"]
                .try_extract_array::<bool>()
                .map_err(|e| format!("boundaries: {e}"))?
                .iter()
                .copied()
                .collect();
            let n = prev.iter().filter(|b| **b).count();
            report.per_step.push(n);
            progress(
                &format!("{tag}：{}", seg_status(step + 1, steps, n)),
                base + share * (step + 1) as f64 / (steps + 1) as f64,
            );
        }
        report.segmenter_seconds += t_seg.elapsed().as_secs_f64();

        // ── 区间/时值 ────────────────────────────────────────────────
        let regions = algo::boundaries_to_regions(&prev, Some(&mask_t));
        let max_n = regions.iter().copied().max().unwrap_or(0) as usize;
        if max_n == 0 {
            done_samples += chunk.len();
            continue;
        }
        let n_mask: Vec<bool> = vec![true; max_n];
        let durations = algo::regions_to_durations(&regions, max_n);

        let mask_arr: Array2<bool> =
            Array2::from_shape_vec((1, t_len), mask_t.clone()).map_err(|e| e.to_string())?;
        let n_mask_arr: Array2<bool> = Array2::from_shape_vec((1, max_n), n_mask)
            .map_err(|e| e.to_string())?;
        let bd_arr: Array2<bool> =
            Array2::from_shape_vec((1, t_len), prev.clone()).map_err(|e| e.to_string())?;
        let thr: ArrayD<f32> = ArrayD::from_elem(IxDyn(&[]), algo::EST_THRESHOLD);

        progress(&format!("{tag}：判断音高…"), base + share * steps as f64 / (steps + 1) as f64);
        let t_est = std::time::Instant::now();
        let out = est
            .run(ort::inputs![
                OrtTensor::from_array(x_est.clone()).map_err(|e| e.to_string())?,
                OrtTensor::from_array(bd_arr).map_err(|e| e.to_string())?,
                OrtTensor::from_array(mask_arr).map_err(|e| e.to_string())?,
                OrtTensor::from_array(n_mask_arr).map_err(|e| e.to_string())?,
                OrtTensor::from_array(thr).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("estimator 推理失败：{e}"))?;
        report.estimator_seconds += t_est.elapsed().as_secs_f64();

        let presence: Vec<bool> = out["presence"]
            .try_extract_array::<bool>()
            .map_err(|e| format!("presence: {e}"))?
            .iter()
            .copied()
            .collect();
        let scores: Vec<f32> = out["scores"]
            .try_extract_array::<f32>()
            .map_err(|e| format!("scores: {e}"))?
            .iter()
            .copied()
            .collect();

        // 音符抽取，照 `inference/callbacks.py:47-88`：时值累加成起止时间，
        // 跳过没有 presence 的、以及零长度（去重叠后会出现）的。
        let mut cursor = *offset;
        for i in 0..max_n {
            let d = durations[i] as f64 * algo::TIMESTEP as f64;
            let onset = cursor;
            let end = onset + d;
            cursor = end;
            if !presence.get(i).copied().unwrap_or(false) || end - onset <= 0.0 {
                continue;
            }
            report.notes.push(NoteOut {
                onset,
                offset: end,
                pitch: scores.get(i).copied().unwrap_or(0.0),
            });
        }
        done_samples += chunk.len();
    }

    report.notes.sort_by(|a, b| {
        a.onset
            .partial_cmp(&b.onset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    progress("完成", 1.0);
    Ok(report)
}

fn seg_status(step: usize, steps: usize, n: usize) -> String {
    format!("去噪 {step}/{steps}（候选边界 {n} 个）")
}

/// 把音符写成单轨 MIDI 字节。
///
/// 上游的写出回调会先做**单音化去重叠**（按 `(onset, offset, pitch)` 排序后把
/// 每个起音顶到上一个的收尾），所以这里也照做 —— 保持和官方 CLI 输出一致。
pub fn notes_to_midi(notes: &[NoteOut]) -> Vec<u8> {
    let notes: Vec<Note> = notes
        .iter()
        .map(|n| Note { onset: n.onset, offset: n.offset, pitch: n.pitch })
        .collect();
    super::midi::write_midi(&super::midi::deoverlap_mono(notes))
}

/// 把音符转成 `(onset, offset, pitch, midi)` 四列，界面表格和 CSV 都用它。
pub fn note_rows(notes: &[NoteOut]) -> Vec<(f64, f64, f32, i32)> {
    notes
        .iter()
        .map(|n| (n.onset, n.offset, n.pitch, n.pitch.round() as i32))
        .collect()
}

/// 模型目录里三个图是否齐全。
pub const MODEL_FILES: [&str; 3] = ["encoder.onnx", "segmenter.onnx", "estimator.onnx"];

/// 返回缺失的模型文件名。
pub fn missing_models(dir: &Path) -> Vec<&'static str> {
    MODEL_FILES
        .iter()
        .copied()
        .filter(|f| !dir.join(f).is_file())
        .collect()
}

/// 模型目录的候选位置，按优先级排列。
///
/// 和音轨分离同一套两层模型：可写的 `writable/game/models`（下载物落这儿），
/// 以及随包只读的 `<root>/app/data/game/models`。
/// ⚠️ **绿色版这两层是同一个绝对路径**（`writable == <root>/app/data`），
/// 所以「下载物」与「随包物」在那台机器上分不开 —— 判层只能比绝对路径，
/// 判完还得看那一层**齐不齐**（见 `midi_transcribe::status` 的 `origin`）。
pub fn model_dirs(root: &Path, writable: &Path) -> Vec<PathBuf> {
    vec![
        writable.join("game").join("models"),
        root.join("app").join("data").join("game").join("models"),
    ]
}

/// 找到第一个可用的模型目录；一个都没有就返回第一个候选（供下载用）。
///
/// ⚠️ 判「可用」用 `missing_models(d).is_empty()`，**不是「目录存在」** ——
/// 空目录必须当成没有。`writable/game/models` 下过一半只解出两个图、
/// 或者用户点了「删掉下好的依赖」把它清空之后，**随包那一层明明齐全**，
/// 都不该因为这个空目录排在候选表第一位就被选中（那会显示成「缺 3 个」、
/// 冒出下载按钮，把本来能用的那份模型盖掉）。
/// ⛔ 别把这个 `is_empty()` 检查改成 `d.is_dir()`。
pub fn resolve_model_dir(root: &Path, writable: &Path) -> PathBuf {
    let dirs = model_dirs(root, writable);
    for d in &dirs {
        if missing_models(d).is_empty() {
            return d.clone();
        }
    }
    dirs.into_iter().next().unwrap_or_else(|| writable.join("game").join("models"))
}
