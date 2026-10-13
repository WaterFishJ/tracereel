# 对齐原版 Gyroflow：稳定化面板 / 变焦语义 / 界面风格

> 记录于 2026-09。三件事：①把隐藏项收进「高级选项」并补齐参数、名称对齐官方中文；
> ②修「变焦缩放不生效」；③界面配色/控件/动画对齐原版。
> 相关代码：`app/src/gyro.rs`、`app/src/settings.rs`、`app/src/settings_proxy.rs`、`app/src/theme.rs`（新）、
> `app/src/bin/gui.rs`、`app/src/bin/gui.qml`。概览见 `docs/防抖问题与修复总结.md` §13。

---

## 1. 变焦为什么以前完全不生效（已修）

**根因在 core，不在我们**：`gyroflow-src/src/core/zooming/mod.rs:62-75` 只按 **`adaptive_zoom_window`** 决定模式：

| `adaptive_zoom_window` | 行为 |
|---|---|
| `< -0.9` | 静态缩放（取整段最小 fov，恒定裁切） |
| `> 0.0001` | 动态缩放（窗口 = 秒） |
| 其余（= 0） | **关闭缩放** |

`adaptive_zoom_method` 只是**动态缩放的算法**（`impl From<i32> for ZoomMethod`：0=GaussianFilter、1=EnvelopeFollower），
**不决定开关**。我们之前把「缩放方式」写进 `set_zooming_method()`、把「缩放速度」写进 `set_adaptive_zoom()`，
而速度默认 0 → 核心恒走「关闭」，三种模式都不缩放；静态缩放需要的 `-1` 也没有任何路径能产生。

**修法**（`app/src/gyro.rs`）：

```rust
fn effective_zoom_window(g: &GyroSettings) -> f64 {
    match g.zoom_mode {
        2 => -1.0,                       // 静态缩放
        1 => if g.adaptive_zoom_window > 0.0001 { g.adaptive_zoom_window } else { 4.0 }, // 动态：缺省 4 秒
        _ => 0.0,                        // 无缩放
    }
}
fn zoom_algorithm(mode: i32) -> i32 { if mode == 1 { 1 } else { 0 } } // 只送 0/1，避免 core 报 Invalid zooming method
```
`apply_gyro_settings()` 与 `apply_gyro_field()` 都改用这两个函数；
`GyroSettings::apply_preset_json()` 里把「method=1 但 window≤0」的旧预设归一成 4.0，
同理静态模式的存值也归到 4.0（window 的语义只保留「动态缩放的速度」）。

**验证（`gyrotest` + 临时打印 `params.fovs` 的 min/max，1.0 = 未缩放；DJI 85237 帧）**：

| 模式 | core 里的 window / method | fovs min–max | 结论 |
|---|---|---|---|
| 无缩放 | 0 / 0 | 1.0000 – 1.0000 | 关闭 ✓ |
| 动态缩放 | 4 / 1 | 0.4320 – 0.8490 | 随片段变化的动态裁切 ✓ |
| 静态缩放 | -1 / 0 | 0.8235 – 0.8235 | 恒定的静态裁切 ✓ |

---

## 2. 稳定化面板：分组 / 中文名 / 补齐参数

**依据**：`gyroflow-src/src/ui/menu/Stabilization.qml`（主区 374–721 行、`AdvancedSection` 722–980 行）＋
官方中文 `gyroflow-src/resources/translations/zh_CN.ts`。

### 2.1 分组

| 分区 | 控件（顺序照原版） |
|---|---|
| **主区** | 平滑方式 + 参数（非 advanced 的）、锁定地平线、锁定量、横滚角度校正、锁定俯仰角、俯仰角度校正、使用重力矢量、积分方法、自动锁定（转弯阈值 / 转弯平滑 / 转弯倍率 / 限制倾斜加速度 / 倾斜加速度上限）、缩放限额、缩放速度、镜头校正、稳定焦距（+ 焦距平滑度）、只读「最大旋转 / 最大缩放」 |
| **高级选项**（折叠，700ms 动画） | 视场角、卷帘快门校正（+ 帧读出时间 / 方向）、视频速度（+ 链接平滑 / 链接缩放速度 / 链接缩放限额）、缩放方式、缩放中心偏移 X/Y、附加 3D 旋转（俯仰/偏航/横滚）、附加 3D 平移 X/Y/Z、缩放限额迭代次数、陀螺偏移（毫秒 / 帧） |

