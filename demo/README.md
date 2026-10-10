# demo（开发用脚本与可选夹具）

本目录**只放开发/逆向用的脚本，不放任何素材**：

- 真实赛事地图（`.qrt` 内嵌地图）、视频、GPX、渲染截图一律不入库 —— 既有版权问题，
  也会夹带个人路径。
- `example.trr` 是一个**结构完整、路径中性**的工程模板（路径全部指向 `C:/media/...`）。
  要真跑起来，请把里面的路径改成你自己的文件。`app/src/settings.rs` 的默认值单测
  就 `include_str!` 这个文件，所以它必须留在仓库里。

## 脚本

| 脚本 | 作用 | 依赖 |
|---|---|---|
| `qrt_dump.ps1` | 权威交叉验证：用 .NET `BinaryFormatter` 把 `.qrt` 反序列化成 JSON | PowerShell 5.1 / .NET Framework |
| `qrt_verify.ps1` / `qrt_verify.py` | 校验 dump 出来的 Handle ↔ 矩阵 ↔ 航点索引 | 上一步的 JSON |
| `qrt_pairs.ps1` | 校验「相邻两点反推矩阵」与文件里存的矩阵一致 | 同上 |
| `qrt_objgraph.py` / `qrt_reader.py` / `qrt_map_extract.py` | 纯 Python 的对象图探索与内嵌地图提取 | Python 3 |
| `gen_qrtlike.ps1` / `gen_qrtlike2.ps1` / `gen_ref.ps1` / `gen_ref2.ps1` | 生成 .NET 参照流，用来对照类表布局 | PowerShell 5.1 |
| `qrt_panel_stats.ps1` | 统计渲染图左侧地图面板的颜色数 / 路线像素 | Pillow |
| `qml_assert_bisect.ps1` | 按大括号配平切 `gui.qml` 做二分，定位触发 Qt 断言的 QML 块 | Qt + `qmlprobe` |

用法与结论见 [docs/qrt格式逆向笔记.md](../docs/qrt格式逆向笔记.md)。

## 放自己的夹具

把样例文件直接放进本目录即可：`.gitignore` 已忽略
`demo/*.qrt`、`*.jpg`、`*.jpeg`、`*.png`、`*.mp4`、`*.gpx`、`*.orv`，
不会被误提交（`example.trr` 是显式入库的例外）。
