# 第三方组件与许可

TraceReel 以 **GPL-3.0-or-later** 发布。这不是随手选的：本项目在**同一个进程**里链接了
GPL 代码（`gyroflow-core` 是 Cargo 路径/子模块依赖，不是外部进程），并且移植了另外两个
GPL 项目的算法。因此整个程序必须是 GPL，不能改成 MIT/Apache 之类的宽松许可。

## 上游项目（本项目的来源）

| 项目 | 作者 | 许可 | 用途 |
|---|---|---|---|
| [OrientView](https://github.com/mikoro/orientview) | Mikko Ronkainen | GPL-3.0-or-later | 原始程序：界面布局与交互、地图/轨迹渲染、RouteManager 取景、设置模型 |
| [QuickRoute](https://github.com/matstroeng/quickroute) | Mats Troeng、Jörgen Ohlin | GPL-3.0+ | `.qrt` 工程格式与轨迹/分段语义、配速色带算法 |
| [Gyroflow](https://github.com/gyroflow/gyroflow) | Adrian、Elvin Chen 等 | GPL-3.0（含第 7 条应用商店例外） | 陀螺防抖核心（`gyroflow-core`，以子模块 fork 链接进同一进程） |
| [gyroflow/lens_profiles](https://github.com/gyroflow/lens_profiles) | Gyroflow 项目 | 同 Gyroflow | 相机镜头标定数据库 `profiles.cbor.gz` |

> Gyroflow 在其 README 中说明：其二进制在部分构建下会链接闭源 `mdk-sdk`，并为此附带
> GPL 第 7 条的额外许可。TraceReel 只使用 `gyroflow-core`，未启用 `mdk-sdk`。

## 随便携包一起分发的组件

| 组件 | 许可 | 说明 |
|---|---|---|
| Qt 6.8.2 | GPL-3.0 / LGPL-3.0（动态链接） | 采用动态链接；本项目整体已是 GPL，无需商业许可 |
| FFmpeg 8.1（GPL 构建，[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds)） | GPL-2.0-or-later | 以独立可执行文件 + 动态库形式分发 |
| SVT-AV1、libaom | 随 FFmpeg 的 GPL 构建一同分发 | 软件 AV1 编码回退 |
| Microsoft Visual C++ 运行库 | Microsoft 再分发许可 | 版权归 Microsoft Corporation |
| ONNX Runtime 1.22.0 | MIT | CPU 推理运行时（`onnxruntime.dll`） |
| [ort](https://github.com/pykeio/ort) 2.0.0-rc.10 | MIT OR Apache-2.0 | Rust 绑定（`load-dynamic`） |
| PaddleOCR PP-OCRv4 模型 | Apache-2.0 | 文本检测/识别模型 |
| [RapidOCR](https://github.com/RapidAI/RapidOCR) 1.4.4 模型分发 | Apache-2.0 | 模型打包来源 |

各许可原文随便携包放在 `licenses/` 目录下；OCR 相关许可文本也同时放在 `ocr/licenses/`。

## Rust 依赖

第三方 crate 及其许可见 `Cargo.lock`。需要完整清单时可运行：

```powershell
cargo install cargo-about   # 或 cargo install cargo-license
cargo about generate about.hbs > RUST-DEPS.md
```

## GPL 的源码对应义务

分发本程序的二进制（含便携包）时，必须同时提供**完整对应的源码**。本仓库即对应源码：

- 主仓库：本仓库的 `v0.2.1` 标签；
- 防抖核心：`gyroflow-src` 子模块指向的 fork 提交（含 TraceReel 的改动），地址
  <https://github.com/WaterFishJ/gyroflow>（分支 `tracereel`）；基线为上游 gyroflow 的提交
  `81c9eda`（"Allow exporting IMU samples in one sample per frame mode"），差异只有
  `src/core/gyro_source/mod.rs` 与 `src/core/lib.rs` 两个源文件、一个 `.gitattributes`
  和镜头库 `resources/camera_presets/profiles.cbor.gz`。

再分发时请保留 `LICENSE`、本文件与 `licenses/` 目录，并让接收者能够取到上述源码。
