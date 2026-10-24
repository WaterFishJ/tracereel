# QuickRoute `.qrt` 读取器：格式笔记与实现路线

目标：在 tracereel 的轨迹配置页直接添加 `.qrt`，一次性拿到**地图 + 轨迹 + 分段**，
省掉「在 QuickRoute 里导出 gpx.jpg」这一步。

> **素材与脚本现状**：本文实测用的 `demo/AM 男子短距 第一面.qrt`、`demo/黑色接力.gpx.jpg`、
> `demo/qrt_e2e.orv`、`demo/qrt_split_check.orv` 等样例**都不在仓库里**（真实赛事地图有版权，
> 不入库），要复现请自备 `.qrt`。探索期写的一次性脚本（`demo/qrt_classtable.py`、
> `qrt_geometry.py`、`qrt_values.py`、`qrt_handles.py`、`dump_qrt.ps1`）在整理公开仓库时已移除，
> 但结论与字节布局都完整保留在下面。

## 状态总览

| 能力 | 状态 |
|---|---|
| 内嵌地图提取 | ✅ 字节精确（4278305 字节，3000×4243） |
| 轨迹航点（1475 个有效点，含经纬度） | ✅ 纯 Rust 扫描 |
| 投影原点 `projectionOrigin` | ✅ 纯 Rust |
| 航点时间戳（1474/1475，1 Hz） | ✅ 纯 Rust |
| **地理配准：投影米 → 地图像素** | ✅ 复刻 Handle 矩阵与初始拟合，锚点误差 0.00 px |
| **分段 `Lap`（第一面 29 个 → 28 段；第二面 2 个 → 0 段）** | ✅ 纯 Rust，与用户手工设置一致 |
| Handle（地图上的可拖动标记，**不是**分段） | ✅ 137 个（去重后 138 个中的 137 个带像素坐标） |
| GUI：轨迹页可加 `.qrt` + 分段弹窗 | ✅ 接入；当前工程还没有分段时直接导入（不弹窗），已有分段才问；判断用真实分段数，导入时锁定「绝对时间」。**顺序不能反**：先把地图/轨迹路径落地到工程（`trackRows.writeBack()`），再导分段——`adoptQrtSplits()` 里的 `refreshFields()` 会让轨迹列表按工程重建，未落地的行会被清掉（0.3.0「只导入分段、没导入地图」的回归） |
| 单测 | ✅ tracereel-core 24/24；`cargo run --example qrt_geo` 可打印配准自检 |

### 分段来自 `Lap`，不是 Handle ⚠️（重要更正）

早期把 Handle 的 `value` 当成分段时间、得出 135 个，**是错的**。
QuickRoute 里这是两套东西：

| 概念 | 类型 | 含义 |
|---|---|---|
| **分段** | `Session.laps` → `LapCollection<Lap>` | 打卡点（`LapType`: Start / Lap / Stop） |
| 地图标记 | `Session.handles` → `HandleCollection<Handle>` | 用户在地图上拖动的控制点圆点 |

`Lap` 只有三个字段：`time`（DateTime）、`lapType`（枚举）、`description`（通常为 null）。

**每个工程都自动带 Start + Stop 两个 `Lap`**，用户真正设置的是 `Lap` 类型的那批。
判定规则因此是：**没有 `Lap` 类型条目就没有分段**。

实测两个演示工程：

```
第一面  29 个 Lap = Start(0s) + 27×Lap + Stop(1483s)  → 28 个分段
第二面   2 个 Lap = Start(0s) + Stop(1483s)            →  0 个分段（用户确认没设置）
```

分段时间列表 = **除 Start 外的所有 Lap**（含终点），即第一面 28 项：

```
1:14 1:54 2:59 3:46 4:25 5:00 5:49 6:54 7:20 7:56 8:46 9:09 9:43 10:56
11:31 11:50 12:33 13:28 14:31 15:51 17:42 19:35 20:29 21:07 22:20 23:46 24:32 24:43
```

### `Lap` 实例的字节形态

普通实例（31 字节，自描述）：

```
0x01 | objId(4) | lapClassId(4) | time(8)
     | 0x01 | enumObjId(4) | lapTypeClassId(4) | kind(4) | 0x0A
```

两个 `classId` 是**按文件分配的对象编号**，不能写死：扫一遍收集所有候选，
取「被最多候选共用」的那个 classId 即为 `Lap`。

**每个文件的第一条 `Lap` 写法不同**：它的 `lapType` 枚举是内联定义的，嵌套部分因此变成

```
time(8) | 0x05 | enumId(4) | 0x23 "QuickRoute.BusinessEntities.LapType"
        | 01 00 00 00 | 07 "value__" | <btype> | <primType> | <libId> | kind(4) | 0x0A
```

漏掉这一条就会少一个分段（第一面 27→28、第二面 0→1 都受影响）。两个文件各恰好一条。


### 关于「497 个 Handle」的更正 ⚠️

早期用 `demo/qrt_dump.ps1` 的 **递归遍历**数出 497 个 Handle，那是个**重复计数**：
递归会多次走进同一个 `OrderedBag`/红黑树节点，同一个 `Handle` 对象被访问 3~5 次。

真实的**去重后**数量：

```
dump 原始条目           497
去重后 (x, y, value)    138
其中带地图像素坐标       137   ← 读取器提取到的就是这个数
重复次数分布            1×1, 2×1, 3×66, 4×54, 5×16
```

同理 `waypointCount = 2048` 里后 573 个是全零填充（`lon=lat=time=0`），
有效点是 **1475** 个，与时间戳数组长度一致。

### 纯 Rust 取数（`tracereel-core/src/qrt.rs`）

不依赖 .NET/BinaryFormatter，全部按字节不变式扫描：

| 数据 | 定位不变式 |
|---|---|
| 地图 | `ArraySinglePrimitive` 头 + JPEG 标记链（不是简单找 `FF D9`） |
| 经纬度 | 25 字节定长记录：`lon(f64) lat(f64) 0x01 index(i32)`，连续排列 |
| 投影原点 | 内嵌图之前最后一个 `(lon, lat)` 对 |
| 时间戳 | 34 字节记录：`ticks(i64, ToBinary) …`，连续 stride=34、时间递增 |
| Handle | 以 x 为锚：`value` 在 x−17、`0x01` 在 x−9、`segmentIndex` 在 x−24 |
| Handle 的变换矩阵 | **不读**，由相邻两个 Handle 的 `(p, L)` 按下文公式反推 |

`DateTime.ToBinary()` 把 kind 位并进低 2 位，所以要减去常量
`TICKS_KIND_BIAS = 4611685998607387904` 才是真实 ticks。

**Handle 的 `value` 是「小数航点序号」**（`ParameterizedLocation.Value`），既不是秒数
也不是米数：整数部分是 `segments[segmentIndex].Waypoints[]` 的下标，小数部分在相邻两点间
插值。本题第一面 138 个 Handle 的 `value` 从 0 递增到 1474，正好覆盖 1475 个航点。

