# Auto Judge

Auto Judge 是一个面向数据结构课程机试训练的 macOS / Windows 桌面应用。它会从本地课程资料和 `ds_lz/` 历年题中整理考点，调用用户配置的 OpenAI-compatible / DeepSeek API 生成 C11 编程题，并在本地生成测试数据、标准答案和评测结果。

## 功能

- 知识点选择：从课程资料中归纳链表、栈队列、树、图、排序、查找、文件输入等考点。
- 历年题内置：`ds_lz/` 中完整题面会作为内置题库，带本地样例与 10 组测试数据。
- 自动命题：生成贴近往年风格的长题面、低思维难度、高实现细节题目。
- 本地出数据：模型返回 C11 `dataGenerator`，应用本地编译运行生成 10 组测试输入。
- 本地对拍：模型返回 C11 `referenceSolution`，应用对每组输入运行标准程序生成 expected output。
- 本地评测：用户只提交 C11 代码，应用本地编译运行并给出 AC / WA / TLE / RE / CE。
- 文件题支持：支持 `in.txt`、`files.txt` 等附带文件，样例和评测结果均可直接打开本地文件。
- 本地历史：生成题目、样例、测试点与参考代码保存在本机应用数据目录。

## 数据生成协议

新生成题不要求模型直接返回大段 `testInputs`。模型需要返回：

- `referenceSolution`：完整 C11 标准对拍程序。
- `dataGenerator`：完整 C11 数据生成程序。

`dataGenerator` 必须：

- 使用固定种子 `srand(20260616)`。
- 恰好生成 10 组正式测试输入。
- 第 1-2 组为边界/最小规模。
- 第 3-5 组为人工构造小数据。
- 第 6-8 组为随机中大规模。
- 第 9-10 组为极限/卡边界大规模。
- 覆盖重复值、相等关键字、逆序/乱序、空结果/无解、最大值或接近最大值。
- 用单独一行 `---AUTO_JUDGE_CASE---` 分隔相邻测试输入。

应用会校验生成器输出：必须恰好 10 组、不能完全相同、后 5 组需要明显大于前 5 组。

## 安装使用

在 GitHub Release 中下载对应平台安装包：

- macOS Apple Silicon：`Auto Judge_0.1.0_aarch64.dmg`
- Windows x64：`Auto Judge_0.1.0_x64-setup.exe`

首次使用时，在右侧 Agent 面板中填写：

- API URL：例如 `https://api.deepseek.com`
- API Key：用户自己的密钥
- Model：默认 `deepseek-v4-pro`

API Key 和 API URL 只保存在本机应用数据目录，不会写入仓库。

## 本地开发

依赖：

- Node.js
- Rust
- Tauri 2 依赖
- `gcc`，用于 C11 编译和本地 judge

安装依赖：

```bash
npm install
```

运行开发版：

```bash
npm run dev
```

仅运行前端预览：

```bash
npm run dev:web
```

检查和构建前端：

```bash
npm run build:web
```

运行 Rust 测试：

```bash
cd src-tauri
cargo test
```

## 打包

macOS DMG：

```bash
npm run pack:mac -- --no-sign
```

Windows NSIS：

```bash
npm run tauri -- build --bundles nsis --target x86_64-pc-windows-gnu
```

产物默认位于：

- `src-tauri/target/release/bundle/dmg/`
- `src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/`

## 安全说明

本地 judge 会执行用户提交的 C11 程序。当前版本主要依赖临时目录隔离和运行超时，不提供完整系统级沙箱。请勿在不可信环境中运行恶意代码。正式分发前建议补充 macOS sandbox、Windows Job Object、文件系统白名单和内存限制。

## License

当前仓库未指定开源许可证。发布到 public 仓库不代表自动授予开源使用许可；如需开放复用，请补充 LICENSE。
