# 开发与验证

开发与验证按当前功能组织。结果写入 `target/verification/`，源码与工具链快照写入 `source-manifest.json`；运行日志、截图和快照不放进版本控制。启动方式和应用参数见 [README](../README.md)，模块与第三方依赖见[架构说明](architecture.md)。

## 开发环境

使用 `rust-toolchain.toml` 固定的 Rust 工具链及其 rustfmt、Clippy 组件；Cargo 依赖由 `Cargo.lock` 锁定。Python 工具只使用 Python 3 标准库。编译共用根 `target/`，并行配置由 `.cargo/config.toml` 管理。macOS 需要 Xcode Command Line Tools；Windows 原生构建使用 MSVC 工具链和 Windows SDK；Windows/Linux 运行图形程序时需要系统 Vulkan loader 与驱动。
二进制源资产使用 Git LFS，初次准备仓库时按 README 安装并拉取；不需要初始化 C++ 第三方子模块。

## 验证命令

```sh
python3 tools/verify/verify.py
python3 tools/verify/verify.py --gpu
python3 tools/verify/verify.py --native --linux --windows
```

直接执行验证工具前确保 rustup 在 PATH 中。默认执行格式检查、全 workspace 测试、Clippy、模块边界、依赖审计、应用构建、资产构建和独立包校验。
`--gpu` 增加真实离屏渲染与原生 GPU 能力检查；`--native` 还打开和关闭窗口，验证 start.sh、Release App 模板与编辑器导出的独立包。窗口烟测提交 8 帧后自动退出。
`--smoke` 使用确定的 1/60 秒宿主帧时间，初始帧与宿主状态恢复帧为零；离屏 App 必须实际执行 7 个固定步。日常运行使用单调实时时间，烟测的确定时间不用于性能结论。
`--linux` 对全部模块交叉编译和 Clippy，并检查目标平台的边界及依赖；需要安装对应标准库目标。它不代表已经在 Linux 上执行 GPU 工作。`--windows` 同样检查 x86_64-pc-windows-msvc 的全部模块、测试源码、Clippy、边界和依赖；不进行 Windows 链接，也不执行 Windows 程序。先运行 `rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc` 准备目标标准库；在已有原生验证的平台上按需选择另一目标，避免重复相同检查。Windows 运行验证工具使用 `python tools/verify/verify.py`，启动由同一 Python 实现负责，不要求 POSIX Shell。

## 依赖与模块审计

```sh
cargo tree -p comet-render --locked
cargo tree -p comet-app --locked
python3 tools/verify/audit_dependencies.py
python3 tools/verify/check_boundaries.py
```

`cargo tree` 展示解析后的依赖和版本；平台条件决定实际选用的后端。审计结果写入 `target/verification/dependencies/`。
依赖审计拒绝 cc/cmake、相关 C/C++ 构建封装与非正式图形路线，并检查 Objective-C 异常 shim 未启用。
模块门禁还拒绝 Core 以外的正式包直接依赖 glam；模块边界约束：Scene 不依赖产品或 Store，Render 只依赖资产数据，App 不依赖编辑器、导入器和通用源图像解码库；Application 仅解码内置或包内 PNG 图标。

## GitHub CI

[工作流](../.github/workflows/ci.yml) 仅运行 Linux，在 PR、`main/master` 推送及手动触发时执行；相同 PR 或分支的新运行会取消旧运行。使用 Ubuntu 24.04 x64，覆盖 Linux 原生测试、Vulkan 后端编译与 Clippy、资产与独立包检查、Release 游戏和资产工具构建，并交叉检查 Windows MSVC 全工程。

CI 复用 `python3 tools/verify/verify.py --windows`，包括格式、workspace 测试、Clippy、模块边界、依赖审计、Debug Editor/Release App 模板准备、编辑器后台导出及产品包隔离检查。Rust 版本、rustfmt 与 Clippy 由 `rust-toolchain.toml` 自动安装，Cargo 命令使用锁文件；checkout 拉取 Git LFS 资产，不初始化 C++ 第三方子模块。首次冷编译的测试、Clippy 与应用构建各允许 15 分钟，其余检查默认 5 分钟；单个 CI job 最长 30 分钟。

