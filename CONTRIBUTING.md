# 参与桌面笔记开发

桌面笔记目前以 Windows 为主要平台。所有贡献都应遵守本文档说明的产品和安全边界。

## 工作流程

1. Fork 本仓库。
2. 为一项明确改动创建独立分支。
3. 用最小改动解决问题。
4. 运行格式化、Lint、类型检查和相关测试。
5. 创建 Pull Request，说明行为变化、已运行测试和仍存在的限制。

## 必须通过的检查

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

修改 SQLCipher、加密、密钥处理、备份、持久化或窗口行为时，还必须提供对应的专项回归证据。

## 数据安全

不得提交或附带：

- API Key、令牌、密码、证书或私钥
- 真实笔记、笔记数据库、DataRoot 内容、备份或剪贴板捕获内容
- 个人截图、日志、环境转储或特定机器凭据
- `target`、`node_modules`、前端生成物、WebView 配置目录或本地测试产物

测试资料必须使用明确的虚构内容。如果问题无法在不使用私人数据的情况下复现，请只描述数据结构，不要上传原始数据。

## 代码与文档要求

- Rust 代码必须通过 `cargo fmt` 和禁止警告的 Clippy 检查。
- 前端代码必须通过 ESLint 和 TypeScript 类型检查。
- 面向用户的新界面和提示默认使用简体中文；协议标识和错误码保持稳定。
- 文档应简洁且技术表述准确。
- 不得把 Windows 支持范围扩大到尚未测试的环境。
