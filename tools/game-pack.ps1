# 把「人声转 MIDI」的 GAME 模型打成一个给用户下载的 zip
#
#   源  app\data\game\models\{encoder,segmenter,estimator}.onnx + config.json
#   产物  <程序根目录>\资料归档\GAME-1.0.3-large-onnx.zip   （实测 364,093,888 B ≈ 347 MiB）
#
# ── 为什么要自己打一份 ──────────────────────────────────────────────
# 上游 openvpi/GAME 的 release 里**已经有**同名同内容的包
# （v1.0.3 的 `GAME-1.0.3-large-onnx.zip`，361,619,205 B；它就是我们这个包的来源）。
# 自己再打一份的唯一理由是**托管**：GitHub 的 release 资产在境内经常连不上
# （实测直连 github.com 通、api.github.com 通，但 release 资产的 302 跳到
#  objects.githubusercontent.com 之后 TLS 握手就断），而音轨分离的运行时与
# 模型早就放在 123 云盘 CDN 上了。这个脚本产出**结构完全一样**的包，
# 换到 CDN 就能用，用户那边一行代码都不用改。
#
# ── zip 布局不是自由的 ──────────────────────────────────────────────
# `midi_transcribe.rs::download_models()` 解包时剥掉 `GAME-1.0.3-large-onnx/`
# 这一层，解到 <可写目录>\game\models\。**故意沿用上游那个目录名**，
# 这样「官方包」与「我们的包」是同一个布局 —— 哪天下游要换回官方源、
# 或者用户自己拿官方 zip 来用，都不用改代码。
# 打包时打成平铺（把 4 个文件直接放根）看起来更「干净」，但那样
# strip 就命不中，文件会被解到 game\models\GAME-1.0.3-large-onnx\ 下面，
# 而 `engine::missing_models` 找的是 `game\models\encoder.onnx` ——
# **不会报任何错**，只会在用户点「开始扒谱」时说「模型还没装全」。
#
# ⚠️ 只打白名单里的 4 个文件。`app\data\game\models\` 是开发机上正在用的目录，
#    里面可能有临时备份（`*.bak`）或调试模型（`segmenter_det.onnx`），
#    整目录打包会把它们一起发出去 —— 那是几百 MB 的白流量。
#
# ⚠️ segmenter 要用**官方那一版**（带 `RandomUniformLike` 节点，真随机），
#    不是逐位验证时用的注入版（`export\onnx_inj\`，把随机数改成图输入）。
#    两个都跑得通、结果都对，但注入版是固定种子 —— 每次结果一模一样。
#    `engine.rs` 会自己探测图里有没有 `rnd` 输入，两种都能吃，所以拿错
#    不会报错，只会静默地换掉随机性。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File tools\game-pack.ps1
#   powershell -ExecutionPolicy Bypass -File tools\game-pack.ps1 -Src 'H:\别处的models'
#   powershell -ExecutionPolicy Bypass -File tools\game-pack.ps1 -Force
#
# ⚠️ 这个脚本**只打包**。上传到 123 云盘是手工活，传完把直链填进
#    `app\desktop\src\midi_transcribe.rs` 的 `MODEL_URL`（并同步
#    `MODEL_ZIP_BYTES` / `MODEL_BYTES` 两个实测常量）。

param(
    [string]$Src,                        # 模型目录；默认 <root>\app\data\game\models
    [string]$Out,                        # 输出目录；默认 <root>\资料归档
    [switch]$Force,                      # 覆盖已存在的 zip
    [ValidateSet('Fastest', 'Optimal', 'NoCompression')]
    [string]$Level = 'Optimal'
)

$ErrorActionPreference = 'Stop'
# ⚠️ 两个都要加载：ZipFile / ZipFileExtensions 在 .FileSystem 里，而
#    ZipArchiveMode 在 System.IO.Compression 里（svsep-pack.ps1 里踩过这条）。
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Split-Path -Parent $here
if (-not (Test-Path (Join-Path $root 'app\desktop'))) {
    throw "看着不像程序根目录：$root（下面应该有 app\desktop\）"
}

if (-not $Src) { $Src = Join-Path $root 'app\data\game\models' }
if (-not $Out) { $Out = Join-Path $root '资料归档' }
$Src = [IO.Path]::GetFullPath($Src)
$Out = [IO.Path]::GetFullPath($Out)

# zip 里的顶层目录名 —— 与上游 release 一字不差，别改（理由见文件头）
$Top = 'GAME-1.0.3-large-onnx'
$Name = "$Top.zip"

