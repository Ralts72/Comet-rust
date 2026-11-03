# 仓库协作规范

本文件适用于整个 Comet Rust 仓库。用户当前任务中的明确要求优先于本文件；旧仓库文档作为行为与迁移参考，不作为新工程的构建指令。

## 项目与目录

这是 Comet 的独立 Rust 重构工程，使用 Cargo workspace；工具链以 `rust-toolchain.toml` 为准，Edition、依赖与构建配置统一在根 `Cargo.toml`。
新工程不链接旧 C++ 引擎或其 C/C++ 第三方实现；系统 SDK、驱动和原生 API 绑定可以使用。

- `engine/core`：分类 UUID、摘要、原子文件发布等基础协议。
- `engine/scene`：具体组件、字段 Schema、ECS 场景、层级与 Prefab。
- `engine/asset/data`：CPU 产品与索引协议，不包含 Store、导入器或 GPU。
- `engine/asset/runtime`：只读产品加载、校验、依赖闭包、版本切换与缓存。
- `engine/asset/pipeline`：源扫描、导入、构建、打包与项目配置库；保持独立 Cargo 包。
- `engine/runtime`：独立运行世界与模拟生命周期。
- `engine/application`：窗口、输入、egui 接入、帧提取和公共宿主流程。
- `engine/render`：帧协议、渲染线程、共享 Shader 与原生 Metal/Vulkan 后端；Shader 位于其 `shaders/`。
- `editor/core`：作者文档、编辑事务、撤销/重做与保存点；`editor/src`：编辑器入口、会话与面板。
- `app`：独立游戏运行入口，仅加载构建好的产品。
- `tools/asset`：资产 CLI 薄入口；`tools/gpu`：按能力组织的原生 GPU 诊断。
- `tools/launch`：编辑器启动准备及其测试；`tools/verify`：工程验收、模块边界与依赖审计。
- `demo/assets`：唯一默认示例的作者资源；`tests/fixtures`：按功能组织的独立测试数据。
- `docs`：`architecture.md` 记录架构与依赖，`development.md` 记录开发与验证，`migration.md` 记录能力迁移清单；根 `target`：缓存和生成物。

新增模块前明确职责、数据所有权和依赖方向；在已有职责内扩展时优先留在对应模块，不提前创建空的未来模块。
`lib.rs` 负责模块组织与公共出口，具体规则放到所属文件；测试就近放在模块内或各包的 `tests/`。
正式目录、包名、默认路径和验收命令按完整项目职责组织，不保留阶段专属流程或选型对照代码。阶段编号只用于迁移计划；可复用能力进入正式模块或按功能组织的验证工具，不能仅改名后继续保留原型流程。

## 架构与技术边界

- 架构、格式、依赖及接入状态以 `docs/architecture.md` 为准，开发与验收流程见 `docs/development.md`。已确定的 ECS、物理、导航、脚本、UI 路线按实际功能接入，区分计划、已实现与已验证。
- 桌面平台共同支持 macOS、Windows、Linux；macOS 使用原生 Metal（objc2-metal），Windows/Linux 共用原生 Vulkan（ash），暂不接入 DX12。新增或修改通用功能时同步考虑三个平台的实现和验收，不再将 Windows/Linux 的适配整体后置。共享帧协议、Shader 与数据布局，原生资源和编码归各自后端；保留显式多队列、原生显存别名和 Mac 硬件光追的直接控制。
- wgpu、Rune 不属于正式路线，不引入对照包或其可选构建路径。依赖更新通过 Cargo 管理，不修改本机 registry 缓存或第三方源码。
- Scene 不拥有编辑历史或资产产品；Editor Core 不拥有运行实例；Play 使用独立 Runtime 世界，停止运行不覆盖作者文档。
- 导入与构建属于资产 Pipeline，只读消费属于资产 Runtime。App 不依赖编辑器或导入器。
- Application 负责窗口和主线程工作；Render 只依赖资产数据，消费提取后的帧数据，不接触 AssetStore 或借用正在修改的场景。GPU 对象的创建、使用和释放遵守后端的线程与生命周期约束。
- 持久身份使用项目已有的分类 UUID 和稳定字段 ID；保留资产 metadata。内容协议变化需明确版本与迁移，不以 Rust 字段重命名或重新生成 ID 隐式改变协议。
- 调整正式模块或依赖方向时，同步维护 `tools/verify/check_boundaries.py`；门禁应表达实际架构约束，不通过放宽禁用依赖来掩盖职责混用。

## 构建与启动

日常只保留 `start.sh` 一个启动入口，固定准备 Debug Editor 和 Release App 模板，再启动编辑器：

```sh
./start.sh
./start.sh --help
./start.sh --project /path/to/project --scene main.scene
# Windows 原生终端使用同一公共实现：python tools/launch/launch.py
```

脚本只接收编辑器参数，不提供可选编译动作或构建模式切换，不恢复独立 App/Debug/Release 启动脚本。
默认使用 demo 与 target/products/demo。游戏导出、场景选择和试玩由编辑器处理；导出复用匹配的 Release App 模板，不调用 Cargo，程序源码变化后重新运行 start.sh。
项目、产品、截图、报告和导出目的地相对调用目录解析；作者场景通过公共 ProjectPaths 限制在项目 assets 内。正式导出在同级候选完成校验后整体发布，不覆盖已有目的地。