依赖缓存按平台、工具链和 Cargo 配置区分，只在仓库默认分支保存，PR 可以读取已有缓存。缓存不保留 workspace 自身产物、增量编译目录、资产产品或验证临时项目；CI 关闭增量编译，本地构建配置保持原有用途。日志、报告、依赖审计与源码快照在失败时也尝试上传，保留 7 天，不上传整个 `target/` 或游戏包。外部 Actions 固定到完整 commit SHA。

托管 CI 不执行 `--gpu` 或 `--native`；此处的 Vulkan 检查覆盖编译和 CPU 行为，Metal、物理 GPU、呈现、多队列和显存别名需在有对应设备的环境验收。Runner 标签和架构以 [GitHub 官方说明](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)为准。

## 内容、编辑与资产

- Runtime 测试覆盖不同渲染帧率下的固定步一致性、补步上限与掉时、快速点按、事件顺序、鼠标增量仅消费一次、输入队列失败保持、暂停单步和新世界基线；宿主测试覆盖焦点/可见性恢复与 UI 键盘授权。Editor 回归通过实际 egui 帧调用同一套 Runtime。
- 物理测试覆盖落体着地、完整 32 位碰撞过滤、触发进入/退出、走跑跳与落地、墙/台阶/坡道/移动平台、暂停单步、瞬移、非法命令及不同渲染帧率下的一致性；Scene 测试另验证旧版本往返、物理候选失败和保留句柄的批量姿态写回。
- Scene 测试覆盖稳定身份、字段与引用、层级、复制、句柄失效、序列化和 Prefab 展开。
- Document 测试覆盖连续编辑合并、撤销/重做、取消、保存点和无效候选；物理组件测试覆盖版本 1 升级、刚体/角色成组变更、Prefab 展开后的重复玩家拒绝及失败保持。Editor 测试覆盖物理配置保存重开、Play 只读与隔离、拾取代次，以及实际 egui 控件的物理参数/Gizmo 合成拖动。
- 资产流程测试使用 `tests/fixtures/prefab`、`tests/fixtures/gltf` 的临时副本，覆盖 metadata 身份、产品复用、失败后的索引保持、外部输入复核、glTF 多材质/子身份和坏输入。
- 默认 `demo/` 是当前示例项目，不用于测试写回。导入、保存和破坏性测试使用独立临时项目。
- 路径测试覆盖越界、绝对路径、符号链接及保存前复核；发布测试覆盖未完成候选不可见、并发占位、不覆盖已有目标和失败清理。
- 文档测试覆盖预览脏状态、稳定只读快照、256 条历史与保存点；编辑器缓存测试包含产品更新与 Play/Stop。模板测试覆盖未提交源码指纹、不可变快照、摘要/架构/协议失配；打包测试覆盖额外场景独占依赖、陈旧索引及错误 App 摘要。
- 中文字体测试检查比例/等宽字体的汉字覆盖及实际排版；字体许可随模板和导出包分发并校验摘要。界面翻译不修改作者对象名称或资产身份。
- 退出回归使用临时项目，检查未保存预览的取消与丢弃、保存失败重试、Play 与作者保存隔离，以及确认窗口期间快捷键和模拟冻结。窗口管理器关闭按钮仍需各平台实际交互验收。
- 应用图标检查验证编辑器 App、App 模板与导出包使用各自原版图标，包含 ICNS 摘要、Info.plist 声明和签名；快照回归验证程序配对、缓存复用、图标损坏及修复。Windows 图标测试覆盖 ICO 格式、资源语言、写入失败和快照恢复，原生 Windows 环境另验证 EXE 资源写入与读回；PNG 窗口图标在原生窗口烟测中实际解码，X11 窗口图标仍需目标桌面实测。
- 项目图标回归检查设置保存失败保留旧值、UUID 在改名后有效、错误资产拒绝、配置符号链接拒绝和原始模板不变。完整验收在临时项目中使用蓝色图标覆盖默认橙色图标，验证包内 PNG 摘要、ICNS、签名，以及移除作者源后的运行；Windows 原生导出另执行资源读回，macOS 交叉检查不能代替它。
- Python 工具测试位于 tools/launch/tests，使用标准库 unittest；测试数量与结果以本次执行日志为准。

当前默认动作采样为 WASD 移动、空格跳跃、Shift 跑步、Ctrl 蹲伏、E 交互、左键主动作和视口右拖视角增量。WASD 按相机方向驱动 Rapier 胶囊，Space 和 Shift 分别执行跳跃与跑步，右拖环绕、滚轮缩放，相机跟随角色。Ctrl/E/左键目前只传递动作，没有蹲伏、交互或攻击消费者；项目可配置按键、手柄与原生 IME 仍属后续工作。

