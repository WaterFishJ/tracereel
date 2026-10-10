# 参与开发

欢迎提 issue 与 PR。这个仓库目前以 Windows 为主要平台，构建方式见
[README 的「从源码构建」](README.md#从源码构建)。

## 提交前请确认

```powershell
cargo test -p tracereel-core -p receipt-ocr    # 纯逻辑单测，必须全绿
cargo check --workspace --all-targets          # 需要 Qt、ffmpeg 与 gyroflow-src 子模块
```

如果改了 GUI 或导出，请在 PR 描述里写清楚验证方式（用了什么素材、跑了哪条命令、
看到了什么结果）。GUI 相关的手工/无头验证手法见
[docs/对齐gyroflow-面板与风格.md](docs/对齐gyroflow-面板与风格.md)。

## 约定

- **`.ps1` 必须存为 UTF-8 带 BOM**。Windows PowerShell 5.1 对没有 BOM 的脚本按系统 ANSI
  （简体中文环境是 GBK）解码，中文注释会变成乱码并吃掉引号，报「表达式或语句中包含意外的
  标记」。`.cmd` 相反，**不能**带 BOM。仓库根目录的 `.gitattributes` 只负责行尾，BOM 需要
  你自己保证（VS Code 右下角选「UTF-8 with BOM」）。
- 行尾交给 `.gitattributes`（源码 LF），不要依赖本地的 `core.autocrlf`。
- 注释与文档用中文；标识符、API 名与格式名（QSV、H.264、AV1、`.gyroflow`）保留原文。
- 不要往仓库里放素材：真实定向地图（`.qrt` 内含地图）、视频、GPX、渲染截图一律不入库，
  `.gitignore` 已挡掉常见类型；需要样例时放在本地 `demo/` 下，它不会被提交。
- 也不要把个人预设（`.gyroflow`）提交上来，见 [presets/README.md](presets/README.md)。

## 提交信息

- 首行一句话说清「改了什么、为什么」，必要时在正文写验证方式与影响面。
- 一次提交只做一件相关的事；纯格式化请单独提交，别混进功能改动。
