# docs 索引

这里是开发笔记：记录「为什么这么做」以及「哪些坑不能再踩」。语言为中文，与代码注释一致。

| 文档 | 内容 | 什么时候读 |
|---|---|---|
| [防抖问题与修复总结.md](防抖问题与修复总结.md) | 从防抖集成到「预览与导出观感一致」过程中踩过的全部问题、根因与修复，以及不能再改回去的关键点 | 改 `app/src/gyro.rs`、`app/src/pipe.rs`、`app/src/settings.rs` 之前 |
| [防抖预设与镜头标定库.md](防抖预设与镜头标定库.md) | `.gyroflow` 预设的字段语义、镜头标定库的加载与搜索、与 Gyroflow 面板的对应关系 | 动预设/镜头标定/防抖参数时 |
| [对齐gyroflow-面板与风格.md](对齐gyroflow-面板与风格.md) | 稳定化面板与原生 Gyroflow 的逐项对齐、界面风格、QML 改动的验证手法 | 改设置面板/QML 时 |
| [qrt格式逆向笔记.md](qrt格式逆向笔记.md) | QuickRoute `.qrt` 的容器、类表、对象图逆向过程与结论，Handle 的几何语义 | 动 `.qrt` 读取或分段导入时 |

相关文档：

- [../packaging/ffmpeg获取说明.md](../packaging/ffmpeg获取说明.md) — ffmpeg 开发包获取与校验
- [../packaging/打包说明.md](../packaging/打包说明.md) — 便携版打包流程
- [../ocr/README.md](../ocr/README.md) — OCR 运行时与模型
- [../demo/README.md](../demo/README.md) — 开发用脚本（不含素材）
- [../THIRD-PARTY.md](../THIRD-PARTY.md) — 第三方组件与许可
