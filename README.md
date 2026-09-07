# 桌面笔记（Desktop Notes）

一款面向 Windows 的本地优先日历笔记应用。正常记笔记时无需联网，主要数据保存在当前 Windows 设备上。

当前公开预览版为 **v0.1.0-beta.1**，属于 Beta 测试版本，不是稳定版 1.0。

## 主要功能

- 按日历日期创建和整理笔记
- 删除不再需要的笔记
- 富文本编辑
- 可左右拖动日历与笔记区域之间的分界线
- 兼容粘贴包含行内代码样式的 ChatGPT 富文本
- 标签、置顶和最近笔记
- 本地全文搜索
- 加密图片
- 全局快捷键快速笔记
- 自动加密备份
- 收起、标准和展开三种窗口状态
- 使用 SQLCipher 加密的本地数据库
- 中文界面和中文日期显示

## 截图

后续只会添加使用虚构笔记内容制作的公开安全截图。请勿提交含有个人笔记、账号信息或其他私人数据的截图。

## 支持平台

- 主要平台：Windows 11 x64
- 已测试兼容：Windows 10 22H2 x64

其他 Windows 版本尚未完成充分验证。

### 运行依赖

当前便携版可执行文件依赖 Microsoft Edge WebView2 Runtime 和 Microsoft Visual C++ 2015-2022 Redistributable（x64）。这些组件通常已存在于受支持的 Windows 系统中，但不会打包进便携版可执行文件。

当前 Beta 可执行文件未签名，Windows 可能显示“未知发布者”提示。

## 已知验证缺口

下列项目尚未在对应物理硬件上完成资格验证，它们是测试缺口，不代表已确认的产品故障：

- 使用真实电池供电设备时的 Windows 节电模式
- 真实双显示器、混合 DPI 和负坐标布局
- 真实显示器断开与重新连接

## 隐私与安全边界

- 应用采用本地优先设计，正常使用笔记不需要网络连接。
- 笔记数据库使用 SQLCipher 加密。
- 图片资源和自动备份均经过加密。
- 数据库密钥材料由 Windows 数据保护 API（DPAPI）的 CurrentUser 范围保护。

这些措施用于保护其文档边界内的静态数据，但不会在已解锁的 Windows 会话内再增加一层应用登录验证。能够使用当前 Windows 用户会话并直接运行桌面笔记的人或进程，可能通过应用读取笔记。项目不宣称零知识、不可破解或能抵御所有本机入侵。

报告问题时，请勿上传真实笔记数据库、DataRoot、备份、密钥材料、剪贴板内容或个人截图。

## 开发环境

- Windows 11 x64，或用于兼容性开发的 Windows 10 22H2 x64
- Rust 1.98.0，安装 `x86_64-pc-windows-msvc`、`rustfmt` 和 `clippy`
- Node.js 20.12.2 与 npm；前端依赖由 `apps/desktop/package-lock.json` 锁定
- Microsoft C++ Build Tools，并安装“使用 C++ 的桌面开发”
- Microsoft Edge WebView2 Runtime
- 带有 `Locale::Maketext::Simple` 和 `IPC::Cmd` 模块的 Perl，用于构建仓库内锁定的 OpenSSL 源码

## 构建

安装锁定的前端依赖并构建前端：

```powershell
Set-Location apps/desktop
npm ci
npm run build
Set-Location ../..
```

验证仓库内锁定的 SQLCipher 源码，再把未签名的 Windows x64 Release 可执行文件构建到当前盘符根目录下一个新的短 ASCII 路径中：

```powershell
./scripts/test-production-sqlcipher-vendor.ps1
$driveRoot = [System.IO.Path]::GetPathRoot((Get-Location).Path)
$targetRoot = Join-Path $driveRoot 'desktop-notes-release-target'
./scripts/run-production-dependency-build.ps1 -TargetRoot $targetRoot
```

只有在 `Cargo.lock` 对应依赖源码已完整存在于 Cargo 缓存中时才使用 `-Offline`。当前 Tauri 配置生成未签名便携版可执行文件，不生成安装包，也没有启用生产签名。

## 测试

在仓库根目录运行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
```

自动化套件覆盖 Rust、前端、安全和回归行为，但不能代替真实电池和多显示器硬件测试。

## 欢迎参与硬件测试

欢迎社区帮助验证真实 Windows 节电模式、双显示器混合 DPI、负坐标布局以及显示器断开/重连。有效报告请包含 Windows 版本、设备类型、显示器布局、各显示器缩放比例、准确步骤和观察结果；不要附带任何私人数据。

## 贡献与安全报告

- 贡献指南：[CONTRIBUTING.md](CONTRIBUTING.md)
- 安全报告：[SECURITY.md](SECURITY.md)

## 许可证

桌面笔记项目代码采用 [MIT License](LICENSE)，版权归 `JasonYang-GJ` 所有。

第三方组件继续适用各自的许可证和声明，详见 [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt)。
