# presets（预设目录）

本目录**不随仓库发布任何预设文件**。`.gyroflow` 预设是个人化配置：它绑定具体相机、
镜头标定和你的稳定化偏好，随仓库分发对别人没有意义（也可能夹带本机路径）。

## 怎么获得预设

1. 打开 [Gyroflow](https://github.com/gyroflow/gyroflow)；
2. 选择你的相机对应的镜头配置文件，按需要调整稳定化/变焦/地平线等参数；
3. 菜单里「保存预设」，把得到的 `.gyroflow` 文件放进本目录。

放进本目录后，TraceReel 的防抖设置页「预设」下拉里会直接列出来（会自动扫描
工作目录下的 `presets/`）。

## 说明

- `package.ps1` 打包时会把本目录里的 `*.gyroflow` 一并放进便携包的 `presets/`，
  并把其中的 `output_folder` 清空（避免把本机输出路径带给别人）。本目录为空时
  便携包里就是一个空的 `presets/`。
- 相机镜头标定数据库不在这里：它由 gyroflow 子模块的
  `resources/camera_presets/profiles.cbor.gz` 提供，构建时会自动复制到可执行文件旁。