平滑参数里 `get_parameters_json()` 标了 `advanced: true` 的（`per_axis`、`trim_range_only`、`max_smoothness`、
`alpha_0_1s`）自动进入高级选项；`Default` 方式的四个 `smoothness` 按原版**以百分比显示**（×100、单位 %、精度 1，
写回时再 ÷100）。

### 2.2 中文名对照（官方 zh_CN 为准；★=官方未译，按语义补；⚠️=与既有偏好冲突，已按对齐取舍）

| 原版英文 | 采用名称 |
|---|---|
| Lock horizon / Lock amount | 锁定地平线 / 锁定量 |
| Roll angle correction / Lock pitch angle / Pitch angle correction | 横滚角度校正 / 锁定俯仰角 / 俯仰角度校正 |
| Use gravity vectors / Integration method | 使用重力矢量 / 积分方法（互补滤波 / VQF / 陀螺+加速度计 / Mahony / Madgwick） |
| Automatic lock / Turn threshold / Turn smoothness / Turn multiplier / Limit tilt acceleration / Tilt acceleration limit | 自动锁定 / 转弯阈值 / 转弯平滑 / 转弯倍率 / 限制倾斜加速度 / 倾斜加速度上限 ★ |
| Zoom limit / Zooming speed / Zooming method / Zooming center offset / Zoom limit iterations | 缩放限额 / 缩放速度 / 缩放方式（无缩放·动态缩放·静态缩放）/ 缩放中心偏移 / 缩放限额迭代次数 |
| Lens correction / Stabilize focal length / Smoothness | 镜头校正 / 稳定焦距 / 焦距平滑度 |
| FOV ⚠️ | **视场角**（官方译法；早前自拟的「视野」已弃用） |
| Rolling shutter correction / Frame readout time | 卷帘快门校正 / 帧读出时间 |
| Video speed / Link with smoothing·zooming speed·zooming limit | 视频速度 / 链接平滑·链接缩放速度·链接缩放限额 |
| Additional 3D rotation / translation | 附加 3D 旋转 / 附加 3D 平移（官方 zh_CN 误写“转换”，未采用） |
| Advanced | 高级选项 |
| No smoothing / Default / Plain 3D / Fixed camera | 无平滑 / 默认 / 纯 3D / 固定摄像头 |
| Pitch / Yaw / Roll | 俯仰 / 偏航 / 横滚（官方保留英文，按本项目偏好译出） |

### 2.3 新增的设置字段（`.orv [gyro]` 键名同列）

| 字段 | 默认 | 说明 / 键名 |
|---|---|---|
| `horizon_lock_enabled` | true | 锁定地平线总开关（`horizonLockEnabled`）；关了送 0 |
| `horizon_automatic_lock` | false | 自动锁定（`horizonAutomaticLock`）：勾上锁定量按 100 送 |
| `horizon_lock_pitch_enabled` / `horizon_lock_pitch` | false / 0 | 锁定俯仰角 + 俯仰角度校正（`horizonLockPitchEnabled` / `horizonLockPitch`） |
| `horizon_lock_integration_method` | 0 | 积分方法 0–4（`horizonLockIntegrationMethod`）→ `set_horizon_lock_integration_method` |
| `horizon_turn_threshold` / `horizon_turn_smoothing_ms` / `horizon_turn_multiplier` | 5 / 500 / 1 | 转弯阈值(°/s) / 转弯平滑(ms) / 转弯倍率（`horizonTurn*`） |
| `horizon_limit_tilt_accel` / `horizon_tilt_accel_limit` | false / 3 | 限制倾斜加速度 + 上限(°/s²)；不勾时给 core 送 ∞（`horizonLimitTiltAccel` / `horizonTiltAccelLimit`） |
| `frame_readout_enabled` | true | 卷帘快门校正开关（`frameReadoutEnabled`）；关掉时送 0 毫秒 |
| `focal_length_smoothing_enabled` / `_strength` | false / 30 | 稳定焦距（`focalLengthSmoothing*`）；只有逐帧焦距的视频才显示 |

