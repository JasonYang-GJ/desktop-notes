# 桌面笔记 Desktop Notes

[![持续集成](https://github.com/JasonYang-GJ/desktop-notes/actions/workflows/ci.yml/badge.svg)](https://github.com/JasonYang-GJ/desktop-notes/actions/workflows/ci.yml)
![支持平台](https://img.shields.io/badge/平台-Windows%2010%20%7C%2011-0078D4?logo=windows&logoColor=white)
![项目状态](https://img.shields.io/badge/状态-Beta-F59E0B)
[![MIT License](https://img.shields.io/badge/许可证-MIT-2EA44F.svg)](LICENSE)

**把每天的笔记放回日历，而不是丢进越来越长的列表。**

桌面笔记是一款面向 Windows 的本地优先日历笔记应用。你可以按日期记录想法、待办和资料，再通过标签、最近笔记与全文搜索快速找回。笔记数据库、图片和自动备份均加密保存在当前 Windows 设备上，日常记笔记无需联网。

> [!IMPORTANT]
> 当前公开版本为 **v0.1.0-beta.1**，属于 Beta 测试版，并非稳定版 1.0。仓库目前提供源码，尚未发布安装包或预编译可执行文件；开发者可按下方说明自行构建。

## 一眼看懂

| 你关心的 | 当前情况 |
| --- | --- |
| 它是做什么的？ | 按日历日期创建、整理和查找本地笔记 |
| 数据放在哪里？ | 主要数据保存在当前 Windows 设备，不依赖云端笔记服务 |
| 有哪些核心能力？ | 富文本、标签、置顶、最近笔记、全文搜索、快速笔记和自动备份 |
| 数据如何保护？ | 数据库使用 SQLCipher，加密图片与备份，密钥材料由 Windows DPAPI CurrentUser 保护 |
| 现在能直接安装吗？ | 暂时不能；当前提供源码，尚无 GitHub Release、安装包或预编译程序 |
| 支持哪些系统？ | 主要支持 Windows 11 x64；已测试 Windows 10 22H2 x64 |

## 使用方式

1. 在日历中选择一天，新建或打开当天的笔记。
2. 使用富文本、标签和置顶整理内容，也可以通过全局快捷键快速记录。
3. 通过最近笔记或本地全文搜索，找回标题、正文和标签中的内容。
4. 应用在本机保存加密数据库，并自动生成加密备份。

## 核心功能

- **按日期记录：** 在日历中创建和整理每天的笔记。
- **富文本编辑：** 支持常用格式，并能安全粘贴含行内代码样式的 ChatGPT 富文本。
- **快速整理：** 使用标签、置顶和最近笔记管理内容。
- **本地搜索：** 在标题、正文和标签中进行全文搜索。
- **快速笔记：** 通过可配置的全局快捷键随时记录。
- **本地加密：** 使用 SQLCipher 加密数据库，图片和自动备份同样加密保存。
- **灵活窗口：** 提供收起、标准和展开三种窗口状态，日历与笔记区域的分界线可拖动。
- **中文体验：** 提供简体中文界面和中文日期显示。

## 项目状态

- 已公开：完整源码、锁定依赖、公共测试、构建说明、安全政策和贡献指南。
- 尚未提供：安装包、预编译便携版、代码签名和正式稳定版。
- 当前目标：继续验证 Beta 版本，尤其是真实电池与多显示器环境。

如果你只是想直接安装使用，建议关注本仓库后续的 [Releases](https://github.com/JasonYang-GJ/desktop-notes/releases)。如果你愿意参与开发或从源码体验，请继续阅读下方说明。

<details>
<summary>English summary</summary>

Desktop Notes is a local-first calendar notes app for Windows. It organizes notes by date and provides rich-text editing, tags, pinned and recent notes, full-text search, quick capture, encrypted images, and encrypted automatic backups. Normal note-taking does not require a network connection.

The current public version is **v0.1.0-beta.1**. Source code is available, but no installer or prebuilt executable has been published yet.

</details>

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

## 技术栈

- 桌面框架：Tauri 2
- 前端：React、TypeScript
- 核心与系统集成：Rust
- 本地存储与搜索：SQLite、FTS5
- 数据库加密：SQLCipher 4.18.0
- Windows 密钥保护：DPAPI CurrentUser

## 从源码构建

### 开发环境

- Windows 11 x64，或用于兼容性开发的 Windows 10 22H2 x64
- Rust 1.98.0，安装 `x86_64-pc-windows-msvc`、`rustfmt` 和 `clippy`
- Node.js 20.12.2 与 npm；前端依赖由 `apps/desktop/package-lock.json` 锁定
- Microsoft C++ Build Tools，并安装“使用 C++ 的桌面开发”
- Microsoft Edge WebView2 Runtime
- 带有 `Locale::Maketext::Simple` 和 `IPC::Cmd` 模块的 Perl，用于构建仓库内锁定的 OpenSSL 源码

### 构建步骤

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

## 参与项目

- 贡献指南：[CONTRIBUTING.md](CONTRIBUTING.md)
- 安全报告：[SECURITY.md](SECURITY.md)
- 更新记录：[CHANGELOG.md](CHANGELOG.md)
- Beta 说明：[docs/releases/v0.1.0-beta.1.md](docs/releases/v0.1.0-beta.1.md)
- 普通问题与建议：[GitHub Issues](https://github.com/JasonYang-GJ/desktop-notes/issues)

## 许可证

桌面笔记项目代码采用 [MIT License](LICENSE)，版权归 `JasonYang-GJ` 所有。

第三方组件继续适用各自的许可证和声明，详见 [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt)。