使用锁定的工具链与 `Cargo.lock`，尊重 `.cargo/config.toml` 的并行配置并复用 workspace 缓存。
Debug 保留本仓库代码的完整调试信息，第三方依赖关闭调试信息；测试继承开发配置。Release 使用性能优化、Thin LTO、符号剥离与非增量构建。
修改构建策略统一调整根 `Cargo.toml`，不在启动工具里维护另一套编译配置。

常用开发与验证命令：

```sh
cargo build --locked
cargo test --locked -p comet-scene                # 按修改范围选择包
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/verify/check_boundaries.py
python3 tools/verify/audit_dependencies.py
python3 tools/verify/verify.py                            # 当前完整 CPU/构建/打包验收
python3 tools/verify/verify.py --native --linux --windows  # 本机 GPU/窗口、Linux 与 Windows 交叉检查
```

默认 `cargo build/test` 面向引擎库、编辑文档和资产工具；`--workspace` 包含应用与独立 GPU 工具，均为当前正式模块。
Python 工具需要 Python 3；直接运行 Cargo 或验证工具时确保 rustup 命令目录在 PATH 中。macOS 需要 Metal/AppKit SDK 与 Xcode Command Line Tools。
`--native` 会打开本机真实窗口；`--linux` / `--windows` 分别需要 x86_64-unknown-linux-gnu / x86_64-pc-windows-msvc 标准库。CI runner 仍仅用 Linux，并执行 Windows 交叉检查；每个功能区分三平台的编译、CPU 测试、原生链接和 GPU/窗口验收，无法实测的项记录为待验收，不能用另一平台结果替代。

## 代码风格

- Rust 遵循 rustfmt，使用四空格缩进；类型和 trait 使用 `PascalCase`，函数、模块与变量使用 `snake_case`，常量使用 `SCREAMING_SNAKE_CASE`。
- 保持文件按职责组织，公共 API 取实际需要的最小范围；不引入包揽所有状态的 Engine/Context 单例或无实际用途的抽象层。
- 可恢复的输入、资产和 I/O 错误通过 `Result` 传播，并附带足以定位问题的上下文。保持候选校验与成功发布边界，避免失败后只更新部分状态。
- 所有权、资源释放和线程退出路径应明确；必要的 `unsafe` 写明安全前提，尤其是 FFI 对象、原始句柄和线程交接，不能用未证明安全的 `Send/Sync` 实现绕过约束。
- 注释简短，解释原因、协议或安全约束；Python 和 Shell 修改沿用现有风格。

## 验证与生成物

- 按影响范围验证：纯文档修改检查内容、路径和 diff；逻辑修改运行相关测试、格式与 Clippy；跨模块、资产、渲染或线程改动运行当前完整验收。依赖变化还需检查模块边界与依赖审计。
- 测试关注行为与失败边界，例如身份稳定、引用校验、历史与 Play 隔离、产品切换、线程关闭和 Shader 布局；不为低影响改动增加复述实现的测试。
- 资产写入、保存、打包与恢复测试使用独立临时项目，避免修改示例作者源或用户项目。
- 分别记录编译、单元测试、GPU 离屏和真实窗口结果；Metal 实测不代表 Vulkan 实测，交叉编译不代表目标平台已运行。
- 新增生成物放在根 `target/` 或临时目录，避免在源目录新增构建缓存。不要自动清理已有缓存、历史证据或用户产物。
- 当前验收结果及源码快照位于 `target/verification`，CI 附件保存本次运行结果；阶段报告、日志、截图与运行快照不放进 `docs` 或版本控制。证据必须对应源码、命令和实际结果，不用旧快照声称新改动已经通过。

## 沟通、README 与 Git

- 与用户沟通使用中文，`README.md` 保持中文。结果说明改了什么、为何修改、实际验证与仍未验证的部分。
- 每次变更复查 README；仅当项目说明、架构、环境、用法、命令或贡献流程发生变化时更新，不把 README 当作变更流水账或临时报告索引。
- 未经用户明确要求，不自动暂存、提交或推送。保留已有未提交改动，不重置或覆盖用户文件；旧 C++ 仓库默认只读，除非任务明确包含旧仓库修改。
- 用户要求提交或 PR 时，使用简短的 Conventional Commit 风格标题，说明问题、最终变化与验证；渲染/UI 改动提供相应截图或运行证据。
- 未经用户明确要求，不重写已有 `AGENTS.md`。

<!-- CODEGRAPH_START -->
## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root), reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **MCP tool** (when available): `codegraph_explore` answers most code questions in one call — the relevant symbols' verbatim source plus the call paths between them, including dynamic-dispatch hops grep can't follow. Name a file or symbol in the query to read its current line-numbered source. If it's listed but deferred, load it by name via tool search.
- **Shell** (always works): `codegraph explore "<symbol names or question>"` prints the same output.

If there is no `.codegraph/` directory, skip CodeGraph entirely — indexing is the user's decision.
<!-- CODEGRAPH_END -->