（早期笔记写「value 是沿轨迹秒数、分段时间 = value×1000」——**两处都错**，
分段请见上文从 `Lap` 取的规则。）

### Handle 的几何语义：完整地理配准 ⚠️（关键更正，2026）

上面「Handle 只当标记」的说法不完整：**Handle 才是把轨迹贴到地图上的东西**。
每个 `Handle` 带有 `location`(PointD) 与 `transformationMatrix`(GeneralMatrix 3×3)，
矩阵形状固定为

```
[[ a,  b, tx],
 [ b, -a, ty],
 [ 0,  0,  1]]      a = s·cosθ, b = −s·sinθ
```

即「旋转 + 等比缩放 + y 翻转（投影米是 y 向上，像素是 y 向下）+ 平移」，作用对象是
`LongLat.Project(projectionOrigin)` 得到的**投影米坐标**，输出是**地图像素（原点左上角）**。

用 `demo/qrt_dump.ps1`（已扩展输出 handle 的 `matrix` 与 `session.initialTransformationMatrix`）
在 `demo/AM 男子短距 第一面.qrt` 上实测：

```
initialTransformationMatrix = [[5.77981752384538, 0, 1500.0022865791],
                               [0, -5.77981752384538, 2121.50009826141],
                               [0, 0, 1]]
                                 ↑ 缩放 5.78 px/m        ↑ 平移正好是地图中心 (3000/2, 4243/2)
handle[0].matrix            = [[4.5653607697162766, -0.26946146514501368, 1398.5680219781191],
                               [-0.26946146514501368, -4.5653607697162766, 2048.5816105248855],
                               [0, 0, 1]]
```

**矩阵是「导出量」，可以从 Handle 自身反推**，不必去解析埋在对象图里的 `GeneralMatrix`：

* `Session.UpdateTransformationMatrices` 每次拖动 Handle，都会用
  `LinearAlgebraUtil.CalculateTransformationMatrix(p_k, L_k, p_{k+1}, L_{k+1}, …, useRotation: true)`
  **重写第 k 个 Handle 的矩阵**，使它同时把「第 k 个航点」映射到 `L_k`、把「第 k+1 个航点」
  映射到 `L_{k+1}`。
* 也就是 `M_k` 完全由**相邻两个 Handle** 决定：

  ```
  θ = GetAngleR(p_{k+1} − p_k, (Δq.x, −Δq.y))
  s = |q_{k+1} − q_k| / |p_{k+1} − p_k|
  a = s·cosθ ,  b = −s·sinθ ,  t = q_k − M·p_k
  ```

* `tracereel-core/src/qrt.rs::handle_transforms` 就按这个公式算。实测**与文件里存的矩阵逐位吻合到
  1e-10**（136/136 组相邻对）；锚点自检「`M_k·p_k == L_k` 且 `M_k·p_{k+1} == L_{k+1}`」
  在 137 个 Handle 上误差 0.00 px。
* 没有任何 Handle 时的初值 `initialTransformationMatrix` 也能纯算出来，见
  `RouteAdjustmentManager.CreateInitialTransformationMatrix`：把轨迹投影包围盒映射到地图
  中间 3/4 区域（并按轨迹宽高比调整），`useRotation: false` ⇒ 结果是对角矩阵，
  `a = −sy` 恒成立。

**轨迹点位用的是「它前面那个 Handle」的矩阵**，而且一次循环最多前进一个 Handle，
最后一个 Handle 永远不用（`QuickRouteReader::processRoutePoints` 原注释：
"do not use the last handle at all (use second to last handle instead for the last points)"）。
`transform_indices` 就是这个 walk 的移植。

**坐标系：文件里是「左上角原点」，渲染器要「地图中心原点」**。
`QuickRouteReader` 最后一步是 `extraTransformation.scale(...).translate(−w/2, −h/2)`，
导出 jpg 那条路径（`tracereel-core/src/quickroute.rs`）同样减了 `w/2, h/2`。
`.qrt` 路径必须一起减——这就是「**导入分段后地图不显示**」的根因：

```
旧实现：把轨迹包围盒拉满整张地图，且输出左上角坐标
        ⇒ FixedSplit 取景（把当前分段框到面板里）把视点算到偏离地图半张图的位置
        ⇒ 地图被推出可视区 ⇒ 看上去"地图没了"
新实现：Handle 矩阵给出真实像素坐标，再减地图中心
        ⇒ 分段取景框的是地图上真实的这一段，地图自然可见
```

顺便修掉的第二个 bug：`to_route_points` 把**毫秒**写进了 `RoutePoint.time`（该字段单位是**秒**）。
路线管理器按 1 秒重采样（`time[j] − time[i] > 1.0`），单位错了整条轨迹只会被压成一段，
分段取景矩阵随之退化（实测先算出 scale = 11.92，改对后为 0.768）。

现在 `FixedSplit` 在第一面复现出：无分段 scale = 1.000（整图），
有分段 scale = 0.768、`x = −5.0 / y = 617.2`——把第一段框进面板，符合预期。

### 早期 .NET 侧交叉验证（保留）

`demo/qrt_dump.ps1` 用 `BinaryFormatter` + **捕获型 `SerializationBinder`**：
把 QuickRoute/PowerCollections 的每个类型名都绑到一个 `Bag` 桩，
其反序列化构造函数把 name/value 记下来——于是**整张对象图变成纯数据**，
不需要原程序集、也不需要自己解析嵌套泛型。

实测输出（`demo/qrt_verify.py` 校验，全部符合）：


```
mapBytes      = 4278305        (FF D8 FF = JPEG)
projLon/Lat   = 113.0243020, 23.1763540
sessions = 4   segments = 4
waypointCount = 2048
handleCount   = 497            （递归重复计数；去重后 138）
handle 像素范围 x 1295..2114  y 1908..3200（+ 每条的 matrix 与 initialMatrices）
lapsPerSession = 29,0,0,0      （第一面；第二面是 2,0,0,0）
首航点 lon 113.0240430 lat 23.1757680 time 639242590180000000 alt 4.8
末航点 lon 113.0226600 lat 23.1764200 time 639242605010000000 alt 4.5
时间跨度 1483.0 秒（约 24.7 分钟）
```

**这解决了此前所有"取不到值"的问题**：轨迹是 `Route.segments[].waypoints[]`，
Handle 是 `session.handles` → `OrderedBag<Handle>` → `RedBlackTree` →
含 `location`(PointD)、`parameterizedLocation`(segmentIndex, value) 与
`transformationMatrix`(GeneralMatrix) 的节点；分段则是 `session.laps`。

用法：

```
powershell -NoProfile -ExecutionPolicy Bypass `
  -File demo/qrt_dump.ps1 <file.qrt> <out.json>