* `apply_horizon_lock()` 一次把 9 个参数送进 `horizon_lock.set_horizon()`，取值规则照抄原版 `updateHorizonLock()`。
* 只读状态由 `TraceReelItem.gyro_stats`（`get_smoothing_max_angles()` + `params.fovs` 最小值 → 最大缩放）提供；
  `TraceReelItem.gyro_has_focal_length` 决定「稳定焦距」是否显示（对应原版 `has_per_frame_focal_length()`）。
* `.gyroflow` 往返：新字段按原版键名读（`horizon_lock_pitch_enabled`、`horizon_lock_pitch`、
  `horizon_lock_integration_method`、`focal_length_smoothing_*`；转弯类参数原版不写进预设，故只存 `.orv`）。

---

## 3. 界面风格对齐（配色 / 控件 / 动画）

### 3.1 主题令牌：`Theme` QML 单例

新增 `app/src/theme.rs`，用 `qml_register_singleton_type` 在 `app/src/bin/gui.rs` 注册（必须在创建 QmlEngine 之前）。
做成单例而不是上下文属性，是为了 `gui.qml` 与 `HudEditor.qml`/`HudCanvas.qml` 都能直接 `Theme.xxx`，
而且带 NOTIFY 可以运行时切换。取值与 `gyroflow-src/src/ui/ui_tools.rs:94-125` 逐字一致：

| 令牌 | 浅色 | 深色 |
|---|---|---|
| `background` / `background2` | #f9f9f9 / #f3f3f3 | #1e1e1e / #191919 |
| `button` | #fbfbfb | #282828 |
| `text` / `muted` | #111111 / #6b6b6b | #ffffff / #a8a8a8 |
| `accent` | #116cad | #76baed |
| `video_border` | #d5d5d5 | #2b2b2b |
| `text_on_accent` | #ffffff | #000000 |
| `hr` / `popup_border` | #e5e5e5 / #d5d5d5 | #2e2e2e / #0f0f0f |
| `slider_handle` / `slider_background` | #c2c2c2 / #cdcdcd | #454545 / #949494 |
| `highlight` | #10000000 | #10ffffff |
| `font_family` / `dpi_scale` | Arial / 屏幕 DPI÷96 | 同 |

切换入口：菜单「外观 → 浅色 / 深色 / 跟随系统」（`settings.theme` 存 `.orv [window] theme`，默认浅色；
「跟随系统」用 `Qt.styleHints.colorScheme` 解析）。`refreshFields()` 里会 `applyTheme()`，
所以切换即时生效、不需要重新应用工程（不会触发重探测/重算）。

### 3.2 关键做法：窗口调色板 + 组件覆写

* **窗口 palette 绑定**到上述令牌 → Fusion 默认控件（文本框、下拉框、菜单、弹层、滚动条、工具提示、
  进度条、数字框…）整体跟着主题走，改动量最小；
* **自定义组件**再逐项覆写外观（`TButton` / `TSlider` / `Check` 指示器 / `GroupBox` / `TabBar`），
  数值与动画都取自原版：

| 组件 | 原版参考 | 对齐要点 |
|---|---|---|
| 分区标题 | 面板写法 | 加粗 14px 标题 + 1px `hr` 分隔线（去掉原来的灰底标题条） |
| 按钮 `TButton` | `components/Button.qml` | 圆角 6·dpi、浅色 1px `popup_border` 描边、hover 提亮 1.2、按下 `scale 0.97`（100ms OutExpo）、禁用 0.6 |
| 滑杆 `TSlider` | `components/Slider.qml` | 轨道 4·dpi 圆角 4（`slider_background` + 已选段 `accent`）、圆形手柄（`slider_handle` + 内圈 `accent`），内圈 hover/press `0.9 / 1.1`（200ms OutExpo） |
| 勾选框 | `components/CheckBox.qml` | 20·dpi 圆角 5、选中 `accent` 填充（颜色 300ms）、对勾 200ms InOutCubic 生长 |
| 折叠区 | `components/AdvancedSection.qml` | 居中强调色按钮 + 高度/透明度 **700ms OutExpo** |
| Tab 栏 | `components/Tabs.qml` | 背景 `background2` + 底部 1px 线、选中项 `accent` 文字 + 2px 指示条（250ms 滑动） |
| 忙碌遮罩 | `components/LoaderOverlay.qml` | 背景 `background2`、保留真实百分比进度（淡入淡出沿用现有实现） |
| 预览区 | 视频边框 | `video_border` 1px 边框；底色深色主题下更暗 |