## 渲染、脚本入口与独立包

Render 单元测试编译共享 Shader 的两种输出，检查帧布局、CPU 拾取、背压、失败反馈与关闭路径。
GPU 验收检查 9 次场景绘制（原示例 5 次，加上落体和角色的两组多材质网格）、专用渲染线程、零尺寸不呈现、目标代次变化、PNG 读回和窗口 resize。产品独立 App 的离屏烟测还检查角色确实受到重力推进。
原生编辑器测试使用 `start.sh`；它准备并启动 Debug Editor，同时固化 Release App。游戏测试启动编辑器导出的包，不再提供独立游戏构建启动脚本。

默认验证复制 demo 到临时项目，增加一个独立的空场景，通过编辑器同一导出实现打包两场景及 13 个产品；默认 demo 单场景的闭包仍为 12 个产品。验证模板失配和已有输出拒绝。macOS 在临时 `.app` 完成签名与校验，Linux 验证目录包。验收将包搬移，移除本次临时作者源和开发产品，从空工作目录检查产品并启动包内游戏，验证没有作者源与 cwd 依赖。
可执行文件从 Cargo JSON 产物消息获取，兼容 Cargo 的 target 目录和构建目标配置，不手工拼接 Debug/Release 可执行文件路径。

## 原生 GPU 能力

```sh
cargo run -p comet-gpu-check --locked
MTL_CAPTURE_ENABLED=1 cargo run -p comet-gpu-check --locked -- --capture --output target/diagnostics/gpu-capture
```

Metal 工具执行两队列提交与事件同步/读回、同一 placement heap offset 的显存别名、硬件光追三角形命中/未命中及 refit 后读回。命令缓冲时间区间重叠不作为内核并行或整帧性能提升的证明。
抓帧目录必须没有旧 trace，工具不会覆盖已有 GPU trace。Vulkan 工具目前验证原生 loader、设备与队列创建，不宣称已有 Metal 同等的执行覆盖。
这些是独立能力检查；正式 Render 的多队列调度、显存寿命规划与光追 Pass 尚未接入。

## 平台与交互边界

macOS、Windows、Linux 均为正式支持目标，新增功能需要维护相同的持久协议、编辑语义、Play 行为和独立包能力。当前验证基线为 macOS Apple Silicon / Metal、Windows x64 / MSVC / Vulkan、Linux x64 / Ubuntu 24.04 / X11 / Vulkan；其他发行版、架构与原生 Wayland 尚未验收。CI runner 仍只使用 Linux。

| 平台 | 编译与静态检查 | CPU 测试 / 原生链接 | GPU、窗口、独立游戏实测 |
| --- | --- | --- | --- |
| macOS / Metal | 本机执行 | 本机执行 | 本机 --native 验收 |
| Windows / Vulkan | --windows；Linux CI 同样执行 | 需 Windows 原生环境验收 | 待 Windows 设备验收 |
| Linux / Vulkan | --linux；Linux CI 原生执行 | Linux CI 执行，结果以对应提交的 CI 为准 | 待 Linux GPU/桌面验收 |

每个功能分别记录三平台的实际证据；只有编译检查通过时，不能标记为完整的平台验收。通用功能同步推进，原生系统或硬件能力的差异要明确说明，不静默跳过。Windows EXE 图标写入及读回已接入，原生资源 API 和资源管理器显示等待 Windows 实机验收；Linux 桌面菜单入口和系统 IME 等尚未完成的接入继续按功能补齐。

macOS Metal 已有实际窗口和 GPU 运行；Windows/Linux 的原生 Vulkan 设备运行、Windows 原生链接、实体手柄、系统中文输入候选窗和物理鼠标 Gizmo 拖动仍需设备验收。合成 egui 拖动测试与真实鼠标操作分开记录。
烟测的零尺寸帧由程序主动注入，不代表已经验收系统最小化、跨屏或设备丢失。当前视口 Gizmo 只支持平移，旋转和缩放使用 Inspector。
任何未执行、环境不具备或检查失败的项目，必须保留状态，不用编译通过或另一平台结果代替。
当前发布物支持可搬移目录与 macOS ad-hoc 签名 `.app`；开发者证书签名、公证与安装器尚未实现。
