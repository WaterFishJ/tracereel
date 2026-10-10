# TraceReel · 迹映

定向越野视频制作与路线回放软件：把实拍视频、定向地图、轨迹（QuickRoute `.qrt` / GPX / GPX 叠加图）
和运动数据放到同一条时间轴上，并用陀螺数据做防抖，最后导出成片。

**让路线成为影像。**

> TraceReel 是 [OrientView](https://github.com/mikoro/orientview)（C++/Qt）的 Rust 重写，
> 整合了 [QuickRoute](https://github.com/matstroeng/quickroute) 的工程格式、
> [Gyroflow](https://github.com/gyroflow/gyroflow) 的防抖核心与 ffmpeg 编码。
> 许可为 **GPL-3.0-or-later**，来源、第三方组件与 GPL 义务见 [THIRD-PARTY.md](THIRD-PARTY.md)。

## 功能

**视频与预览**

- ffmpeg 解码，软解 + `d3d11va` / QSV 硬解；播放由按 fps 的时间轴驱动，拖动进度可即时出帧。
- 键盘 seek 多档步长（可配置），地图/轨迹可平移、旋转、缩放。
- 窗口标题实时显示防抖状态（加载中 / 已就绪 / 无运动数据）。

**地图、轨迹与分段**

- 轨迹来源：QuickRoute `.qrt`（内嵌地图 + 轨迹 + 分段）、QuickRoute 导出的 `.gpx.jpg`、GPX、普通图片底图。
- 分段取景、跟随点、尾巴（可切换成配速色带）、控制点显示、按时间换多张地图。
- `.qrt` 里带分段时可直接导入为分段时刻。

**防抖（Gyroflow）**

- 进程内链接 `gyroflow-core`（本仓库的 fork 子模块），与原生 Gyroflow 1.6.3 用同一预设时逐帧对齐。
- 稳定化参数以设置页为唯一真源，不依赖 `.gyroflow` 文件（预设只取其镜头标定）。
- 多文件（分段）导出带**接缝预热平滑**：用相邻分段的原始四元数做 3 秒预热，消除接缝处的稳定化重置。

**导出**

- 分段拼接导出：每段用自己的源音轨，接缝处 30 毫秒等功率交叉淡化，避免画面跳变与音频断点。
- H.264 / HEVC / AV1，硬件（QSV / NVENC / AMF）与软件编码，可回退。
- HUD 叠加（用时 / 分段 / 配速 / 距离 / 心率 / 海拔 / 指南针等），地图与轨迹可单独开关。
- 导出队列与导出日志；中止后已经写出的片段仍可播放。

**票据识别**

- `receipt-ocr`：ONNX Runtime + PP-OCRv4 模型的收据/表格识别，预处理与后处理都是纯 Rust。

## 系统要求

- **Windows 10 / 11 x64**。目前只支持 Windows：硬解走 `d3d11va`/QSV，构建脚本用 MSVC 与 `windeployqt`。
- 便携包开箱即用；从源码构建见下节。
- 硬件编码需要相应的显卡驱动；没有时自动回退软件编码。

## 安装

### 方式一：便携包（推荐）

到本仓库的 **Releases** 下载 `TraceReel-<版本>-windows-x64-<时间戳>-portable.zip`，
**完整解压**到一个普通文件夹后双击 `TraceReel.exe`。不需要安装 Rust、Qt 或 FFmpeg。
ZIP 旁的 `.sha256` 是校验值。

### 方式二：从源码构建

见下节。

## 从源码构建

### 0. 前置

| 依赖 | 版本 | 说明 |
|---|---|---|
| Rust | stable，`x86_64-pc-windows-msvc` | <https://rustup.rs> |
| Visual Studio 2022 | 含「使用 C++ 的桌面开发」 | 提供 MSVC x64 工具链与 Windows SDK（`rc.exe`） |
| Qt | 6.8.2 `msvc2022_64` | 需要 Quick / QML 模块 |
| FFmpeg | 8.1 共享库构建，**必须含软件 AV1 编码器**（`libsvtav1` 或 `libaom-av1`） | 获取步骤见 [packaging/ffmpeg获取说明.md](packaging/ffmpeg获取说明.md) |
| LLVM / clang | 较新版本即可 | rust-cpp 需要 `libclang.dll` |
| Python 3 | 可选 | 只有 `demo/*.py` 与 `tools/export_branding.py` 需要 |

### 1. 取源码（含子模块）

```powershell
git clone --recurse-submodules https://github.com/WaterFishJ/tracereel.git tracereel
cd tracereel
# 已经 clone 但没带子模块时：
git submodule update --init
```

`gyroflow-src` 是 TraceReel 维护的 Gyroflow fork（GPL-3.0，地址见 [.gitmodules](.gitmodules)；
基线为上游 gyroflow 提交 `81c9eda`），提供 `gyroflow-core` 和镜头标定数据库
`profiles.cbor.gz`。改动只有两个源文件（接缝预热平滑），便于日后 rebase 到新的上游版本。

### 2. FFmpeg

按 [packaging/ffmpeg获取说明.md](packaging/ffmpeg获取说明.md) 下载并解压到 `tools/ffmpeg`
（该目录不入库）。确认 `tools\ffmpeg\bin\ffmpeg.exe -hide_banner -encoders` 的输出里有
`libsvtav1` 或 `libaom-av1`，否则「CPU + AV1」导出会失败。

### 3. 环境变量

| 变量 | 默认值 | 作用 |
|---|---|---|
| `TRACEREEL_QT_ROOT` | `C:\Qt\6.8.2\msvc2022_64` | Qt 目录 |
| `TRACEREEL_FFMPEG_DIR` | `<仓库>\tools\ffmpeg` | ffmpeg 开发包与 `bin` |
| `LIBCLANG_PATH` | 自动探测 LLVM / pip 安装的 clang | `libclang.dll` 所在目录 |

### 4. 构建与运行

```powershell
.\build.ps1 build -p tracereel --bin tracereel-gui   # 只构建 GUI
.\run.ps1                                            # 编译检查后启动（空白工程）
.\run.ps1 demo\example.trr                           # 带一个模板工程启动（路径要自己改）
.\run.ps1 --Release                                  # release 启动
```

`run.ps1` 每次都交给 cargo 按内容指纹判断是否需要重编译，因此不会启动过期程序；确实想跳过
检查时用 `--SkipBuild`。首次全量构建要编译 wgpu / gyroflow-core / Qt 绑定，耗时较长。

### 5. OCR 资源（只影响票据识别）

```powershell
.\ocr\setup.ps1     # 下载 ONNX Runtime 1.22 与 PP-OCRv4 模型到 tools\ocr
```

不需要 Python；打包脚本会自动调用它。

## 打包便携版

```powershell
.\package.cmd        # 等价于 .\package.ps1；-SkipBuild 只重新收集依赖并压缩
```

产物在 `dist\`：`TraceReel-<版本>-windows-x64-<时间戳>-portable.zip` 与同名 `.sha256`。
可用 [packaging/verify-package.ps1](packaging/verify-package.ps1) 解压校验。

## 工程与用户数据

- 工程文件默认 `.trr`，同时支持打开旧 `.orv`（内容仍是兼容的 INI 格式）。
- 用户数据写在 `%APPDATA%\TraceReel`（预设、HUD 模板、导出队列）。首次运行时若发现旧的
  `OrientView` 目录会复制过来，保留原文件且不覆盖已存在的 TraceReel 文件。
- **本仓库不包含任何 `.gyroflow` 预设**：预设绑定你自己的相机与偏好，请用 Gyroflow 调好后
  保存到 `presets\`（见 [presets/README.md](presets/README.md)）。

## 仓库结构

| 路径 | 内容 |
|---|---|
| `app/` | `tracereel` 主 crate：GUI（QML + qmetaobject）、解码/编码、防抖、HUD、导出 |
| `tracereel-core/` | 纯逻辑 crate：投影与渲染数学、QuickRoute `.qrt` / GPX 解析、RouteManager、分段 |
| `receipt-ocr/` | 票据 OCR crate：ONNX Runtime + PP-OCRv4 推理与版式还原 |
| `gyroflow-src/` | **子模块**：TraceReel 的 Gyroflow fork（GPL-3.0），提供 `gyroflow-core` |
| `assets/branding/` | 品牌资源（矢量主文件 + PNG/ICO），构建时嵌入可执行文件 |
| `docs/` | 设计笔记与防回归手册（中文） |
| `demo/` | 开发用脚本与中性工程模板（不含任何素材） |
| `ocr/` | OCR 运行时下载脚本与许可文本 |
| `packaging/` | 打包说明、文件哈希与校验脚本 |
| `presets/` | 预设目录（仓库内为空，只放说明） |
| `tools/` | 辅助脚本；`tools/ffmpeg` 与 `tools/ocr` 需自行下载，不入库 |

## 测试

```powershell
cargo test -p tracereel-core -p receipt-ocr    # 纯逻辑，不需要 Qt / ffmpeg / 子模块
cargo check --workspace --all-targets          # 编译全部目标（需要 Qt、ffmpeg 与子模块）
```

`app` 里有一部分单元测试依赖真实素材（DJI 视频、`.qrt` 地图）。素材不在仓库中，文件缺失时
这些用例会自动跳过，因此可以用环境变量指定自己的样本：

```powershell
$env:TRACEREEL_TEST_DJI_VIDEO   = 'D:\media\DJI_xxx.MP4'
$env:TRACEREEL_TEST_DJI_0176    = 'D:\media\DJI_0176.MP4'
$env:TRACEREEL_TEST_QUICKROUTE_JPEG = 'D:\media\route.gpx.jpg'
```

界面与导出的手工/无头验证：`build.ps1`、`run.ps1`、`verify_gui.ps1`，细节见
[docs/对齐gyroflow-面板与风格.md](docs/对齐gyroflow-面板与风格.md) 与
[docs/防抖问题与修复总结.md](docs/防抖问题与修复总结.md)。

## 常见问题

- **构建报找不到 Qt / 找不到 libclang / 找不到 ffmpeg**：分别设置 `TRACEREEL_QT_ROOT`、
  `LIBCLANG_PATH`、`TRACEREEL_FFMPEG_DIR`。
- **首次构建很慢**：正常，`gyroflow-core` 含 wgpu 与 shader 编译，请耐心等待。
- **「CPU + AV1」导出失败**：所用 ffmpeg 没有软件 AV1 编码器，换 [packaging/ffmpeg获取说明.md](packaging/ffmpeg获取说明.md) 里的构建。
- **Windows 提示「未知发布者」**：便携包未做代码签名，选择「更多信息 → 仍要运行」即可，
  或自行校验 ZIP 的 SHA256。
- **防抖显示「无运动数据」**：该视频没有陀螺/IMU 数据（例如未开防抖录制的素材），属于正常。
- **导入分段后地图不显示**：确认地图与轨迹用的是同一个 `.qrt`；渲染取景依赖 `.qrt` 里的
  Handle 矩阵，只有经纬度而没有配准信息的地图无法取景。

## 许可与致谢

TraceReel 以 **GPL-3.0-or-later** 发布（见 [LICENSE](LICENSE)）。之所以是这个许可：
`gyroflow-core` 是链接进同一进程的 GPL 代码，同时本项目移植了 OrientView 与 QuickRoute 的
GPL 实现，因此整体必须是 GPL。

致谢：

- **OrientView** — Mikko Ronkainen（GPL-3.0-or-later）：原程序与界面/渲染设计。
- **QuickRoute** — Mats Troeng、Jörgen Ohlin（GPL-3.0+）：`.qrt` 工程格式与轨迹语义。
- **Gyroflow** — Adrian、Elvin Chen 等（GPL-3.0）：防抖核心与镜头标定库。
- **FFmpeg**、**Qt**、**ONNX Runtime**、**PaddleOCR / RapidOCR**：见 [THIRD-PARTY.md](THIRD-PARTY.md)。

分发本程序的二进制时，请一并提供对应源码（本仓库的同一标签 + `gyroflow-src` 子模块提交）。