### 3.3 现在还没做的（后续可选）

* 自绘无边框标题栏（原版是 frameless + 自定义关闭按钮）：窗口拖拽/缩放/多屏 DPI 风险大，未做；
* `HudEditor.qml` / `HudCanvas.qml` / `HudColorDialog.qml` 内部仍用自己的颜色常量（它们只在 HUD 页显示），
  需要的话再统一到 `Theme`；
* 原版的移动端布局分支、关键帧面板、时间轴皮肤与本项目功能无关，不移植。

### 3.4 验证

`qmllint` 无语法错误；实际跑一遍：启动无 QML 报错（`RangeError/TypeError/Binding loop: 0`）、窗口正常响应；
临时钩子切到深色主题后无报错（验证完已移除）。

**怎么无人值守地验证界面改动**：`.\verify_gui.ps1 -Orv demo\example.trr -Seconds 15`（`gui.qml` 是运行时从磁盘读的，改 QML 不用重编译）。它按 `run.ps1` 备好 Qt/ffmpeg 环境，自动点掉 debug 版 Qt 断言对话框（「忽略(&I)」，否则进程卡在对话框上），并把输出按关键字过滤。关键点：

- GUI 子系统默认把 QML 的 `console.log` 发给调试器 —— 脚本里设了 `QT_FORCE_STDERR_LOGGING=1`（+ `QT_LOGGING_TO_CONSOLE` / `QT_ASSUME_STDERR_HAS_CONSOLE`）让它落到 stderr；
- 于是「界面内部状态」可以直接验证：QML 里临时加 `Timer { onTriggered: console.log("[uiverify] " + ...) }`，跑完看日志，**验证完务必删掉钩子**（本轮就是靠它确认了分段跳转的落点与分辨率下拉的取值/标签顺序）。

---

## 4. 其它 UI 调整（本轮一并做掉）

| 需求 | 做法 | 验证 |
|---|---|---|
| 「分段」页每行加跳转按钮 | 行尾加「跳转」按钮 → `win.jumpToSplit(rowIndex)`；时间换算走 **Rust** `Settings::split_video_ms(idx)`（= `boundary_ms(idx, add_video_offset=true)`，与导出用的边界同一套：分段点绝对时间 + `录像-出发时间`）。**语义：行里填的时间是「该分段的结束」= 下一个分段点，所以本段起点 = 上一个分段点：行 k（0 基）→ 分段点 k**；分段点 0 = 出发时刻，故第 1 段跳到视频开头 | `split(0..3)=0,3000,7000,12000`；`jumpToSplit(1)`→`t_ms=3000`（= 第 2 段起点，旧实现是 7000 = 该段终点）✓ |
| 预览进度条拖动时显示时间 | `TSlider` 手柄上挂 `ToolTip`（`visible: seek.pressed`，显示 `fmtTime(seek.value)`，原版 Slider 拖动气泡同款）；同时时间标签在拖动期间显示拖动位置而非播放位置 | 无 QML 报错 ✓ |
| 导出页「分段导出」改下拉 | 新代理属性 `encode_seg_mode` ∈ `none` / `all` / `custom`，**UI 唯一真源**：`apply_settings` 由旧的 `segEnabled/allSeg` 推出模式，`collect_settings` 由模式反推两个开关（**.orv 格式不变、无新键**）；选「自定义」时出现「分段范围」输入框，格式 `xx-xx`（正则 `^[0-9]{0,3}-[0-9]{0,3}$`），`setSegRange()` 解析后写回 start/end | 旧工程 → `seg_mode=none`；`setSegRange("1-3")` → `custom, seg=1-3` ✓ |
| 分辨率下拉改显式尺寸 + 4K + 4:3 | 标签格式 = `尺寸 (档位名 比例)`，先 16:9（4K → 1080p → 720p）再 4:3（同序）：`3840x2160 (4K 16:9)` / `1920x1080 (1080p 16:9)` / `1280x720 (720p 16:9)` / `2880x2160 (4K 4:3)` / `1920x1440 (1440p 4:3)` / `1440x1080 (1080p 4:3)`；默认仍是 `1920x1080`。`encode.rs::encode_output_size` 本来就支持 `WxH`；`Settings::default()` 与 `load_orv()` 把旧档位名（`1080p/fhd`、`720p/hd`、`4k/uhd/2160p`）归一成同尺寸显式写法，否则下拉选不中、显示与导出尺寸会不一致（`source` 后来变回下拉里可选的一项，见第 5 节） | 运行钩子打印：`res=1920x1080`（旧工程归一后的选中项）、`resValues/resLabels` 与上表一致 ✓ |
| 视频页「备注式」小字不显示 | 去掉「稳定化」组的说明段；镜头库状态那行小字已移除（只在缺失时才有意义，当前实现直接不显示）；功能性状态（防抖状态、最大旋转/最大缩放只读值）保留 | 运行时无报错 ✓ |