python demo/qrt_verify.py <out.json>
```

配准自检（纯 Rust，直接读 `.qrt`，不需要 .NET）：

```
cargo run -p tracereel-core --example qrt_geo -- "demo/AM 男子短距 第一面.qrt"
# handle-marker error: worst ... px / pair invariant: worst 0.000 px
```

（注意：PS 5.1 读不了含中文的路径，先把文件复制成 ASCII 名。）

### 已解决：不必再重写 NRBF

曾经写过一版 `nrbf.rs`（独立解析 MS-NRBF 对象图），是走不通的探索路线
（嵌套泛型让成员宽度判定失效）：`qrt.rs` 用字节不变式 + 上面那套几何反推就够了，
不需要解析 `GeneralMatrix`。该实现、它的三个探针 example（`nrbf_probe` /
`nrbf_fields` / `nrbf_dump`）以及 `nrbf-parser` 依赖**已从公开仓库移除**，
本文保留这段探索过程，以免后来者重走。

## 关键交付物

- `tracereel-core/src/qrt.rs` — 地图 + 轨迹 + 投影原点（单测随代码，已可用）
- `demo/qrt_dump.ps1` — **已可取出全部数据**的 .NET 提取器
- `demo/qrt_verify.py` — 提取结果的校验脚本
- `demo/qrt_objgraph.py` — 纯 Python 对象图解析探索
- `demo/qrt_reader.py` — Python 参考读取器（同一算法）
- `demo/qrt_map_extract.py` — 仅提取内嵌地图
- `docs/qrt格式逆向笔记.md` — 本文件

> 已移除（曾用于逆向，公开仓库不再包含）：`tracereel-core/src/nrbf.rs`、
> `tracereel-core/examples/nrbf_{probe,fields,dump}.rs`、依赖 `nrbf-parser`。

---

## 0. 官方源码（关键突破）

QuickRoute 是开源的（Mats Troeng），原 Google Code 项目已迁到 GitHub：

- 源码仓库：<https://github.com/matstroeng/quickroute>（C#，`master`）
- 镜像：<https://github.com/romanbdev/quickroute-gps>
- 原始项目页：<https://code.google.com/archive/p/quickroute-gps/>

命名空间 `QuickRoute.BusinessEntities`，源码里的 `[Serializable]` 类定义
**就是 `.qrt` 的文件格式定义**。（分析时在本地抓过一份关键文件，未随仓库发布。）

> 注意：仓库里**没有** `RouteSegment.cs` / `Waypoint.cs` / `LongLat.cs` 独立文件——
> `RouteSegment` 与 `Waypoint` 定义在 `Route.cs` 末尾（约 2031 行起）。

## 1. 容器总览

`.qrt` 是 QuickRoute 2.4 用 .NET `BinaryFormatter`（MS-NRBF）写出的流。
以 `AM 男子短距 第一面.qrt`（4 434 605 字节）为例：

| 区段 | 偏移 | 说明 |
|---|---|---|
| 序列化头 | 0 .. 16 | `00` + rootId(int32=1) + headerId(int32=-1) + major(1) + minor(0) |
| 类表（BinaryLibrary 0x0C） | 17 .. ~436 | 程序集名 + `typeCount` + 类型定义 |
| 内嵌地图 | 561 816 .. 4 280 247 | `map.rawData` 里的整张 JPEG，3000×4243 CMYK，4 278 305 字节 |
| 对象图 | 4 280 247 .. 4 434 605 | 154 358 字节：轨迹、分段、会话、显示设置 |

**地图是内嵌的**（`MapStorageType.Inline`），不依赖外部文件；
轨迹是矢量数据，不在图片里。

## 2. 序列化结构（来自源码，权威）

所有相关类都实现 `ISerializable`，即按**字段名 → 值**写入，字段名就在流里。

```
Document         → map, settings, sessions, projectionOrigin, properties
Map              → source, sourceType, storageType, rawData(byte[], Inline 时才有)
Route            → segments : List<RouteSegment>
RouteSegment     → waypoints : List<Waypoint>
Waypoint         → altitude, heartRate, longLat, time, mapReadingState, cadence, power
                   （其余 attributes 标了 [NonSerialized]，不进文件）
HandleCollection → handles : OrderedBag<Handle>      (Wintellect.PowerCollections)
Handle           → transformationMatrix(GeneralMatrix), parameterizedLocation,
                   location(PointD), markerDrawer(IMarkerDrawer), type(HandleType)
