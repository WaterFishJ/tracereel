# MSIX 打包（微软商店）

## GitHub Actions 自动更新准备

流程：推送正式版本标签（如 `v0.3.1`）→ 构建与验证便携包 → 生成 MSIX 与源码包 →
发布 GitHub Release →（启用后）上传该 Release 中的同一份 MSIX 并提交微软商店认证。
提交成功不等于商店已上线，最终状态以 Partner Center 为准。

### Partner Center / Microsoft Entra

1. TraceReel 必须已在商店正式上线，首次提交（包括年龄分级、截图、隐私政策、许可条款及
   `runFullTrust` 说明）先在 Partner Center 手动完成。
2. Partner Center → 账户设置 → 租户：关联 Microsoft Entra 租户。
3. 在该租户注册一个专用于 CI 的应用。在 Partner Center → 账户设置 → 用户管理 →
   Microsoft Entra 应用程序中添加该应用，授予 **Manager** 角色。
4. 记录租户 ID、应用（客户端）ID，并在应用的“证书和密码”中创建客户端密码。
   保存密码的 **Value**（不是 Secret ID），并记录过期时间。
5. 从 Partner Center 账户标识中取得 Seller ID；从 TraceReel 的产品标识取得 Store Product ID
   （如 `9…`）。Product ID 与 MSIX 包名 `WaterFishJ.TraceReel`、Publisher GUID 是不同字段。
6. 核对 `AppxManifest.xml` 中的包名、Publisher、显示名与商店一致；本次版本将是 `0.3.1.0`，
   必须高于该应用商店内已有的对应包版本。

官方说明：[GitHub Actions 前提条件](https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/github-actions)、
[MSIX CLI 命令](https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/commands)。

### GitHub 仓库设置

在仓库 **Settings → Secrets and variables → Actions** 设置以下 Repository secrets：

| Secret 名 | 值 |
|---|---|
| `PARTNER_CENTER_TENANT_ID` | Entra 租户 ID |
| `PARTNER_CENTER_CLIENT_ID` | 专用 CI 应用的 Application (client) ID |
| `PARTNER_CENTER_CLIENT_SECRET` | 客户端密码的 Value |
| `PARTNER_CENTER_SELLER_ID` | Partner Center Seller ID |

在同一页面的 **Variables** 标签设置 Repository variables：

| Variable 名 | 值 |
|---|---|
| `PARTNER_CENTER_PRODUCT_ID` | TraceReel 的 Store Product ID |
| `MICROSOFT_STORE_ENABLED` | 准备完成后设为 `true`，后续正式标签发布将自动提交商店审核 |

密码直接填入 GitHub Secrets，不要发到聊天或提交到仓库。仅生成并上传商店 MSIX 不需要自购签名证书。

### 用 0.3.1 测试

即使 GitHub Release 已经发布，也无需重新打标签或重新构建：

1. 配好上述 secrets 与 Product ID，确认商店没有正在审核或需要保留的草稿提交。
2. GitHub → Actions → **Microsoft Store update** → **Run workflow**，使用 `main`，
   `release_tag` 填 `v0.3.1`。
3. `submit` 不勾选：下载 Release 的 MSIX、验证 SHA256、上传并保留草稿。
   **这会真实创建或更新商店草稿，并非 dry-run。** 不要同时在 Partner Center 编辑 API 创建的草稿。
4. 要完整测试自动提审，首次运行就勾选 `submit`。如果先上传了草稿，可用已配置凭据的 CLI 执行
   `msstore submission publish <Product ID>` 提交该草稿；不要盲目重跑包上传。
5. 在 Actions 日志及 Partner Center 确认认证结果；Actions 成功仅表示上传/提审命令完成。
   如命令超时或失败，先检查 Partner Center，避免重复提交。
6. 测试完成后设置 `MICROSOFT_STORE_ENABLED=true`，后续正式版本自动提审。预发布标签和普通 main
   提交不会自动提交商店；商店步骤失败不撤回已发布的 GitHub Release。

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
