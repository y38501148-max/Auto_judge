# Auto Judge 架构设计

## 目标

Auto Judge 是一个 macOS/Windows 桌面应用，用于从本地数据结构课程资料中整理考点，调用 DeepSeek API 生成编程题、C11 参考代码、C11 数据生成器和展示样例，并通过本地出数据与对拍生成标准答案。生成结果会保存到本机历史，用户可以提交 C11 代码进行本地评测。

## 技术栈

- 桌面框架：Tauri 2
- 后端：Rust，负责资料索引、DeepSeek/OpenAI-compatible API 调用、Agent 缓存、历史落盘、标准程序运行、用户代码评测
- 前端：React + Vite + TypeScript
- 本地数据：应用数据目录下的 JSON 与测试点文件
- 发布：macOS DMG、Windows NSIS

## 数据来源

- `~/Desktop/Coding/DS_helper/resources`：课程 PPTX 资源，应用会归纳为可选择的知识点，不直接把课件文件名暴露给用户
- `ds_lz/`：历年 PDF 与 Markdown 题面，历年题不作为知识点混选，而是作为内置练习题直接提供 judge

后续可以增加 PPTX XML 正文抽取，把每页标题和关键词写入可检索索引。

当前内置历年题：

- 2018 学生在线上机时间统计
- 2018 后缀表达式计算
- 2018 网络打印机选择
- 2019 空闲内存空间合并
- 2019 火车货运调度模拟
- 2019 查找同名文件
- 2020 机试异常检测
- 2021 汉明距离
- 2021 二叉搜索树
- 2021 解释系统
- 2022 查找同时空人员
- 2022 文件拷贝

每道内置历年题都有样例和 10 组测试输入输出。文件类历年题会在测试点中保存 `in.txt` 或 `files.txt` 等附带文件。

## 生成流程

1. 前端启动后调用 `bootstrap`。
2. Rust 扫描资料目录，返回可选择的考点列表与历史记录。
3. 用户选择一个或多个考点，并决定是否包含文件输入输出考点。
4. 用户在本地设置中输入 API URL、API Key 和模型名，应用保存到本机应用数据目录。
5. 前端调用 `generate_problem`，传入考点、补充要求和本地设置。
6. Rust 组装 prompt，按 API URL 请求 DeepSeek-pro 或其他兼容 Chat Completions 的模型，要求返回严格 JSON。
7. 若相同 API URL、模型、考点和要求已存在 Agent 缓存，则直接复用缓存草稿，避免重复消耗 API。
8. Rust 编译并运行模型给出的 C11 数据生成器，生成恰好 10 组正式测试输入，并校验数据差异和大小分布。
9. Rust 编译并运行模型给出的 C11 参考代码，对样例与正式测试输入生成标准输出。
10. Rust 将题面、参考代码、测试点输入输出和元数据保存到应用数据目录。
11. 前端刷新历史并展示当前题目。

## Judge 流程

1. 用户输入 C11 代码。
2. 前端调用 `judge_submission`。
3. Rust 在临时目录中编译用户代码。
4. 对每个测试点运行程序，3 秒超时。
5. 对标准输出做行尾归一化后比较。
6. 返回每个测试点的 AC/WA/TLE/RE/CE 结果和摘要。

## 安全边界

本地 judge 会执行用户提供的代码。第一版只有临时目录隔离和超时控制，不提供系统级沙箱。正式发布前应补充：

- macOS sandbox profile 或独立受限 runner
- Windows Job Object 限制
- 文件系统访问白名单
- 内存限制与进程树清理

API Key 第一版按用户要求保存在本地应用数据目录的 `settings.json` 中。正式分发前建议升级为 macOS Keychain 与 Windows Credential Manager。

## 发布路径

- macOS：`npm run pack:mac` 或 `npm run pack:mac:universal`
- Windows 本机：`npm run pack:win`
- macOS 交叉构建 Windows：`npm run pack:win:cross`
- 当前验证产物：
  - `release/0.1.0/Auto Judge_0.1.0_aarch64.dmg`
  - `release/0.1.0/Auto Judge_0.1.0_x64-setup.exe`

当前产物未做代码签名。正式分发前应补充 Apple Developer ID/notarization 与 Windows 代码签名。