ParameterizedLocation → segmentIndex(int), value(double)
GeneralMatrix    → 变换矩阵（分段取景用）
```

`HandleType`：`StartHandle = -1`、`Handle = 0`、`EndHandle = 1`。

**分段点就是 `Handle`**：`parameterizedLocation.SegmentIndex` 给段号，
`transformationMatrix` 给该段的变换矩阵，`location` 给地图坐标。
所以「导入分段」要做的事 = 读出 `Document.sessions[*].route.segments` 的分段结构，
或直接读 `HandleCollection`。

## 3. 内嵌地图提取（已跑通 ✅）

规则：定位 `FF D8 FF` 起点后，**按 JPEG 段标记链走到 EOI**。
不能用 `data.find(b"\xff\xd9")`——文件头部的 Photoshop 缩略图里有 `FF FF D9`，
会提前命中，只能拿到 8 KB 缩略图。

脚本：`demo/qrt_map_extract.py`（同时支持 PNG：`89 50 4E 47` … `IEND`）。
已验证两页：4 278 305 / 4 276 981 字节、3000×4243 CMYK，内容正常。

## 4. 轨迹坐标（记录格式已确认 ✅）

对象图里有一段**定长 25 字节**的 waypoint 记录数组：

```
+0    8B   经度 longitude  (double LE)
+8    8B   纬度 latitude   (double LE)
+16   1B   常量 0x01
+17   4B   位置索引 (int32 LE，从 1808 递增到 3282)
+21   4B   常量 00 00 00 00
```

实测：`AM 男子短距 第一面.qrt` 从偏移 **4 362 711** 起连续 **1 475 个**记录，
到 **4 399 561** 结束（索引 1808..3282），相邻点间距 1.1~2.7 m，符合 1 Hz。

**⚠ 重要**：全文件范围内（穷举每 1 字节偏移）只有这 1 475 组经纬度 double，
也就是说索引 0..1807 的点**不是以 double 形式存的**。不要假设"整条轨迹都是这种记录"，
必须靠解析对象图才能拿全（见第 5 节）。

## 5. 实现路线（推荐）

`BinaryFormatter` 流**自描述**：类表给出每个类的成员名，对象图给出值。
既然源码在手，最稳的做法是**写一个 NRBF 解析器**，按第 2 节的字段名取数据：

1. 解析类表 → 得到 `类名 → [成员名]`
2. 解析对象图 → 按 `ISerializable` 的 `name/value` 对取字段
3. 从 `Document` 取 `map.rawData`（地图，直接是图片字节）、
   `sessions[*].route.segments[*].waypoints[*].longLat`（轨迹）、
   `HandleCollection`（分段）

### 解析器要注意的坑（已实测）

- 字符串是 **7 位变长长度 + UTF-8**（不是固定 4 字节长度）
- 类表条目：`[4 字节记录类型][3 字节 0x00][1 字节类名长度][类名][成员数][成员名…]`
  - 参照流（.NET Framework）与 `.qrt` 在这个区域**布局一致**，但 `.qrt` 的
    对象图区整数是 7 位变长，参照流是 4 字节小端——两者要自适应
- **不要靠肉眼数偏移**（这条踩了很久）：用"锚点验证"——先 `find()` 已知类名/字段名
  的真实偏移，再反推字段宽度；写解析器时对每个字符串做
  「读长度 → 读内容 → 校验长度与内容一致」的自校验

### 在 Rust 落地

- `tracereel-core` 新增 `qrt.rs`（纯 Rust、可单测），沿用 `tracereel-core/src/quickroute.rs` 的
  `RoutePoint` / 投影数学（`crate::math::project_coordinate`）
- GUI：轨迹配置页「添加轨迹图片」的过滤器加入 `.qrt`；
  识别到 `HandleCollection` 非空时弹窗「检测到 N 个分段，是否导入？」
- 地图来自 `map.rawData`，用现有图片解码路径处理（可能是 JPEG 或 PNG）

## 6. 类表编码（已用三个参照流 + `nrbf-parser` 交叉验证）

### 6.1 结论：`.qrt` 是**标准** MS-NRBF 流

最初以为 QuickRoute 用了非标准方言，实际不是。用 `nrbf-parser`（crates.io 上
符合 MS-NRBF 规范的实现）直接解析三个**参照流**（自己用 .NET Framework 4.8 生成的
`MiniDoc` / `Font` / `Outer+Inner`），结果完全正确：

```
[0] SerializationHeader { root_id: 1, header_id: -1, major: 1, minor: 0 }
[1] BinaryLibrary { library_id: 2, library_name: "f1uxlnn4, Version=0.0.0.0, ..." }
[2] ClassWithMembersAndTypes {
        class_info: ClassInfo { object_id: 1, name: "Outer",
                                member_count: 2, member_names: ["Name", "Child"] },
        member_type_info: MemberTypeInfo { binary_type_enums: [String, Class],
            additional_infos: [None, Class(ClassTypeInfo{ type_name: "Inner", library_id: 2 })] },
        library_id: 2,
        member_values: [BinaryObjectString{ object_id: 3, value: "hello" },
                        MemberReference{ id_ref: 4 }] }
[3] ClassWithMembersAndTypes { class_info: ClassInfo { object_id: 4, name: "Inner",
        member_count: 3, member_names: ["A","B","C"] }, ... }
[4] MessageEnd
```

标准结构（MS-NRBF 2.3.2.1 / 2.3.1.1）：

```
ClassWithMembersAndTypes = RecordType(1B)=5 | ClassInfo | MemberTypeInfo | LibraryId(4B)
ClassInfo                = ObjectId(4B) | Name(str) | MemberCount(4B) | MemberNames(str*N)
MemberTypeInfo           = BinaryTypeEnum(1B * N) | AdditionalInfos
BinaryLibrary            = RecordType(1B)=0x0C | LibraryId(4B) | LibraryName(str)
```

### 6.2 在 `.qrt` 上：Document 类结构已完整解析 ✅

`nrbf-parser` 解析真实 `.qrt` 的前三条记录完全正确：

```
[0] SerializationHeader { root_id: 1, header_id: -1, major: 1, minor: 0 }
[1] BinaryLibrary { library_id: 2,
      library_name: "QuickRoute.BusinessEntities, Version=2.4.0.0, ..." }
[2] ClassWithMembersAndTypes {
      class_info: ClassInfo { object_id: 1,
        name: "QuickRoute.BusinessEntities.Document",
        member_count: 5,
        member_names: ["map","settings","sessions","projectionOrigin","properties"] },
      member_type_info: [Class(Map), Class(DocumentSettings), Class(SessionCollection),
                         Class(LongLat), Class(DocumentProperties)],
      library_id: 2,
      member_values: [MemberReference(3), MemberReference(4), MemberReference(5),
                      MemberReference(6), ObjectNull] }
```

即：**源码里的对象模型在流里被逐字验证了**——Document 的 5 个成员及其具体类型都在。

### 6.3 唯一剩下的偏差（字节级已定位）

`nrbf-parser` 在**第 4 条记录**上丢同步（`Invalid record type: 101`）。
带偏移打印后：Document 记录报 `105..457`，但实测字节显示它应当在 **458** 结束。

实测字节（`AM 男子短距 第一面.qrt`，逐字节核对过）：

| 偏移 | 内容 |
|---|---|
| 202..206 | `04 04 04 04 04` → 5 个成员的类型码（全 String） |
| 207..431 | 5 条 `ClassTypeInfo`（Map / DocumentSettings / SessionCollection / LongLat / DocumentProperties） |
| 432..435 | `02 00 00 00` → LibraryId |
| 436 | `09 03 00 00 00` → `MemberReference(3)`（map） |
| 441 / 446 / 451 | 同上，分别 ref 4 / 5 / 6（settings / sessions / projectionOrigin） |
| **456** | `0A 05` → `ObjectNullMultiple256`（properties 为 null），**只有 1 个待读值** |
| **458** | `03 00 00 00 1F "QuickRoute.BusinessEntities.Map"` → Map 的 `ClassWithMembers` |

也就是 Document 的值区是 `25 字节（5×MemberReference） + 2 字节（0A 05）= 27 字节`，
结束于 **458**；crate 认为结束于 457，多算了/少算了 1 字节。

**这条记录一修通，剩下的对象图就能顺下来**（`projectionOrigin`、`map.rawData`、
`HandleCollection`、全部 waypoint）。怀疑点是 crate 在
`ObjectNullMultiple256` 的 null_count 与实际待读值数量不一致时的处理。

**这是最后一道坎**：修掉它就能拿到完整对象图（`projectionOrigin`、`map.rawData`、
`HandleCollection`、以及全部 waypoint）。

### 6.4 `projectionOrigin` 已拿到（经纬度 → 地图像素打通 ✅）

`Document.projectionOrigin` 是个 `LongLat`，按声明顺序写 `longitude`、`latitude`
两个 double，**紧接着就是内嵌地图图片**。所以它就是「紧靠图片起点之前的那组经纬度」：

```
偏移 1918: 113.0243020   (longitude)
偏移 1926: 23.1763540    (latitude)
偏移 1936: FF D8 FF E1 … 内嵌地图 JPEG（缩略图）开始
```

已实现（Rust `tracereel-core/src/qrt.rs` 与 `demo/qrt_reader.py` 同一算法）：

- `find_projection_origin(data) -> Option<(lon, lat)>`
- `QuickRouteProject::map_positions() -> Vec<(x, y, index)>`
  正交投影 `Δeast = (lon-olons)·111320·cos(olat)`、`Δnorth = (lat-olat)·110574`，
  再把轨迹包围盒铺到 `map.width × map.height`（纬度向上、像素向下）。
- `to_route_points()` 现在会填 `position_x`/`position_y`（地图像素），
  不再只是经纬度。

实测（`AM 男子短距 第一面.qrt`）：投影原点 `113.0243020, 23.1763540`，
地图像素范围 `x 0..3000, y 0..4243`（1475 点铺满图幅）。

**注意**：包围盒铺图是近似——若地图四周有留白，尺度会略有偏差。
更精确的做法是读 QuickRoute 存的地图地理包围盒（在对象图更深处）。
目前渲染验证：轨迹在面板内的宽高比 `99:336` 与地理范围 `389m:480m` 一致。

### 6.5 类表条目格式：**已解出并验证通过** ✅

`.qrt` 的类表条目（`demo/qrt_classtable.py` 已实现并跑通）：

```
nameLen(1B) | className | memberCount(4B) | (memberNameLen(1B)+memberName)*N
            | binaryTypeEnum(1B)*N | additionalInfos | libraryId(4B)