> 说明：分辨率影响预览宽高比：`item.outputAspect` 用 `encode_resolution` 按 `x` 拆分，显式尺寸后预览比例精确（以前 `1080p` 选不中会退回 16:9）。第 5 节起：`"source"`（与原始视频一致）改走**源视频比例**，不再退回 16:9。

---

## 5. 分辨率新增「与原始视频一致」（保持原分辨率）

需求：导出页「分辨率」要能**不缩放**，直接沿用源视频尺寸。

| 需求 | 做法 | 验证 |
|---|---|---|
| 下拉加一项「与原始视频一致」 | 复用后端早就有的 token `source`（`app/src/encode.rs:199-227` `encode_output_size` 里 `"source" => (src_w, src_h)`，只做偶数对齐）。`app/src/bin/gui.qml` 分辨率 ComboV 变成第一项 `values: ["source", …]` / `labels: ["与原始视频一致", …]`（措辞与「帧率」那一项一致）；默认仍是 `1920x1080` | `app/tests/test_export_resolution.ps1`（无窗口，见下）✓ |
| 存盘 → 重新打开要保住这一项 | `app/src/settings.rs` 的 `load_orv()` 旧档位名归一里**去掉 `"source"`**（原来它跟 `1080p/fhd` 一起被改成 `1920x1080`，会让用户选好的「与原始视频一致」在重开工程时变回 1080p）。老工程的 `1080p/720p/4k` 仍照旧归一；本机全部 `.orv/.trr` 里本来也没有 `source`，所以没有存量工程被改尺寸 | 单测 `settings::tests::load_orv_keeps_source_resolution_and_normalizes_legacy_presets` ✓ |
| 预览画布与导出同比例 | 新增只读属性 `TraceReelItem.source_size`（"宽x高"：`app/src/gui.rs` 的 `apply_prepared` 里按探测到的源尺寸赋值并 `source_size_changed()`，`reload`/`apply_settings` 清零）。`gui.qml` 的 `item.outputAspect` 改走新函数 `win.aspectOf(token)`：token 不是显式尺寸且为 `source` 时，改用 `item.source_size` 的比例 | 探针实测 320x240 → 1.3333，切 1280x720 → 1.7778，切回 → 1.3333 ✓ |
| 界面/队列文案不许露英文 | `app/src/export_queue.rs` 的 `configuration()` 把 `source` 显示成「与原始视频一致」（与「帧率」同款特判）；导出日志仍打裸 token（日志不是界面，既有日志本就写 `帧率=source`） | 单测 `export_queue::tests::configuration_shows_resolution_in_chinese` ✓ |

**回归怎么跑**（自带素材，不需要真 4K 素材）：

```powershell
.\app\tests\test_export_resolution.ps1            # 会自动 build qmlprobe
```

它做三件事：ffmpeg 造一段 320x240（4:3）的 2 秒小视频 → 写一个最小工程（`resolution=source`）→
`QT_QPA_PLATFORM=offscreen` 跑 `qmlprobe` 驱动真实的 `gui.qml`（工程路径经新的 `QMLPROBE_ORV`
环境变量传进去，`qmlprobe` 把它转成 QML 的 `orvPath`，`gui.qml` 启动即加载）。通过时 stdout 有
`EXPORT_RESOLUTION_PASS`，失败是 `EXPORT_RESOLUTION_FAIL`（原因同时打在 `[loop final]` 行里）。

> 踩坑记录：代理属性没有 `NOTIFY`，`outputAspect` 靠 `refreshTick` 失效，而 ComboV 写值走 350ms
> 防抖 —— 探针必须**轮询等**比例变化，读完 `settings` 就立刻读比例会假失败（第一版就是这么红的）。

