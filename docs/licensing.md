# TraceReel 许可与对应源码

本页说明通过微软商店安装的 TraceReel（迹映）适用什么许可，以及在哪里获取对应源码。

## 许可

TraceReel 以 **GPL-3.0-or-later** 发布，并附一条**第 7 条应用商店例外**（允许通过应用商店分发，即使商店条款与 GPL 不兼容；条件是源码仍通过无这些限制的渠道提供）。完整许可文本见仓库根目录的 [LICENSE](https://github.com/WaterFishJ/tracereel/blob/main/LICENSE)。

从微软商店安装的版本，其许可以上述 GPL 条款为准；商店默认的标准应用许可条款中与 GPL 冲突的部分不适用于本程序。

## 对应源码

- 主仓库（每个商店版本对应同一版本标签）：<https://github.com/WaterFishJ/tracereel>
- 防抖核心 `gyroflow-core` 的 fork（子模块提交在源码包中记录）：<https://github.com/WaterFishJ/gyroflow>
- 对应源码包：每个 GitHub Release 附带 `TraceReel-<版本>-source.zip`，内含主仓库、`gyroflow-src` 子模块与 `THIRD-PARTY-SOURCE.md`（第三方组件的版本与源码获取方式）。
- 第三方组件的许可与来源：见 [THIRD-PARTY.md](https://github.com/WaterFishJ/tracereel/blob/main/THIRD-PARTY.md)。

你可以自行编译源码，或基于源码制作修改版本；GPL 授予你的权利不因通过商店安装而减少。

## 再分发

按 GPLv3 再分发本程序（或其修改版）时，请保留 LICENSE、THIRD-PARTY.md 与 `licenses/` 目录，并让接收者能够取到与所分发二进制对应的完整源码。