```

**注意：`.qrt` 的条目没有 record tag、也没有 ObjectId**——直接从「名称长度」开始。
（`.NET` 生成的参照流是另一种写法，带 `05 | ObjectId(4B)` 前缀；两者不要混用，
这是我在这一条上反复出错的原因。）

`AdditionalInfo` 按类型码区分：
`Primitive/PrimitiveArray→1B`、`SystemClass→str`、`Class→(str, INT32)`、
`ObjectArray/StringArray→(INT32 kind, INT32 rank)`。

实测输出（真实 `.qrt`）：

```
typeCount=1, first entry @110
  Document   [('map','Class'), ('settings','Class'), ('sessions','Class'),
              ('projectionOrigin','Class'), ('properties','Class')]
class table: 1 entries, object graph starts @436
bytes at end: 09 03 00 00 00 09 04 00 00 00 09 05 …
```

类表结束于 **436**，紧接着就是 Document 的成员值（`09 xx` = MemberReference）。
**类表这一步是通的**，剩下的难度全在值区。

`.qrt` 里各类名锚点（名称长度字节 + 名称）：

| 类 | 名称位置 | 长度 |
|---|---|---|
| `…Document` | 111 | 36 |
| `…Map` | 207 | 31 |
| `…DocumentSettings` | 243 | 44 |
| `…SessionCollection` | 292 | 45 |
| `…LongLat` | 342 | 35 |
| `…MapSourceType` | 541 | 41 |
| `…MapStorageType` | 587 | 42 |

### 6.6 对象图：已解析出实例值 + 地图数组几何已确认 ✅

用 .NET 同构参照流（`demo/gen_qrtlike.ps1` / `gen_qrtlike2.ps1`）做权威模板，
逐字段对照后 `demo/qrt_objgraph.py` 已能从**真实 .qrt 读出实例值**：

```
library 'QuickRoute.BusinessEntities, …' id=5
[1] Document         ['map','settings','sessions','projectionOrigin','properties']
[2] Map              ['source','sourceType','storageType','rawData']
[3] DocumentSettings ['defaultSessionSettings','colorRangeIntervalSliderSettings','lapHistogramSettings']
[4] SessionCollection['sessions']
[5] LongLat          {longitude: 113.0243020, latitude: 23.1763540}   ← 投影原点
```

**投影原点这次是从对象图里真读出来的**，不再依赖启发式。

#### 内嵌地图数组的精确几何（`demo/qrt_geometry.py` 验证）

```
@1934  0x0F            ArraySinglePrimitive
@1935  int32 = 9       objectId
@1939  int32 = 4278305 length
@1943  0x02            primitiveType = Byte
@1944  FF D8 FF E1 …   地图 JPEG 数据
@1944 + 4278305 - 1 = 4280249   以 FF D9 (EOI) 结束
```

**交叉验证**：该 payload 与 `qrt_reader.py` 提取出的地图**逐字节相等**（4 278 305 字节、
3000×4243）。也就是说之前笔记里"地图位于 561 816"是错的——那是图内的一处 `FF D8`，
真正的地图数组在 **1944**。

#### 关键坑（这轮之前反复卡住的原因）

1. **`0x0A`（ObjectNull）在 `.qrt` 里只占 1 字节**，没有计数字节（.NET 短写法）。
   多读 1 字节就全盘错位——这是"@458 失同步"的真凶。
2. **成员值按声明类型驱动读取**：`ISerializable` 的 double 是裸 8 字节内联，
   没有 `0x08` 标签；`PrimitiveArray` 直接跟 `ArraySinglePrimitive` 记录。
3. **类定义与其实例交替出现**，对象 id 按定义顺序隐式分配。

#### 另一条已验证的路线：.NET 自己的反序列化器能读

`demo/dump_qrt.ps1` 用**真实 QuickRoute 类定义思路**（同名类 + `AssemblyResolve`
把 QuickRoute/PowerCollections 解析到桩程序集）调用 .NET `BinaryFormatter`：

```
FAILED: Unable to find assembly 'PowerCollections, Version=1.0.2141.24679, …'
```

**能报出这个错本身就说明前面的类表/对象头全部解析通过了**——.NET 反序列化器
完全接受这个文件格式。补全桩类型（含 PowerCollections 的 `OrderedBag<T>`）即可
一次性取出全部数据。这是比手写解析器更稳的收尾路线。

### 6.7 分段 Handle：结构已定位，取值未完成

- `HandleCollection` 实例：`count(4B)=2 | refs=[123,124]`（`@4292027`）
- 类定义锚点：`GeneralMatrix`(@4291839)、`HandleCollection`(@4291931)、
  `ParameterizedLocation`(@4301138，成员 `segmentIndex`/`value`)
- 字段名锚点：`transformationMatrix`(@4301017)、`parameterizedLocation`(@4301038)、
  `location`(@4301060)、`segmentIndex`(@4309468)

**未完成原因**：`Handle` 的实例值埋在 `OrderedBag<Handle>` 的嵌套泛型里，
手写解析器在读取 `rawData` 数组后仍需补几处类型分发；上面那条 .NET 路线更可靠。

### 6.8 复现命令

```
# 生成同构参照流（需 PowerShell 5.1 / .NET Framework）
powershell -NoProfile -ExecutionPolicy Bypass -File demo/gen_qrtlike2.ps1

# 解析对象图 / 校验地图数组几何
python demo/qrt_objgraph.py "demo/AM 男子短距 第一面.qrt"
python demo/qrt_geometry.py

