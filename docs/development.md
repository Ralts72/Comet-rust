# 开发与验证

开发与验证按当前功能组织。结果写入 `target/verification/`，源码与工具链快照写入 `source-manifest.json`；运行日志、截图和快照不放进版本控制。启动方式和应用参数见 [README](../README.md)，模块与第三方依赖见[架构说明](architecture.md)。

## 开发环境

使用 `rust-toolchain.toml` 固定的 Rust 工具链及其 rustfmt、Clippy 组件；Cargo 依赖由 `Cargo.lock` 锁定。Python 工具只使用 Python 3 标准库。编译共用根 `target/`，并行配置由 `.cargo/config.toml` 管理。macOS 需要 Xcode Command Line Tools；Linux 运行图形程序时需要系统 Vulkan loader 与驱动。
二进制源资产使用 Git LFS，初次准备仓库时按 README 安装并拉取；不需要初始化 C++ 第三方子模块。

## 验证命令

```sh
python3 tools/verify.py
python3 tools/verify.py --gpu
python3 tools/verify.py --native --linux
```

直接执行验证工具前确保 rustup 在 PATH 中。默认执行格式检查、全 workspace 测试、Clippy、模块边界、依赖审计、应用构建、资产构建和独立包校验。
`--gpu` 增加真实离屏渲染与原生 GPU 能力检查；`--native` 还打开和关闭窗口，验证两个启动脚本、Release 游戏与可搬移的发布包。窗口烟测提交 8 帧后自动退出。
`--linux` 对全部模块交叉编译和 Clippy，并检查目标平台的边界及依赖；需要安装对应标准库目标。它不代表已经在 Linux 上执行 GPU 工作。

## 依赖与模块审计

```sh
cargo tree -p comet-render --locked
cargo tree -p comet-player --locked
python3 tools/audit_dependencies.py
python3 tools/check_boundaries.py
```

`cargo tree` 展示解析后的依赖和版本；平台条件决定实际选用的后端。审计结果写入 `target/verification/dependencies/`。
依赖审计拒绝 cc/cmake、相关 C/C++ 构建封装与非正式图形路线，并检查 Objective-C 异常 shim 未启用。
模块门禁另外约束：Scene 不依赖产品或 Store，Render 只依赖资产数据，Player 不依赖编辑器、导入器和源图像解码库。

## GitHub CI

[工作流](../.github/workflows/ci.yml) 仅运行 Linux，在 PR、`main/master` 推送及手动触发时执行；相同 PR 或分支的新运行会取消旧运行。使用 Ubuntu 24.04 x64，覆盖 Linux 原生测试、Vulkan 后端编译与 Clippy、资产与独立包检查、Release 游戏和资产工具构建。

CI 复用 `python3 tools/verify.py`，包括格式、workspace 测试、Clippy、模块边界、依赖审计、Debug 应用构建及产品包隔离检查。Rust 版本、rustfmt 与 Clippy 由 `rust-toolchain.toml` 自动安装，Cargo 命令使用锁文件；checkout 拉取 Git LFS 资产，不初始化 C++ 第三方子模块。首次冷编译的测试、Clippy 与应用构建各允许 15 分钟，其余检查默认 5 分钟；单个 CI job 最长 30 分钟。

依赖缓存按平台、工具链和 Cargo 配置区分，只在仓库默认分支保存，PR 可以读取已有缓存。缓存不保留 workspace 自身产物、增量编译目录、资产产品或验证临时项目；CI 关闭增量编译，本地构建配置保持原有用途。日志、报告、依赖审计与源码快照在失败时也尝试上传，保留 7 天，不上传整个 `target/` 或游戏包。外部 Actions 固定到完整 commit SHA。

托管 CI 不执行 `--gpu` 或 `--native`；此处的 Vulkan 检查覆盖编译和 CPU 行为，Metal、物理 GPU、呈现、多队列和显存别名需在有对应设备的环境验收。Runner 标签和架构以 [GitHub 官方说明](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)为准。

## 内容、编辑与资产

- Scene 测试覆盖稳定身份、字段与引用、层级、复制、句柄失效、序列化和 Prefab 展开。
- Document 测试覆盖连续编辑合并、撤销/重做、取消、保存点和无效候选；Editor 测试覆盖保存重开、Play 隔离、拾取代次与合成 Gizmo 拖动。
- 资产流程测试使用 `tests/fixtures/prefab`、`tests/fixtures/gltf` 的临时副本，覆盖 metadata 身份、产品复用、失败后的索引保持、外部输入复核、glTF 多材质/子身份和坏输入。
- 默认 `demo/` 是当前示例项目，不用于测试写回。导入、保存和破坏性测试使用独立临时项目。
- 测试数量与结果以本次执行日志为准。

## 渲染、脚本入口与独立包

Render 单元测试编译共享 Shader 的两种输出，检查帧布局、CPU 拾取、背压、失败反馈与关闭路径。
GPU 验收检查 5 次场景绘制、专用渲染线程、零尺寸不呈现、目标代次变化、PNG 读回和窗口 resize。
原生编辑器测试使用 `editor.sh`；游戏脚本使用 `run_game.sh` 和已构建的临时产品，固定 Debug/Release 用途，不提供可选编译动作。

资产工具从入口场景导出 12 个产品的依赖闭包、Player 和 `package.json` 到新目录。验收将包搬移，移除本次临时作者源和开发产品，从空工作目录检查产品并启动包内游戏，验证没有作者源与 cwd 依赖。
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

macOS Metal 已有实际窗口和 GPU 运行；原生 Vulkan 设备运行、Windows、实体手柄、系统中文输入候选窗和物理鼠标 Gizmo 拖动仍需设备验收。合成 egui 拖动测试与真实鼠标操作分开记录。
烟测的零尺寸帧由程序主动注入，不代表已经验收系统最小化、跨屏或设备丢失。当前视口 Gizmo 只支持平移，旋转和缩放使用 Inspector。
任何未执行、环境不具备或检查失败的项目，必须保留状态，不用编译通过或另一平台结果代替。
当前发布物是可搬移二进制与产品目录；完整 macOS 应用包、签名和安装器尚未实现。
