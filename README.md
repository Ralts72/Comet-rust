# Comet Rust

Comet 的独立 Rust 3D 引擎工程，沿用引擎、编辑器、游戏与工具的业务目录，用 Cargo workspace 管理模块依赖。旧 C++ 工程用于行为对照，新工程不链接其引擎或 C/C++ 第三方实现。

编辑器界面使用简体中文，内置中文字体，独立游戏包无需另行安装字体。当前支持静态 glTF/GLB 导入、多材质 PBR/阴影、场景编辑、撤销/重做、保存与独立游戏打包。桌面共同支持目标为 macOS、Windows、Linux；macOS 使用原生 Metal，Windows/Linux 使用原生 Vulkan，暂不接入 DX12。Metal 已实际运行，Windows/Linux 已纳入交叉编译检查，目标设备运行仍需验证。导航、脚本、动画和音频尚未正式接入。
Editor Play 与 App 共用 60 Hz 固定步、暂停单步、失焦冻结及动作输入消费；长帧补步有上限并记录掉时。Rapier 已接入基础刚体、碰撞层、触发器与胶囊角色；默认 demo 可观察落体，Play 和独立游戏均可操作角色。编辑器关闭窗口时检查未保存的场景，支持保存后退出、直接退出或取消；保存失败保留窗口。

## 开发与启动

使用 `rust-toolchain.toml` 固定的 Rust 工具链、Edition 2024 和 `Cargo.lock`。启动与验证工具使用 Python 3 标准库。workspace 共用根 `target/`，编译并行配置在 `.cargo/config.toml`；macOS 需要 Xcode Command Line Tools 提供 SDK 与链接器。

日常只保留一个启动入口：

Editor 是编辑器，App 是独立游戏运行程序；Debug、Release 分别表示它们使用的构建配置。App 的源码入口位于 `app/`，Cargo 包和可执行文件名为 `comet-app`。

```sh
./start.sh                    # 准备 Debug Editor、Release App 模板，启动编辑器
./start.sh --help
./start.sh --project /path/to/project --scene main.scene
```

Windows 原生终端运行 `python tools/launch/launch.py`，与 `start.sh` 共用全部准备和启动逻辑；Windows 构建使用 MSVC 工具链和 Windows SDK，运行需要支持 Vulkan 的显卡驱动。没有额外的构建模式或 App 启动入口。

脚本通过 rustup 准备锁定工具链，按顺序构建两个程序，从 Cargo JSON 获取实际路径；首次需要编译两套依赖，后续复用缓存。构建配置统一由根 `Cargo.toml` 管理，没有动作分发或模式切换。默认项目为 `demo/`，开发产品位于 `target/products/demo/`。
macOS 启动时自动准备带原版蓝色彗星图标的 `Comet Editor.app`，缓存于 `target/editor-apps/`；导出游戏使用原版橙色彗星图标。两套图标分别由 `editor/resources/icons/` 和 `app/resources/icons/` 管理，PNG 嵌入程序供桌面窗口及 Windows 任务栏使用，ICNS 随 macOS App 分发。Windows 启动时将原版 ICO 写入编辑器和 App 的独立程序快照，导出游戏复用带图标的 App；Windows 原生显示仍待实机验收。

在编辑器“项目设置 → 游戏图标”中选择项目 PNG，选择后自动保存到项目根 `project.json`，通过资产 UUID 引用，PNG 与 `.meta` 一起移动可保留配置。建议使用 1024 × 1024 像素图像；导出生成 PNG、macOS ICNS 和 Windows EXE 图标，App 窗口/任务栏同步使用项目图标。未配置时使用 Comet 默认图标，Editor 图标与原始 App 模板保持不变。demo 的图标位于 `demo/assets/icons/game.png`；导出游戏不需要 Python。
模板快照位于 `target/app-templates/`，记录源码/依赖/配置指纹、平台架构、协议版本和程序摘要；包含未提交源码，不以 Git commit 代替构建身份。修改程序源码后重新运行 `start.sh`，资源变化则由编辑器导出时重新构建。