# 用 .NET 自己的反序列化器（会提示缺 PowerCollections 桩）
powershell -NoProfile -ExecutionPolicy Bypass -File demo/dump_qrt.ps1
```

### 6.9 下一步

1. **首选**：补全 `demo/dump_qrt.ps1` 的桩类型（`OrderedBag<T>`、`Session`、`Route`、
   `Handle` 等），让 .NET 反序列化器直接吐出分段与完整轨迹；
2. 备选：继续补 `qrt_objgraph.py` 的类型分发，穿过 `OrderedBag` 到 Handle 实例；
3. 两者任一打通后，把结果移植进 `tracereel-core/src/qrt.rs` 并接上已就位的 GUI 弹窗。


## 7. Rust 落地进度

## 7. Rust 落地进度

### 已完成 ✅ `tracereel-core/src/qrt.rs`

```rust
pub fn find_map_image(data: &[u8]) -> Option<MapImage>;      // JPEG/PNG，段链走法
pub fn scan_waypoints(data: &[u8]) -> Vec<QrtWaypoint>;      // 两遍扫描（先全体、再按中位数收窄 ±0.5°）
pub fn read_project<P: AsRef<Path>>(p: P) -> Result<QuickRouteProject, QrtError>;
pub fn read_project_bytes(data: &[u8]) -> QuickRouteProject;
```

`QuickRouteProject { map: Option<MapImage>, waypoints: Vec<QrtWaypoint>,
waypoints_complete: bool, split_count: usize }`，另有 `to_route_points()`
转成 `RoutePoint`（`coordinate_x/y` = 经纬度）。

**为什么必须两遍扫描**：对象图里还有 marker/gradient 表的 double 看起来像坐标
（`12,3`、`-10,10` 这种），一遍全局扫描会多收 10 条假点。第一遍拿中位数当地图中心，
第二遍按 ±0.5° 重扫即可剔除。

测试（`cargo test -p tracereel-core qrt`，4 项全绿，含对真实 demo 文件的断言）：
地图 = JPEG 3000×4243、>4MB、SOI/EOI 正确；轨迹 = 1475 点、首点索引 1808、
首点坐标 (113.0240430, 23.1757680)、`waypoints_complete == false`。

### 已完成 ✅ GUI 接入（轨迹配置页可直接加 .qrt）

| 落点 | 改动 |
|---|---|
| `app/src/bin/gui.qml` `TrackList.fileFilter` | `"*.jpg *.jpeg *.qrt"`（文件对话框可直接选 .qrt） |
| `app/src/bin/gui.qml` 拖放 `imageExt` | 加入 `qrt`（拖进来也认） |
| `app/src/gui.rs` `decode_map_source()` | `.qrt` 用 `tracereel_core::qrt::find_map_image` 取内嵌字节再解码 |
| `app/src/gui.rs` `crop_qrt_header()` | **`.qrt` 内嵌图不做 65px 裁剪**——`.qrt` 存的是原始地图（3000×4243），导出的 QuickRoute JPEG 才带头部条（gpx.jpg 是 6998×5055） |
| `app/src/gui.rs` `init_route_manager()` | 统一入口，`.qrt` 走 `initialize_from_points`，JPEG 走原路径；**GUI 预览与无窗口合成渲染共用**，避免两边漂移 |
| `app/src/gui.rs` `qrt_has_splits` qt_method | QML 可调，判断 `.qrt` 是否带分段 |
| `app/src/bin/gui.qml` `qrtSplitDialog` | 选到带分段的 `.qrt` 时弹窗：`导入分段` / `仅地图 + 轨迹` |
| `tracereel-core/src/route_manager.rs` | 抽出 `set_route_points()` + 新增 `initialize_from_points()` |

### 端到端验证 ✅（不是只有单测）

用 `demo/qrt_e2e.orv`（地图路径与轨迹路径**都指向同一个 `.qrt`**）跑无窗口合成渲染：

```
target\debug\guirender.exe demo\qrt_e2e.orv --at 1000 --out demo\qrt_e2e_render.png
→ wrote demo\qrt_e2e_render.png (1280x720, 1.35 MB)
```

产物分析：9490 种颜色（地图内容），轨迹色 `#0064ff` 像素 914 个
（x 76..175、y 345..681）——**地图与轨迹都真的画出来了**。

顺带修掉一个真实回归：合成渲染路径原本直接调 `rm.initialize()`，
拿 `.qrt` 当 JPEG 解析 → 报 `Not a supported JPEG file format`。
这正是抽出 `init_route_manager()` 的原因。

GUI 本体也实测启动过（`tracereel-gui.exe demo\qrt_e2e.orv`，Fusion 风格，
offscreen 平台）跑满 22 秒未崩溃。

### 多地图换图：换图时间的语义 ⚠️（修正）

### 播放开始时第一段不取景 ⚠️（RouteManager 移植漏了一面旗）

用户实测：导入带分段的 `.qrt` 再导入视频，**播放时第一段地图没有对应旋转缩放**，
只有跳到后面的分段再跳回来才正常。

根因在 `tracereel-core/src/route_manager.rs` 的移植：

* 原版 `calculateSplitTransformations()` 结尾有一句 `instantSplitTransitionRequested = true;`
  ——**整表重算之后，当前取景必须立刻换成新表里的那一项**。
* 原版 `RouteManager::initialize()` 末尾还补了一手：如果第一帧落在任何分段之外
  （`findCurrentSplitTransformationIndex` 取不到），就用第 0 段取景。

Rust 侧两处都漏了。后果：预览面板拿到真实尺寸后调 `window_resized()`
（`full_update_requested = true`）→ 下一帧整表按新尺寸重算，但 `current_st` 还是
**初始窗口尺寸**那份；索引没变化 ⇒ 不会重新应用 ⇒ 第一段一路用错的缩放。
切到别的分段（索引变化触发切换）或来回跳转才会被纠正——与用户描述完全一致。

修法（`route_manager.rs`）：

```rust
self.full_update_requested = false;
self.instant_split_transition_requested = true;      // ← 原版在 calculateSplitTransformations 里
if self.view_mode == ViewMode::FixedSplit {
    let n = self.routes[0].split_transformations.len();
    if n > 0 {                                       // ← 原版 initialize() 末尾那一手，
        let pick = clamp(self.current_split_transformation_index, 0, n - 1);
        self.current_st = self.routes[0].split_transformations[pick];   // 扩成"整表重算后就地补一次"
        self.current_split_transformation_index = pick as i32;
    }
}
```

`set_route_points` 里原来那份重复的初值兜底随之删掉（现在由 `update()` 统一负责）。

两个回归测试（都验证过"去掉修复就失败"）：

```
resize_reapplies_current_split_transform            4.5 -> 4.5   ❌ / ✅ 2.25
resize_reapplies_split_transform_before_departure   1 -> 1       ❌ / ✅ 0.5
```

