# FFmpeg 放置说明

`build.ps1` / `package.ps1` 默认从这里取 FFmpeg（头文件、导入库、`ffmpeg.exe` 与 DLL）：

```
tools/ffmpeg/
├── bin/     ffmpeg.exe / ffprobe.exe / avcodec-62.dll ...
├── include/ libavcodec/ libavformat/ ...
└── lib/     avcodec.lib ... （Rust 的 ffmpeg-next 链接用）
```

该目录**不入库**（约 213 MB 二进制，见 `.gitignore`）。换机器时按下文下载。

## 要求：必须带软件 AV1 编码器

TraceReel 支持 AV1 硬件编码（`av1_qsv` / `av1_nvenc` / `av1_amf`…）。
当显卡不支持 AV1 时，程序会自动**回退到 CPU 软件编码**，这要求 FFmpeg 里有
`libsvtav1`、`libaom-av1` 或 `librav1e` 之一。

> ⚠️ 不要用 avbuild 的 **gpl-lite** 构建：它 configure 时只有
> `--enable-libx264 --enable-libx265 --enable-small`，**没有任何软件 AV1 编码器**。
> 用该构建时 AV1 硬件不可用就只能降级输出 H.264。

## 下载（Windows x64，GPL 构建）

```powershell
# FFmpeg n8.1 GPL shared 构建（含 libsvtav1 / libaom-av1 / librav1e）
$url = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-gpl-shared-8.1.zip'
Invoke-WebRequest $url -OutFile "$env:TEMP\ffmpeg.zip"

# 解包并放到 tools/ffmpeg
Expand-Archive "$env:TEMP\ffmpeg.zip" "$env:TEMP\ffmpeg-x"
$src = (Get-ChildItem "$env:TEMP\ffmpeg-x" -Directory | Select-Object -First 1).FullName
New-Item -ItemType Directory -Force tools\ffmpeg | Out-Null
Copy-Item "$src\bin","$src\include","$src\lib","$src\LICENSE.txt" -Destination tools\ffmpeg -Recurse
```

网络受限时先设置代理，例如：`$env:HTTPS_PROXY = 'http://<代理地址>:<端口>'`。

## 校验（AV1 软件编码器必须在列）

```powershell
.\tools\ffmpeg\bin\ffmpeg.exe -hide_banner -encoders | Select-String 'av1'
```

期望至少看到：

```
 V....D libaom-av1           libaom AV1 (codec av1)
 V....D librav1e             librav1e AV1 (codec av1)
 V..... libsvtav1            SVT-AV1(Scalable Video Technology for AV1) encoder (codec av1)
```

## 注意

- **ABI 要与 `ffmpeg-next 8.1` 匹配**：libavcodec 主版本须为 **62**
  （`avcodec-62.dll`）。换其它大版本需要同步改 `app/Cargo.toml` 的 `ffmpeg-next`。
- 程序同时以两种方式使用 FFmpeg，两套都要满足：
  1. **子进程** `ffmpeg.exe`（编码、硬件解码管道）→ 需要 `bin/` 下的 exe 与 DLL；
  2. **进程内** `ffmpeg-next`（探测、软件解码）→ 需要 `include/` 头文件与 `lib/*.lib`。
- 因此要选 **shared** 构建（自带 `include/` + `lib/`）；`-gpl.zip`（静态单文件）
  不带头文件和导入库，无法用于本项目的进程内链接。
- 打包时 `package.ps1` 会从该目录复制 `bin/*.dll`、`ffmpeg.exe` 与许可证文件。

## 改 .ps1 脚本时注意（重要）

本仓库的 `.ps1` 里含中文注释，**必须保存为「UTF-8 带 BOM」**。

Windows PowerShell 5.1 读取无 BOM 的 `.ps1` 时按系统 ANSI（中文系统是 GBK）解码，
中文注释会变成乱码并**破坏脚本解析** —— 表现是脚本一行都不输出、直接以退出码 1 结束，
很容易误判成"程序起不来"。PowerShell 7 默认按 UTF-8 读，所以只在 5.1 下暴露。

检查与修复：

```powershell
# 检查是否有 BOM（输出 True 才安全）
$b = [IO.File]::ReadAllBytes('run.ps1'); ($b[0] -eq 0xEF -and $b[1] -eq 0xBB -and $b[2] -eq 0xBF)

# 补上 UTF-8 BOM
$t = [IO.File]::ReadAllText('run.ps1', [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText('run.ps1', $t, (New-Object Text.UTF8Encoding($true)))
```

验证脚本在 5.1 下能被解析（比运行更快定位这类问题）：

```powershell
& "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -Command {
  $e = $null
  [void][System.Management.Automation.Language.Parser]::ParseFile("$PWD\run.ps1", [ref]$null, [ref]$e)
  if ($e) { $e | ForEach-Object { "line $($_.Extent.StartLineNumber): $($_.Message)" } } else { 'PARSE OK' }
}
```