在编辑器中点击“运行”后，WASD 按相机方向移动，Space 跳跃、Shift 跑步，右键拖动环绕、滚轮缩放；相机跟随角色。当前角色使用方块可视化，蹲伏、手柄和完整玩法仍待接入。
工具栏提供“新建场景”和“打开场景”，也可使用 Ctrl/Cmd-N、Ctrl/Cmd-O。新建时指定 assets 内的 `.scene` 位置，立即保存空场景和稳定的 `.meta` 身份，不覆盖已有文件；打开从当前项目场景列表选择。有未保存修改时，可保存后切换、放弃修改后切换或取消。候选场景加载失败保留当前文档与 Play；切换成功后使用独立的撤销历史，并将导出启动场景同步为当前场景。
属性面板可为作者根对象添加、移除刚体和胶囊角色，配置运动类型、碰撞形状、过滤层及角色参数；连续拖动合并为一次撤销，保存后供 Play 和导出使用。运行期间物理配置只读；Prefab 实例保留已有字段覆盖，组件结构在源 Prefab 中维护。

层级面板可新建空对象，右键作者对象可创建子对象、复制或删除；操作支持撤销和重做。删除对象时一并删除其子对象及挂接的 Prefab，并清除场景内指向它们的本地引用。单根 Prefab 可作为实例整体复制或删除，内部对象和多根 Prefab 的结构仍需在源文件中维护。新建的空对象暂不包含模型。

资源面板显示项目中已构建的 glTF/GLB 模型与 Prefab；选择后点击“添加到场景”或双击资源，即可创建可移动的对象容器并挂接资源实例，操作支持撤销。将源资源放入项目 assets 后，点击“刷新资源”在后台构建并更新列表与场景，无需重启；已有资源随 metadata 保持 UUID。刷新不自动保存作者场景，失败保留当前文档与已接纳资源。运行期间需先停止 Play，处理时暂时禁用编辑、切换和导出。拖放、外部文件导入界面与自动文件监听尚未接入。
“导出游戏”窗口选择启动场景、额外场景和新输出位置，提供 “导出”、“导出并运行”。导出会结束 Play、提交编辑事务并保存作者场景，再后台构建资源和打包；窗口显示阶段、日志和失败原因，任务期间暂停修改操作。日常“保存”只保存作者内容。
macOS 默认导出 `.app`，资源放在 `Contents/Resources`，发布前完成本机临时签名与校验；其他目录形式也可导出。游戏直接运行导出包，不依赖 Cargo、开发目录或编辑器进程。正式输出必须尚不存在，完整候选通过后才整体发布。对外分发的证书签名、公证与安装器尚未接入。

项目、产品、截图、报告和导出目的地的相对路径以调用目录为基准；作者场景路径统一限制在项目的 `assets/` 内，打开、保存和导出都会校验。公共启动实现位于 `tools/launch/launch.py`，只使用 Python 3 标准库。

| 配置 | 用途与取舍 |
| --- | --- |
| Debug | 编辑开发；本仓库代码保留完整调试信息、断言、溢出检查与增量缓存，第三方依赖关闭调试信息 |
| Release | 游戏运行与分发；优化等级 3、Thin LTO、剥离符号、关闭增量编译，保留默认并行 codegen 与 panic unwind |

测试继承开发配置；配置统一在根 `Cargo.toml`，选项含义见 [Cargo Profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)。`target/` 包含缓存、测试和验证输出，不作为发布包。修改配置不会自动回收旧缓存；需要回收时可使用 Cargo 的清理命令。

作者资产及其 `.meta` 身份文件需要一起入库，包括 glTF 配套的 `.bin`、纹理和测试数据；`Cargo.lock` 也保留在版本控制中。
自有作者格式使用 `.scene`、`.prefab`、`.mesh`、`.material`，内容仍为 JSON。默认场景为 `demo/assets/main.scene`；metadata 在完整源文件名后追加 `.meta`，例如 `main.scene.meta`、`showcase.gltf.meta`，重命名时随源文件一起移动并保留 UUID。
可再生产品、截图、报告与诊断输出放在 `target/`，导出包可放在 `target/packages/` 或根 `dist/`。根 `target/`、`products/`、`dist/`，以及构建锁、原子写入临时文件、GPU 捕获包和 Python 缓存由 `.gitignore` 排除；运行证据保存在 `target/` 或 CI 附件，不放进文档目录或版本控制。

## 源码归属