# 白名单 + 每个文件的实测大小。大小对上才算「是官方导出的那一版」：
# 注入版 segmenter 比官方小 37 字节，光看名字看不出来。
$Files = @(
    @{ Name = 'encoder.onnx';   Bytes = 81312536  }
    @{ Name = 'segmenter.onnx'; Bytes = 160373028 }
    @{ Name = 'estimator.onnx'; Bytes = 152478761 }
    @{ Name = 'config.json';    Bytes = 198       }
)
# 解包后总共多大（=`midi_transcribe.rs::MODEL_BYTES` 那个常量，
# 那边现在是 393,794,532 —— 改这里的白名单就要同步改那边）
$ExtractBytes = 394164523     # 81,312,536 + 160,373,028 + 152,478,761 + 198
                              # ⚠️ PS 5.1 不认 `394_164_523` 这种下划线分隔（会当成命令名）

Write-Host "源  ：$Src"
Write-Host "产物：$(Join-Path $Out $Name)"
Write-Host ""

if (-not (Test-Path $Src)) { throw "源目录不存在：$Src" }

# ── 校验（宁可在这里红，也别把一个结构不对的包传上 CDN）───────────────
$bad = 0
foreach ($f in $Files) {
    $p = Join-Path $Src $f.Name
    if (-not (Test-Path $p)) {
        Write-Host ("  [缺] {0}" -f $f.Name)
        $bad++
        continue
    }
    $len = (Get-Item $p).Length
    if ($len -ne $f.Bytes) {
        Write-Host ("  [大小不对] {0}：{1:N0} B，期望 {2:N0} B" -f $f.Name, $len, $f.Bytes)
        $bad++
    } else {
        Write-Host ("  [ok] {0,-16} {1,12:N0} B" -f $f.Name, $len)
    }
}
if ($bad -gt 0) {
    Write-Host ""
    throw "有 $bad 个文件不对（见上）。注入版 segmenter 会小 37 字节，别拿它当真品。"
}

# 源目录里的**其他**文件会被忽略 —— 明确说出来，免得以为漏打了
$extra = @(Get-ChildItem $Src -File | Where-Object { $_.Name -notin ($Files | ForEach-Object { $_.Name }) })
if ($extra.Count -gt 0) {
    Write-Host ""
    Write-Host ("  （忽略源目录里另外 {0} 个文件：{1}）" -f $extra.Count, (($extra | ForEach-Object { $_.Name }) -join '、'))
}

New-Item -ItemType Directory -Path $Out -Force | Out-Null
$to = Join-Path $Out $Name
if ((Test-Path $to) -and -not $Force) {
    Write-Host ""
    Write-Host ("[跳过] 已存在 {0}（{1:N1} MB）—— 要重打加 -Force" -f $Name, ((Get-Item $to).Length / 1MB))
    exit 0
}
if (Test-Path $to) { Remove-Item $to -Force }

# ── 打包 ────────────────────────────────────────────────────────────
# 不用 Compress-Archive（它先在内存里建完整清单，而且控不了条目名）；
# 也不用 CreateFromDirectory（一个包只能有一个根目录，而我们**要**显式
# 指定条目名带 $Top 前缀）。ZipFile.Open + CreateEntryFromFile 是流式的，
# 条目名自己给 —— 这样就不必先在磁盘上搭一个中转目录。
Write-Host ""
Write-Host ("打包中（压缩档 {0}）…" -f $Level)
$sw = [Diagnostics.Stopwatch]::StartNew()
$zip = [IO.Compression.ZipFile]::Open($to, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($f in $Files) {
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $zip, (Join-Path $Src $f.Name), "$Top/$($f.Name)",
            [IO.Compression.CompressionLevel]::$Level)
    }
} finally { $zip.Dispose() }
$sw.Stop()

$size = (Get-Item $to).Length
# ⚠️ 不能写 `Measure-Object -Property Bytes`：`$Files` 是**哈希表数组**，
#    `Measure-Object` 不认识哈希表的键（实测报 "The property Bytes cannot be
#    found"）。要么像这样把值挑出来，要么用 `ForEach-Object` 求和。
$srcBytes = ($Files | ForEach-Object { $_.Bytes } | Measure-Object -Sum).Sum
$hash = (Get-FileHash $to -Algorithm SHA256).Hash

Write-Host ""
Write-Host ("[完成] {0}" -f $Name)
Write-Host ("       {0:N1} MB（源 {1:N1} MB，压缩率 {2:N0}%）  耗时 {3:N0}s" -f `
    ($size / 1MB), ($srcBytes / 1MB), (100 * $size / $srcBytes), $sw.Elapsed.TotalSeconds)
Write-Host ("       SHA-256 {0}" -f $hash)
Write-Host ""
Write-Host "下一步（这个脚本不做）：把 $Name 传到 123 云盘、拿直链，然后改"
Write-Host "  app\desktop\src\midi_transcribe.rs 的 MODEL_URL，并把"
Write-Host "  MODEL_ZIP_BYTES 改成 $size"
Write-Host "  MODEL_BYTES     改成 $ExtractBytes"
Write-Host "（两个常量都只是显示与校验用的实测值，改了更准，不改也不会错。）"
