## 改了什么

<!-- 一句话说明；关联的 issue 用 #编号。 -->

## 为什么

## 验证方式

<!--
必填。写清楚跑了什么命令、用了什么素材、看到的结果。
纯逻辑改动请贴 `cargo test -p tracereel-core -p receipt-ocr` 的结果；
GUI / 导出改动请说明手工验证步骤。
-->

## 检查

- [ ] `cargo test -p tracereel-core -p receipt-ocr` 全绿
- [ ] `cargo check --workspace --all-targets` 通过（本地环境）
- [ ] 没有把素材（真实地图、视频、GPX、截图）或个人预设（`.gyroflow`）提交进来
- [ ] 改动过的 `.ps1` 仍是 UTF-8 **带 BOM**（`.cmd` 不带 BOM）