### 拖入→删除→再拖入变成三行 ⚠️（列表行与 settings 字符串两套事实来源）

用户报：轨迹页和视频页都会「拖入第二个文件 → 删掉它 → 再拖入一个 → 变成三个文件」。

纯「拖入→删除→拖入」在自动化里是 2 行，追不到；真正的机制是两条叠加：

1. **拖入分支直接去拼 `settings.map_extras` / `settings.video_extras` 字符串**，
   而界面编辑的是 `ListModel` 行 —— 同一个东西两套事实来源，任何一处残留都会在下次
   拖入时被「拼回来」。
2. **残留的空行**：点过「添加」还没选文件的行对 settings 是不可见的
   （`encodedExtras()` 跳过空项），于是同步检查认为两边一致、空行留下；下一次拖入
   是**追加**而不是填空行 ⇒ 行数比文件数多。

改法：**拖入也走模型**，并先填掉结尾的待填空行（多余的删掉）：

```qml
function addDroppedPath(p) {                    // TrackList / VideoList 各一份
    var lastFilled = -1                          // 最后一个非空行
    for (var i = count - 1; i >= 0; --i) if (path(i).length) { lastFilled = i; break }
    var hole = lastFilled + 1
    if (hole < count) { setRow(hole, p); if (count - 1 > hole) remove(hole + 1, count - 1 - hole) }
    else appendRow(p)
    writeBack()                                  // settings 只由模型推导
}
```

`handleDrop` 的视频/图片分支现在只调 `videoRows.addDroppedPath(p)` /
`trackRows.addDroppedPath(p)`；三个列表的 ✕ / 添加 / 「按段」也都收敛到
`removeRow()` / `appendRow()` / `setRowTime()` 单一入口，增删行后立即 `writeBackNow()`。

### Qt 断言框（Qt6Cored.dll / qqmlengine.cpp:1441）不再卡住自动化

**现象**：每次跑 GUI（以及任何加载 `gui.qml` 的无头工具）都弹一个断言框，标题栏是
模块名 `Qt6Cored.dll`，正文 `ASSERT: "bit < 2 * props" in file ...\qqmlengine.cpp, line 1441`；
**必须有人点「忽略」进程才继续**，否则一直挂着（自动化直接超时）。

**定位**：断言在 `QQmlData::growBits()` 内，由 `setBit()` 装绑定位时调用，打一行日志后继续。
用 `qmlprobe` 的 `QMLPROBE_FILE` + `demo/qml_assert_bisect.ps1` 逐块二分确认：
**断言发生在 `load_file` 期间**（`gui.qml` 实例化阶段），与测试脚手架无关。
已排除：Controls 风格（Fusion / Default）、内联组件+Repeater、RowLayout/Layout 附加属性、
「同属性既绑定又 Behavior」、`default property alias`。

**修法**：Qt 在 Windows 上把断言交给 CRT 的 `_CrtDbgReport(_CRT_ASSERT, ...)`：

| 报告模式 | 弹框？ | `_CrtDbgReport` 返回 | 结果 |
|---|---|---|---|
| 默认（WNDW） | 弹 | 「忽略」→ 0 / 「中止」→ 1 | 必须人工点忽略才继续 |
| `_CRTDBG_MODE_FILE` | 不弹 | 1 | 进程直接 fail-fast（实测 0xC0000409） |
| `_CRTDBG_MODE_DEBUG` | 不弹 | 1 | 同上 |
| **`_CrtSetReportHook` 钩子返回 TRUE 且 `*returnValue = 0`** | **不弹** | **0** | **进程继续（= 自动一直点忽略）** |

于是 `app/src/asserts_silent.h` 装了这个钩子：断言原文打到 stderr，进程照常往下走。
`gui.rs` 与所有无头工具（`qmlprobe` / `guirender` / `encgui` / …）都调用它。

**验证**：`tracereel-gui.exe`（offscreen）跑 20 秒，stderr 有两行 ASSERT、
同时 `[apply] landed:` 正常打印 = QML 建树完成、没被框住；`qmlprobe` 退出码 0 + `PROBE OK`。

### 自动化交互回放：`qmlprobe` + `qml_assert_bisect.ps1`

界面交互类 bug（拖放、增删行、按段选时刻、防抖同步）单测覆盖不到，于是有了：

| 工具 | 作用 |
|---|---|
| `app/src/bin/qmlprobe.rs` | 加载**真正的 gui.qml**，用 meta-object 调它自己的顶层 JS 函数（`handleDrop` / `debugRemoveRow` / `debugSetRow` / `debugState` …），逐步打印行数与 settings 原始串；带断言自检，不符就退非零 |
| gui.qml 的 `debugRemoveRow` / `debugAppendRow` / `debugSetRow` / `debugState` / `listCounts` | 与按钮走**同一条函数**，所以回放的就是真实路径；`listCounts` 也用在日志里 |
| `demo/qml_assert_bisect.ps1` | 按大括号配平切出 `gui.qml` 的 82 个顶层块，删掉指定块后跑 probe，用于二分定位 QML 触发的 Qt 断言 |

跑法（无头、不弹框）：

```
$env:QT_QPA_PLATFORM='offscreen'; $env:QT_QUICK_CONTROLS_STYLE='Fusion'
$env:QML_IMPORT_PATH='C:\Qt\6.8.2\msvc2022_64\qml'
target\debug\qmlprobe.exe          # 自带断言：PROBE OK / PROBE FAILED
```

### 日志：把"操作了什么"记全

排查这类问题全靠日志，所以 `app/src/bin/gui.qml` 里补了：

| 位置 | 记录内容 |
|---|---|
| `doApply(prop, note)` | **每条设置改动一行**：控件 label + 新值（布尔→勾选/取消，下拉→选项文字，滑杆→带单位的值），没有 label 时退化成 `属性 = 值` |
| `Connections { target: item }` | `apply_summary`（后台加载落地后的摘要：地图张数/主图尺寸/视频参数/运动数据）与 `busy`（开始加载），导出结束（成功/中止/失败） |
| `loadProject` / `saveProject` | 打开、保存项目（含路径） |
| `acceptBrowsed` | 选择文件（属性名 + 路径） |
| 拖放 / `.qrt` 弹窗 | 拖入的每一类文件、检测到分段、读取并导入的 N 个分段（连内容一起打） |
| 各列表 | 增删行（视频 / 轨迹图片 / 分段），「按段」设置换图时间 |
| 日志窗口 | 新增 **复制全部**（1×1 透明 TextArea 走剪贴板）与行数显示；模型封顶最近 3000 行 |

用法：菜单「日志 → 打开日志」→ **复制全部** → 直接贴给我即可。



一场定向拆成两张图（第一面 / 第二面）时，轨迹页可以加多行地图，第 2 行起带
「换图时间」。这条时间轴由 `gui.rs::map_jobs` + `locate_map` 驱动，预览与导出共用。

