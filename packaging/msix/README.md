# MSIX 打包（微软商店）

从现有便携包目录生成商店提交用的 `.msix`。**不需要自购代码签名证书**：商店在认证通过后会用微软证书重签；本地安装测试才需要自签（见下）。

## 生成商店包

```powershell
# 先出便携包（或用已有的最新 dist 目录）
.\package.ps1
# 再从便携包生成 MSIX（版本号自动取自 Cargo）
.\packaging\msix\build-msix.ps1
```

产物：`dist\TraceReel-<版本>-windows-x64.msix` 与同名 `.sha256`。

可选参数：

| 参数 | 用途 |
|---|---|
| `-Version 0.3.0` | 指定版本（默认读 `cargo metadata`） |
| `-PackageDir <目录>` | 指定便携包目录（默认取 `dist` 下最新的 `TraceReel-*-windows-x64-*`） |
| `-SignThumbprint <证书指纹>` | 用本机证书签名，**仅用于本地安装测试** |
| `-KeepStaging` | 保留 `packaging/msix/staging` 便于排查 |

脚本固定做的事：

1. 复制便携包内容到 `staging`；
2. 把两个中文文件名改成 ASCII（`使用说明.txt` → `README.zh-CN.txt`、`licenses/第三方组件.txt` → `licenses/THIRD-PARTY-COMPONENTS.txt`）——MSIX 的包内路径必须是 ASCII；
3. 删除 `qmltooling`（QML 调试插件，商店包不需要）；
4. 从 `assets/branding/tracereel-icon.png` 生成 `Assets` 图标（含 100%/200% 缩放）；
5. 用 `makepri` 生成 `resources.pri`，用 `makeappx` 打包；
6. 输出 SHA256。

## 清单与身份

`AppxManifest.xml` 里的三项必须与 Partner Center 的“应用标识”完全一致：

| 字段 | 当前值 |
|---|---|
| 包名 | `WaterFishJ.TraceReel` |
| 发布者 | `CN=619319A3-9814-4581-9290-5F566F256D2B` |
| 发布者显示名 | `WaterFishJ` |

清单还注册了 `.trr` 工程文件关联（双击工程文件会用 TraceReel 打开），只声明 `runFullTrust` 一项受限能力，最低系统 Windows 10 1809。

## 本地安装测试（可选）

```powershell
# 生成自签证书（仅本机测试；证书主题必须与清单发布者一致）
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=TraceReelLocalTest' -CertStoreLocation Cert:\CurrentUser\My
$pw = ConvertTo-SecureString -String 'test' -Force -AsPlainText
Export-PfxCertificate -Cert $cert -FilePath "$env:TEMP\test.pfx" -Password $pw | Out-Null
# 信任证书（需要管理员）后安装
```

商店提交本身不需要这一步，直接上传未签名的 `.msix` 即可。

## 提交

1. Partner Center → 应用 → 创建提交；
2. 定价与可用性、属性（隐私政策 URL）、年龄分级、包（上传本目录产物）、商店一览（含“其他许可条款”）、提交选项（`runFullTrust` 用途说明 + 认证说明）；
3. 提交认证，通常 1–3 个工作日；通过后微软重签并发布。