---

## 6. 导出解码尺寸不再固定 720（视频层真正按输出尺寸解码）

**问题**：导出一直把源视频按「保比例、限高 720」解码（`app/src/gui.rs` 的 `start_encode` 里
`processing_size(info.width, info.height, 720)`，导出队列同样一处），再由 `draw_scene` 把这张
720 级帧缩放进输出画布的视频面板。于是 1080p/4K 输出里的**视频层**都是从 720p 放大出来的
（地图、路线、HUD 不受影响：它们本来就按输出画布绘制，地图位图上限 4096px）。以
`demo\20261011孝德湖`（源 2688x2016、输出 1920x1080、地图占 25%）为例：解码帧只有 960x720，
面板 1440x1080 → 放大 1.5×；选 4K 输出 → 放大 3×。

**做法**：新增 `app/src/encode.rs::export_decode_size(settings, src_w, src_h)` —— 按输出画布里面板
实际需要的像素算解码尺寸，上限=源尺寸（绝不放大）：

- 面板 = 画布宽 ×(1−地图占比) × 画布高（与 `draw_scene` 同一套算式；地图判据抽成
  `app/src/gui.rs::map_configured`，导出内容不含地图时占比按 0）；
- 缩放系数 = `min(面板宽/源宽, 面板高/源高) × 视频缩放`，`clamp(0.0, 1.0)` → 视频既不会被放大，
  也不多解码用不上的像素；
- `app/src/gui.rs` 的 `start_encode`、`app/src/export_queue.rs` 的队列任务、`app/src/bin/encgui.rs`
  三处都改用它。

**必须一并解决的坑**：gyroflow 的稳定化器与帧尺寸强绑定（`gyroflow-src/src/core/stabilization/mod.rs`
的 `process_pixels` 会 `SizeMismatch` / `InvalidStride`），而导出原先直接复用**预览那套**按 720
初始化的 `Arc<Stabilizer>`。所以 `app/src/encode.rs` 新增 `reload_export_stabs_if_needed()`：尺寸
不一致时在**编码线程**上按导出尺寸重新 `load_stabilizer`（含多分段的 `apply_seam_warmup_smoothing`
接缝平滑），加载失败就退回旧稳定化器的尺寸（宁可分辨率低，也不能让帧与防抖对不上）。逐段导出
（`encode_export_segments`）只在批次开始时加载一次，避免每段都重读运动数据；加载期间弹窗提示
「正在按导出尺寸 … 重新准备防抖」（已有提示时（如硬件编码回退）不覆盖）。实测这段 23 分钟的
4K 素材重读一次约 **46 秒**（`demo\20261011孝德湖`：`[encode] 按导出尺寸 1440x1080 重新加载防抖：分段 1（45.7s）`）。
日志里会写一行 `解码尺寸 WxH（输出 …，源 …，防抖=…）`。

| 验证 | 结果 |
|---|---|
| 单测 `encode::tests::export_decode_size_follows_the_output_panel` | 2688x2016 源：1080p 输出 → 1440x1080（旧：1280x720 再放大）；4K 输出 → 2688x2016（顶到源）；720p 输出 → 960x720（与旧行为一致）；源比输出小时不放大；含/不含地图两条分支 ✓ |
| 端到端（`encgui`，3840x2160 源 → 1920x1080，无地图） | 日志 `解码尺寸 1920x1080（输出 1920x1080，源 3840x2160，防抖=关）`，输出 1920x1080 / 3 帧 ✓（旧行为会写 1280x720） |
| 端到端（`encgui`，真实 4K 工程 `demo\20261011孝德湖`，防抖开） | `[encgui] loading stabilizer at 1440x1080` → 正常出片（59 秒，含读运动数据）✓ |
| 端到端（**真实 gui.qml**，`app/tests/test_export_decode_size_gyro.ps1`，防抖开） | 预览先按 720 加载 → 导出时走尺寸重载分支：`[encode] 按导出尺寸 1440x1080 重新加载防抖：分段 1（45.7s）` → `解码尺寸 1440x1080（输出 1920x1080，源 2688x2016，防抖=开）` → 连续编码 8000+ 帧无 `SizeMismatch`（探针编够 60 帧主动中止）✓ |
| 全量单测 | `.\build.ps1 test -p tracereel --lib` → 88 passed ✓ |