**`换图时间` 留空 = 这张图暂不参与播放**，而不是退化成 0 秒：

原实现把空时间当 `0.0`，于是额外地图与主图同为 `0 ms` 起点，`locate_map`
取"最后一个起点 ≤ t"就把额外地图选成**全程生效**——用户忘填换图时间时，
第 2 张图从片头一直显示到片尾。现在：

* `MapSettings::extra_map_change_times: Vec<Option<f64>>`，`None` 一路保留到
  `extra_maps()`；`map_jobs` 对 `None` 记一条日志并跳过（图都不解码）。
* ini 里空项必须留占位（`extraMapChangeTimes=|600`），否则索引会与
  `extraMapFiles` 错位、把第 3 张图的时间套到第 2 张上。
* 顺带修掉：`map_change_times` 以前按 `t.to_string()` 回写（如 `120.5`），
  而 `parse_time_string` 把 `.` 当分隔符 ⇒ `120.5` 会被读成 120 分 5 秒。
  现在统一按 `h:mm:ss` / `m:ss` 回写。

**「按段」按钮**：每行换图时间右边多一个「按段」，弹出每段的**起点时刻**
（`SettingsProxy::split_starts()`，第 N 段起点 = 第 N-1 个分段的结束时刻，
相对时间模式按累加处理），点一下直接填进该行。段起点与换图时间都从出发时刻算起，
单位一致，所以可以直接填。

**增删行立即生效**：行的增删与「按段」选择走 `win.applyNow()`（跳过 350ms 防抖），
`apply_settings` 清空地图列表后立刻重画一次——否则后台重载落地前面板上还留着
上一张地图，看起来就是"删掉了却还在显示"。

回归验证（真导出，不是单测）：

```
.\build.ps1 run -p tracereel --bin encgui -- demo\_t3.orv --out demo\_t3.mp4 --max-frames 3
[apply] 额外地图未设置换图时间，本次不参与播放: ...\第二面.qrt     ← 只加载主图
```

设成 `extraMapChangeTimes=1` 再导 90 帧：t=0 帧与"未设时间"那版的地图面板
差异 ≈1.4%（纯 H.264 有损差异），t=2s 帧差异 70% —— **换图确实发生在 1 秒**。

### 待办

1. ~~对象图解析 / 拿全轨迹 / 分段~~ —— 已完成，但走的不是对象图路线：
   轨迹 1475 点全部拿到（索引 1808..3282 就是文件内的对象号，不是数组下标），
   分段从 `Lap` 拿到，「导入分段」按钮已真正写入 `settings.split_times`。
2. ~~坐标系~~ —— 已完成，见上文「Handle 的几何语义」；`FixedSplit` 取景已验证。
3. ~~轨迹时间戳缺失~~ —— 已完成（1474/1475 个 1 Hz 时间戳），
   `RoutePoint.time` 单位已修正为**秒**。
4. 仍缺：**第 138 个 Handle**（`value ≈ 498.257660704935`）没被字节扫描命中，
   于是它前面那一段（约 492..508 号航点）的矩阵用的是拉长后的配对。
   影响是局部几米的偏差；要修就得读该记录的另一种写法。
5. 仍缺：多 segment 工程只用了 segment 0（与原版 `QuickRouteReader` 一致）。

## 8. 临时脚本

| 脚本 | 作用 | 状态 |
|---|---|---|
| `demo/qrt_dump.ps1` | **权威交叉验证**：.NET 反序列化 → JSON（含矩阵、laps） | ✅ 可用 |
| `demo/qrt_verify.ps1` | 用 JSON 校验 Handle ↔ 矩阵 ↔ 航点索引的对应关系 | ✅ 可用 |
| `demo/qrt_pairs.ps1` | 校验「相邻两点反推矩阵」与文件里存的矩阵一致 | ✅ 可用 |
| `tracereel-core/examples/qrt_geo.rs` | 纯 Rust 配准自检（锚点误差 / 包围盒） | ✅ 可用 |
| `tracereel-core/examples/qrt_check.rs` | 打印读取器恢复的全部内容 | ✅ 可用 |
| `demo/qrt_split_check.orv` | 端到端回归夹具：`.qrt` 当底图 + 轨迹 + 10 个分段 | 需自备素材 |
| `demo/qrt_panel_stats.ps1` | 统计渲染图左侧地图面板的颜色数 / 路线像素 | ✅ 可用 |
| `demo/qrt_reader.py` 等早期 Python 脚本 | NRBF 探索期产物，**已过时** | 🗄️ |

端到端回归（headless 真渲染，不需要手动点界面）。夹具与素材**不在仓库里**
（真实赛事地图不入库）：请自备 `.qrt`，按 `demo/example.trr` 的写法建一个工程再跑：

```
.\build.ps1 run -p tracereel --bin guirender -- `
  demo/qrt_split_check.orv --at 2000 --out demo/qrt_split_check.png
powershell -NoProfile -ExecutionPolicy Bypass `
  -File demo/qrt_panel_stats.ps1 -Path demo/qrt_split_check.png
# colors= 14935  black= 333/57600  route=  106 px  bbox=(140,66)-(208,702)
```

左侧面板 14935 种颜色 = 地图真的画出来了（背景是纯黑，只有 333/57600 个采样点是黑的），
106 个 `#0064ff` 像素 = 跟随点/尾迹也在。把 `splitTimes` 清空再渲染一次作对照，
地图面板颜色数相同（15351）而 `route=0`——因为 `viewMode=0` 时跟随点落在面板外，
正是原版 `ViewMode::FixedSplit` 的行为。

读取器当前能力（`python demo/qrt_reader.py <file.qrt> [--map o.jpg] [--gpx o.gpx]`）：

```
文件: AM 男子短距 第一面.qrt (4434605 字节)
地图: 内嵌 JPEG，3000x4243，4278305 字节
轨迹: 1475 个点（索引 1808..3282）
       首点 113.0240430, 23.1757680   末点 113.0226600, 23.1764200
       首个索引非 0：前 1808 个点未以该记录格式存储
```

**待办**：索引 0..1807 的点用的另一种编码（疑似 delta/紧凑），要拿全轨迹得解析对象图。

## 10. 本地已抓的源码（`.qrtsrc/`）

`Document.cs`、`DocumentSettings.cs`、`Map.cs`、`Route.cs`（含 `RouteSegment`/`Waypoint`）、
`Handle.cs`、`HandleCollection.cs`、`Session.cs`、`SessionCollection.cs`、
`SessionSettings.cs`、`GeneralMatrix.cs`、`ParameterizedLocation.cs`、`MarkerDrawers.cs`、
`RouteLineSettings.cs`、`Lap.cs`、`LapCollection.cs`、`ColorRange.cs`、
`QuickRouteJpegExtensionData.cs`、`MapReadingState.cs`

## 11. 临时脚本清单
