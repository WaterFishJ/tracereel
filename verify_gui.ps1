# 无人值守地跑一次 tracereel-gui，用来验证 QML 改动（qml 是运行时从磁盘读的，改完不用重编译）。
#
# 做三件事：
#   1. 按 run.ps1 的方式准备好 Qt / ffmpeg 环境；
#   2. 自动点掉 debug 版 Qt 断言对话框（"忽略(&I)"），否则进程会卡在对话框上；
#   3. 收集 stdout/stderr，并按关键字过滤（默认抓 qml: 输出和 QML 运行期错误）。
#
# GUI 子系统默认把 QML 的 console.log 发给调试器，所以这里设 QT_FORCE_STDERR_LOGGING=1
# 让它落到 stderr 里 —— QML 里临时加个 Timer + console.log("[uiverify] ...") 即可验证
# 界面内部状态（记得验证完删掉钩子）。
#
# 用法:
#   .\verify_gui.ps1                                  # 用 demo\example.trr 跑 15 秒
#   .\verify_gui.ps1 -Orv demo\example.trr -Seconds 25
#   .\verify_gui.ps1 -Pattern 'uiverify|Error'        # 自定义过滤
param(
    [string]$Orv = 'demo\example.trr',
    [int]$Seconds = 15,
    [string]$Pattern = 'qml:|RangeError|TypeError|Binding loop|is not a function|undefined|Error'
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $Root 'target\debug\tracereel-gui.exe'
if (-not (Test-Path $exe)) { Write-Output "找不到 $exe（先跑 .\build.ps1 build -p tracereel --bin tracereel-gui）"; exit 1 }
if ($Orv -and -not [System.IO.Path]::IsPathRooted($Orv)) { $Orv = Join-Path $Root $Orv }

$log = Join-Path $env:TEMP 'tracereel_verify_out.txt'
Remove-Item $log -ErrorAction SilentlyContinue

# 与 run.ps1 一致的运行环境（否则找不到 Qt6 DLL，进程会立刻退出）
$QtRoot = if ($env:TRACEREEL_QT_ROOT) { $env:TRACEREEL_QT_ROOT } else { 'C:\Qt\6.8.2\msvc2022_64' }
$QtBin = Join-Path $QtRoot 'bin'
$QtQml = Join-Path $QtRoot 'qml'
$FfmpegBin = if ($env:TRACEREEL_FFMPEG_DIR) { Join-Path $env:TRACEREEL_FFMPEG_DIR 'bin' } else { Join-Path $Root 'tools\ffmpeg\bin' }
if (-not (Test-Path (Join-Path $FfmpegBin 'ffmpeg.exe'))) { throw "找不到 ffmpeg.exe：$FfmpegBin（获取方式见 packaging\ffmpeg获取说明.md）" }
$env:QML_IMPORT_PATH = $QtQml
$env:QML2_IMPORT_PATH = $QtQml
$env:QT_QUICK_CONTROLS_STYLE = 'Fusion'
$env:QT_FORCE_STDERR_LOGGING = '1'
$env:QT_LOGGING_TO_CONSOLE = '1'
$env:QT_ASSUME_STDERR_HAS_CONSOLE = '1'
$env:PATH = "$QtBin;$FfmpegBin;$env:PATH"

Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class Win32Click {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, int msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);

  public static System.Collections.Generic.List<string> Clicked = new System.Collections.Generic.List<string>();
  static string Text(IntPtr h) { var sb = new StringBuilder(512); GetWindowTextW(h, sb, 512); return sb.ToString(); }
  static string Cls(IntPtr h) { var sb = new StringBuilder(256); GetClassNameW(h, sb, 256); return sb.ToString(); }
  public static int Scan(uint targetPid) {
    int clicks = 0;
    EnumWindows((h, l) => {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid != targetPid) return true;
      if (Cls(h) != "#32770") return true;
      EnumChildWindows(h, (c, l2) => {
        string t = Text(c), cls = Cls(c);
        if (cls == "Button" && (t.Contains("忽略") || t.ToLower().Contains("ignore"))) {
          SendMessageW(c, 0xF5 /*BM_CLICK*/, IntPtr.Zero, IntPtr.Zero);
          Clicked.Add(t); clicks++;
        }
        return true;
      }, IntPtr.Zero);
      return true;
    }, IntPtr.Zero);
    return clicks;
  }
}
'@

$job = Start-Job -ScriptBlock {
    param($exe, $orv, $log)
    & $exe $orv *>&1 | Tee-Object -FilePath $log
} -ArgumentList $exe, $Orv, $log

Start-Sleep -Milliseconds 1200
$proc = Get-Process -Name 'tracereel-gui' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Output 'PROCESS-NOT-STARTED'; exit 1 }
Write-Output ("pid=" + $proc.Id + "  orv=" + $Orv)

$deadline = (Get-Date).AddSeconds($Seconds)
$total = 0
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 700
    try { $p = Get-Process -Id $proc.Id -ErrorAction Stop } catch { break }
    $total += [Win32Click]::Scan([uint32]$p.Id)
}
Write-Output ("assertClicks=" + $total + " -> " + ([Win32Click]::Clicked -join ','))

Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Stop-Job $job -ErrorAction SilentlyContinue
Remove-Job $job -Force -ErrorAction SilentlyContinue

Write-Output ("--- LOG (" + $Pattern + ") 全量: " + $log + " ---")
if (Test-Path $log) {
    Get-Content -Encoding UTF8 $log | Select-String -Pattern $Pattern | ForEach-Object { $_.Line }
} else { Write-Output 'no log file' }