| 路径 | 职责 |
| --- | --- |
| `engine/core` | 分类 UUID、摘要、数学入口、项目路径与原子发布 |
| `engine/scene` | 组件、稳定字段 Schema、ECS 场景、层级、复制与 Prefab |
| `engine/asset/data` | 网格、材质、纹理、场景产品与索引的 CPU 数据协议 |
| `engine/asset/runtime` | 只读产品加载、校验、依赖闭包、版本切换与缓存 |
| `engine/asset/pipeline` | 源目录与身份、导入构建、索引发布、打包与项目配置库 |
| `engine/runtime` | 独立 Play 世界、固定步、Rapier 物理、角色运动与动作消费 |
| `engine/application` | 窗口、输入采样与授权、宿主活动策略、egui 与帧提取 |
| `engine/render` | 帧协议、渲染线程、共享 Shader 和原生 Metal/Vulkan 后端 |
| `editor/core` | 编辑文档、事务、历史与保存点 |
| `editor/src` | 编辑器入口、会话、导出后台任务与各功能面板 |
| `app` | 独立游戏运行入口，包内资源相对可执行文件定位 |
| `tools/asset` | `comet-assetc` 命令行入口 |
| `tools/gpu` | 原生 GPU 能力检查；Metal 多队列、显存别名与硬件光追，Vulkan 设备/队列检查 |
| `tools/launch` | 编辑器启动准备、平台图标处理及对应 Python 测试 |
| `tools/verify` | 工程验收、模块边界与依赖审计 |
| `demo/assets` | 默认示例的作者资源与身份文件 |
| `tests/fixtures` | 按功能组织的 Prefab、glTF 测试项目 |
| 各包的 `tests/` | 面向公共 API 的集成测试；私有单元测试在 `src/` 所属模块内或相邻测试文件中 |

每个库在自己的 `Cargo.toml` 声明依赖。只有实际需要隔离的职责独立成包；不为尚未实现的功能创建空目录。
所有 workspace 包统一设置为不可发布，仅用于完整工程内部构建。
第三方库由 Cargo 管理，具体用途、声明位置、已接入与待接入状态见[架构与依赖说明](docs/architecture.md#第三方依赖)。

## 验证与资产工具

```sh
cargo test --workspace --locked
python3 tools/verify/verify.py                         # 格式、测试、Clippy、边界、依赖、构建和独立包验证
python3 tools/verify/verify.py --native --linux --windows # 本机 GPU/窗口、Release 游戏、Linux/Windows 交叉检查
cargo run -p comet-gpu-check --locked           # 原生 GPU 能力检查
cargo run -p comet-assetc --locked -- prepare demo --output target/products/demo
```

默认 `cargo build/test` 面向引擎库、编辑文档和资产工具；`--workspace` 包含编辑器、游戏和 GPU 工具，均为按功能组织的当前模块。
GitHub [CI](.github/workflows/ci.yml) 在 PR、`main/master` 推送及手动触发时在 Linux runner 执行原生 CPU/构建验证及 Windows MSVC 交叉检查，并准备 Release App 模板、验证编辑器导出和构建 Release 资产工具。日志、报告和源码快照保存为 7 天的运行附件；Metal、物理 GPU 与窗口验收使用本地命令，详见[开发与验证](docs/development.md)。
文档按职责保留三份：[架构与依赖](docs/architecture.md)、[开发与验证](docs/development.md)、[功能迁移清单](docs/migration.md)。
仓库协作规范见 [AGENTS.md](AGENTS.md)。

## Git 与协作配置

[.gitattributes](.gitattributes) 沿用旧仓库的 Git LFS 资产管理：模型二进制、纹理、字体和音频走 LFS；`.scene`、`.prefab`、`.mesh`、`.material`、`.meta` 和 `.gltf` 保留普通文本 diff，文本统一为 LF。需要安装 Git LFS；已有克隆可在仓库内执行：

```sh
git lfs install --local
git lfs pull
```

CI checkout 会拉取 LFS 文件。第三方库由 Cargo 和 `Cargo.lock` 管理，没有 Git submodule 初始化步骤。[.editorconfig](.editorconfig) 统一编辑器的 UTF-8、LF 和缩进；本机 IDE 配置与缓存由 `.gitignore` 排除。
