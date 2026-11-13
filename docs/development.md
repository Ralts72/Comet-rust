# 开发与验证

开发与验证按当前功能组织。结果写入 `target/verification/`，源码与工具链快照写入 `source-manifest.json`；运行日志、截图和快照不放进版本控制。启动方式和应用参数见 [README](../README.md)，模块与第三方依赖见[架构说明](architecture.md)。

## 开发环境

使用 `rust-toolchain.toml` 固定的 Rust 工具链及其 rustfmt、Clippy 组件；Cargo 依赖由 `Cargo.lock` 锁定。Python 工具只使用 Python 3 标准库。编译共用根 `target/`，并行配置由 `.cargo/config.toml` 管理。macOS 最低为 14.2，需要 Xcode Command Line Tools；`.cargo/config.toml` 与 Editor/App 的 Info.plist 使用同一最低版本。Windows 原生构建使用 MSVC 工具链和 Windows SDK；Windows/Linux 运行图形程序时需要系统 Vulkan loader 与驱动。声音输出分别使用 macOS CoreAudio、Windows WASAPI、Linux ALSA；Debian/Ubuntu 构建先安装 `pkg-config` 和 `libasound2-dev`，设备与权限需由目标环境提供。
二进制源资产使用 Git LFS，初次准备仓库时按 README 安装并拉取；不需要初始化 C++ 第三方子模块。

## 构建缓存维护

启动器与完整验收共用 `target/launch/slot-*/programs/`，一个槽同时保存匹配的 Editor、Release App、图标、许可和模板清单。调用方持有文件锁直到所有消费者退出；POSIX 编辑器还继承锁描述符，启动器提前退出也不会释放它正在使用的槽。连续构建优先复用编号最小的空闲槽，不再按内容 hash 新建永久目录；同时使用的会话各占一槽，退出后可复用。相同输入只校验并复用已有套件，变化或损坏时先构造并校验完整候选，再替换空闲槽；发布中断可从同槽 previous 恢复，遗留候选在下次取得该槽后回收。验收进程也持锁，防止运行中的导出与 GPU 检查被启动器覆盖。

Cargo 自身的 `deps`、`.fingerprint`、`build` 和 `incremental` 仍使用其标准 hash 布局，不由启动器重命名或逐次清理。持续开发期间定期检查占用；需要清理过期 Cargo 缓存时先等 Cargo、rustc 退出，保留当前依赖产物及近期缓存。旧的 `target/editor-apps/`、`target/app-templates/`、`target/windows-programs/` 不再新增，迁移时保留已有内容；确认没有旧编辑器会话使用后再显式清理。

资产产品、验收报告与截图、用户导出包和其他未知用途目录不纳入普通构建缓存清理。不要直接删除整个 `target/`；需要完全重建时再单独执行 Cargo 清理。 构建身份只注入两个程序入口，保持引擎库缓存独立；完整验收用真实 Cargo 检查比较身份变动及相同身份重复构建的 fresh 标记，防止无效重编译回归。日常入口仍只有 `start.sh`，缓存维护不增加另一套启动或构建模式。

## 验证命令

```sh
python3 tools/verify/verify.py
python3 tools/verify/verify.py --gpu
python3 tools/verify/verify.py --native --linux --windows
```

直接执行验证工具前确保 rustup 在 PATH 中。默认执行格式检查、全 workspace 测试、Clippy、模块边界、依赖审计、程序身份缓存复用、应用构建、资产构建和独立包校验。
`--gpu` 增加真实离屏渲染与原生 GPU 能力检查；`--native` 还打开和关闭窗口，验证 start.sh、Release App 模板与编辑器导出的独立包。窗口烟测提交 8 帧后自动退出。
`--smoke` 使用确定的 1/60 秒宿主帧时间，初始帧与宿主状态恢复帧为零；离屏 App 必须实际执行 7 个固定步。日常运行使用单调实时时间，烟测的确定时间不用于性能结论。
`--linux` 对全部模块交叉编译和 Clippy，并检查目标平台的边界及依赖；需要安装对应标准库目标。它不代表已经在 Linux 上执行 GPU 工作。`--windows` 同样检查 x86_64-pc-windows-msvc 的全部模块、测试源码、Clippy、边界和依赖；不进行 Windows 链接，也不执行 Windows 程序。先运行 `rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc` 准备目标标准库；在已有原生验证的平台上按需选择另一目标，避免重复相同检查。Windows 运行验证工具使用 `python tools/verify/verify.py`，启动由同一 Python 实现负责，不要求 POSIX Shell。

macOS 运行 `--linux` 前使用 `brew install pkgconf` 准备 pkg-config。验证工具的 `linux_sdk.py` 自动下载固定版本和 SHA256 的真实 Debian ALSA 开发/运行包，在 `target/sysroots/` 校验、解包并复用，提供目标头文件、ELF 库及 pkg-config 元数据；首次需要联网。损坏缓存不会被复用。此 SDK 仅用于 ALSA 交叉检查；Linux 原生链接、设备与程序运行仍需对应环境验收。

## 依赖与模块审计

```sh
cargo tree -p comet-render --locked
cargo tree -p comet-app --locked
python3 tools/verify/audit_dependencies.py
python3 tools/verify/check_boundaries.py
```

`cargo tree` 展示解析后的依赖和版本；平台条件决定实际选用的后端。审计结果写入 `target/verification/dependencies/`。
依赖审计拒绝 cc/cmake、相关 C/C++ 构建封装与非正式图形路线，并检查 Objective-C 异常 shim 未启用。
模块门禁还拒绝 Core 以外的正式包直接依赖 glam；模块边界约束：Scene 不依赖产品或 Store，Render 只依赖资产数据，App 不依赖编辑器、导入器和通用源图像解码库；Application 仅解码内置或包内 PNG 图标；Kira/CPAL 只允许在 Audio 输出包使用，Hound WAV 导入归 Pipeline，Runtime 只消费中立声音帧与 PCM 产品。
## 单元测试

- 不要过度编写单元测试。简单的调用接线、配置修改、类型透传、常量调整、薄适配层或直接字段映射，默认不新增单测。
- 仅当改动包含真正复杂的逻辑、重要分支或状态流转、高风险回归面，或用户明确要求时，才新增或更新单测。
- 对简单改动，优先使用类型检查、构建或定向命令/人工检查等与风险匹配的验证方式。

清理已有测试也遵循这一标准：删减重复接线、字段映射和自证断言，保留当前协议校验、事务发布、失败恢复、身份稳定与资源生命周期等关键行为覆盖；不以测试数量作为目标。

测试采用 Rust 常见的混合布局，参见 [Rust 官方测试组织说明](https://doc.rust-lang.org/book/ch11-03-test-organization.html)：私有单元测试放在 `src/` 的所属模块内，较大测试模块拆到相邻的 `模块名/tests.rs`，通过 `#[cfg(test)] mod tests;` 按默认模块路径接入。库公共 API 的集成测试放在各包 `tests/`，由 Cargo 发现入口；集成测试共用辅助模块时使用 `tests/common/mod.rs` 等子模块。包内共享准备代码可使用受 `#[cfg(test)]` 保护的 `test_support` 模块；不为测试扩大正式 API。模块门禁检查依赖与职责边界，不强制所有测试使用同一种物理布局。

## GitHub CI

[工作流](../.github/workflows/ci.yml) 仅运行 Linux，在 PR、`main/master` 推送及手动触发时执行；相同 PR 或分支的新运行会取消旧运行。使用 Ubuntu 24.04 x64，安装 `pkg-config` 与 `libasound2-dev`，覆盖 Linux 原生测试、Vulkan 后端编译与 Clippy、资产与独立包检查、Release 游戏和资产工具构建，并交叉检查 Windows MSVC 全工程。

CI 复用 `python3 tools/verify/verify.py --windows`，包括格式、workspace 测试、Clippy、模块边界、依赖审计、Debug Editor/Release App 模板准备、编辑器后台导出及产品包隔离检查。Rust 版本、rustfmt 与 Clippy 由 `rust-toolchain.toml` 自动安装，Cargo 命令使用锁文件；checkout 拉取 Git LFS 资产，不初始化 C++ 第三方子模块。首次冷编译的测试、Clippy 与应用构建各允许 15 分钟，其余检查默认 5 分钟；单个 CI job 最长 30 分钟。

依赖缓存按平台、工具链和 Cargo 配置区分，只在仓库默认分支保存，PR 可以读取已有缓存。缓存不保留 workspace 自身产物、增量编译目录、资产产品或验证临时项目；CI 关闭增量编译，本地构建配置保持原有用途。日志、报告、依赖审计与源码快照在失败时也尝试上传，保留 7 天，不上传整个 `target/` 或游戏包。外部 Actions 固定到完整 commit SHA。

托管 CI 不执行 `--gpu` 或 `--native`；此处的 Vulkan 检查覆盖编译和 CPU 行为，Metal、物理 GPU、呈现、多队列和显存别名需在有对应设备的环境验收。声音 CPU 回归不要求设备；托管 CI 不作为实际出声验收。Runner 标签和架构以 [GitHub 官方说明](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)为准。

## 内容、编辑与资产

按改动风险选择下列现有回归与流程验证。共用准备代码复用已有 test_support 与 tests/fixtures，不为测试扩大正式接口；完整清单以实际测试输出为准。

| 验证边界 | 主要覆盖 | 入口 |
| --- | --- | --- |
| 文件读取预算 | 超限元数据在读取前拒绝、精确上限、读取中增长、I/O 失败和预算溢出 | [Core 公共接口测试](../engine/core/tests/storage.rs)、[Core 读取单元测试](../engine/core/src/storage.rs) |
| 文件摘要与模板 | 文件摘要与字节协议一致、短读/中断/I/O 失败；模板输入变化与复制结果不符时拒绝发布 | [Core 摘要测试](../engine/core/tests/storage.rs)、[Core 流式测试](../engine/core/src/storage.rs)、[模板测试](../engine/asset/pipeline/src/template/tests.rs)、[打包测试](../engine/asset/pipeline/tests/package.rs) |
| 场景与引用协议 | 当前格式校验、实例词法作用域、复制重映射、删除清理、候选失败保持 | [Scene 测试](../engine/scene/tests/)、[文档协议回归](../editor/core/tests/protocols.rs) |
| 作者文档与历史 | 组件及字段共用候选校验；多选、层级、作者/直接实例混合批量放置的世界间距、展开根精度、份间引用隔离及原子失败、Prefab 逐层解除关联及引用作用域、共用子树捕获与独立发布、剪贴板快照及跨世界引用边界、保存点与 undo/redo；失败保留当前预览 | [Editor Core 测试](../editor/core/tests/) |
| 运行世界 | 固定步与输入、物理/角色、脚本意图事务、暂停/重开/换关、具名动画事件的循环/重播/预算与回调失败隔离、双关六目标/物理门/跨关进度与作者隔离 | [Runtime 测试](../engine/runtime/tests/)、[App 会话测试](../app/src/session/)、[双关作者隔离回归](../editor/src/session/tests/character.rs) |
| 资源身份与产品 | 写入前拒绝路径重叠及资源根越界、合法目录别名保持身份和标签、metadata 稳定、身份预留碰撞与失败重试、源根与衍生产品区分、完整产品编码预算、导入失败与索引原子发布、依赖类型（含禁用绑定/隐藏模型/外部引用）、闭包及标签剥离 | [Pipeline 测试](../engine/asset/pipeline/tests/)、[构建发布回归](../engine/asset/pipeline/src/build.rs)、[Store 测试](../engine/asset/runtime/tests/) |
| 后台接纳与项目切换 | 进度先于结果、一次性交付与所有者回收；世界/revision 守卫、取消及旧候选拒绝、错误和退出 | [Job 测试](../editor/src/jobs.rs)、[会话测试](../editor/src/session/tests/)、[项目加载测试](../editor/src/project/loading/tests.rs) |
| 编辑器交互 | 真实 egui 输入下的多选、Gizmo、吸附、Inspector、拖动事务、焦点和模态隔离 | [UI 测试](../editor/src/ui/tests/)、[Inspector 测试](../editor/src/ui/inspector/) |
| 项目与文档流程 | 路径边界、身份预留、排他创建、保存失败重试、未保存提示与 Play 隔离 | [文档流程测试](../editor/src/documents/tests/)、[Core 路径与发布测试](../engine/core/tests/) |
| 帧与原生输出 | 场景/索引缓存身份、在途产品保活、静态及蒙皮变换、拾取、音频输出和线程关闭 | [Application 测试](../engine/application/src/)、[Render 测试](../engine/render/src/)、[Audio 测试](../engine/audio/src/) |
| 独立程序与启动 | Editor/App 配对、缓存槽复用与恢复、程序图标、同名许可拒绝与候选清理、产品搬移后运行、包外存储与设置 | [启动工具测试](../tools/launch/tests/)、[打包回归](../engine/asset/pipeline/tests/package.rs)、[完整验收](../tools/verify/verify.py)、[App 测试](../app/src/) |

资产导入、保存、打包、破坏性输入和恢复测试只写临时项目，不修改 demo 作者源。模型与声音复用正式资产或既有 fixtures，修改输入时先复制；自动测试不访问玩家存档、设置或最近项目记录。

Scene 与 Editor Core 负责内容规则和历史；Editor 的组件命令校验所选产品，validation.rs 统一作者内容与保留历史的预检，其[回归](../editor/src/validation/tests.rs)验证当前快照返回、历史脚本检查与失败保持。组件批量回归覆盖后续目标失败、重复/失效目标、专用引用与依赖清理、通用引用保持和保存点/redo；实际 egui 输入覆盖缺失项补齐、资源草稿取消及作者/Prefab/Play/模态边界。字段、组件增删与插入命令始终使用当前 Scene 格式，不推算最低版本或执行旧格式升级。失败不能发布半个组件、改写保存点或清除已有有效手势。

Gizmo 回归重点保留世界/局部坐标、父子归并、镜像及非均匀缩放、奇异/剪切/极端尺度拒绝、零位移与回到初始值、Escape、失焦、模态和世界切换。几何、法线及蒙皮的 CPU 结果另与真实 GPU 图像比较；合成输入和窗口烟测不替代物理鼠标操作与目标桌面验收。

Inspector 回归按行为边界组织，共同测试的场景准备归 `group/fixtures.rs`，帧输入与控件定位归 `group/harness.rs`，测试用例不互相提供共享设施。

| 层次 | 重点约束 |
| --- | --- |
| Editor Core | 原子候选、字段基线、显式覆盖/继承、版本与保存点恢复；批量恢复的失败、redo 与旧声明清理 |
| Editor 命令 | 全部目标及产品类型/兼容性校验、控制器与片段的联合候选、蒙皮替换保留动画实体绑定、词法身份转换、脚本参数保护；无关错误不阻止修复，失败保留有效预览 |
| egui 输入 | 组件交集、多值与同值统一、局部轴/其他配置保持、精度与焦点、Esc、单条撤销及 Play/模态只读；菜单打开/取消不产生历史 |
| 缓存与资产 | 空闲菜单只消费元数据，产品 revision 后失效；源文件、metadata、未选中覆盖与引用身份保持 |
| 原生离屏 | 实体及蒙皮实际绘制、共同控件与资源菜单、保存点/历史不变；GPU 图像与 JSON 记录相互对应 |

`--gpu` / `--native` 生成 `target/verification/editor-common-properties.json` / `.png`，覆盖 TRS、标量、混合开关及网格/材质/音频/脚本菜单；`editor-common-animation-clips.json` / `.png` 绘制两个完整骨架实例与共同控制器/片段菜单。合成输入、离屏图像和窗口烟测分别记录，人工输入与 Windows/Linux 目标设备仍需单独验收。

`editor-components.json` / `.png` 绘制两个作者模型及单选/多选共用的组件结构入口，检查空闲绘制不改变作者内容、历史或选择。资源表单和增删行为由同一功能的 egui 输入回归验证，不为组件字段映射分别添加测试。

内容与运行规则见[架构约定](architecture.md)，具体操作见 [README](../README.md)，尚未迁移的完整能力见[迁移清单](migration.md)。导航、动画、声音、脚本和检查点的专项验证在下文维护。

场景进入 App、Play、换关或正式导出时，统一通过 `AssetStore::scene_resources` 准备完整只读资源；作者保存、历史和导航工作流使用对应按需入口。新增资源类型时同步维护 Asset Data 的 `SceneResources`、Asset Runtime 的加载规则与 Runtime 消费，导出不能另写一套资源拼装。Editor 侧资源切换统一走 `session/assets.rs`：后台任务不直接改主线程 Store；异步结果需匹配文档版本，缓存加载结果需匹配索引 Arc 身份。导出已发布与编辑器已接纳分别记录，拒绝接纳时保留包且不自动运行。验证优先沿这些已有行为入口覆盖失败与候选接纳，不为集合字段映射新增单测。

## 项目浏览与本机记录

“文件”菜单的新建/打开表单可直接输入路径，也可打开内置 egui 浏览器；新建选择父目录时保留项目名。目录页支持跳转、刷新、主目录和 Windows 驱动器，作者文件页限制在所选项目的 assets 内。单次只枚举一层，最多扫描 4096 项（包含过滤项），超限时应进入更具体的目录。选中只回填表单；取消浏览不切换项目、不等待后台读取。最终提交仍执行未保存确认、ProjectPaths 边界校验和后台候选加载。

普通 GUI 在“文件 → 最近项目”保存每个项目最后成功接纳的 Scene/Prefab，按规范化根路径去重置顶，最多十项。记录仅是本机偏好，不影响启动参数、项目 metadata 或导出包。其 v1 JSON 上限为 64 KiB，位置如下：

| 平台 | 记录文件 |
| --- | --- |
| macOS | `$HOME/Library/Application Support/Comet/editor/recent-projects.json` |
| Windows | `%APPDATA%/Comet/editor/recent-projects.json` |
| Linux | `$XDG_STATE_HOME/comet/editor/recent-projects.json`；变量缺失或为空时使用 `$HOME/.local/state/comet/editor/recent-projects.json` |

所用环境目录必须为绝对路径；显式配置相对路径或缺少必要变量时停用记录并提示，不回退到工作目录。记录文件为符号链接、格式损坏、版本未知或超限时同样停用，保留原文件；写入失败保留旧列表，项目本身仍正常打开。非 UTF-8 项目可用浏览器的原生路径打开，记录时给出非致命提示。列表加载不扫描项目目录，失效条目可手动移除，点击后由正常加载流程报告错误。

Editor 构造时默认禁用记录；headless、smoke、CLI 导出和普通测试不访问真实用户状态。记录回归使用独立临时文件，覆盖路径规则、格式/大小/符号链接拒绝、原子更新失败、最后文档更新和移除失效项目。目录及合成 UI 回归覆盖有界枚举、最新请求胜出、取消不等待、assets 越界拒绝、原生路径回填、新建父目录保名，以及成功接纳后记录失败仍保留新会话。三个平台的实际目录交互仍需分别人工验收。

启动构建始终从当前作者源生成完整产品候选。既有产品索引只用于变更统计，非当前结构或损坏缓存不阻塞重建；失败保留旧索引和 metadata。Runtime 不读取历史格式，版本统一固定为 1；修改作者结构时直接同步源码、示例和 fixtures。

## 编辑器停靠布局

拖动面板标签到目标边缘可分割布局，拖到标签栏或中心可合为标签组；拖动分隔线分别调整左右宽度和上下高度。检查层级、属性、项目和场景视口均能改变尺寸，属性编辑、视口拾取及 Gizmo 使用调整后的矩形。面板关闭后在“视图”重新打开，或选择“恢复默认布局”。“文件”提供新建/打开/最近项目，“编辑”提供撤销/重做，工具栏保留常用入口；空间不足时可横向滚动工具栏。

普通 GUI 使用上表同目录下的 workspace.json，跨项目保留布局；重启检查分割比例、标签组合及可见性恢复。文件过大、版本未知或拓扑非法时，默认布局仍可使用，坏文件保持，诊断位于“视图”。显式恢复默认布局会允许重新保存；文件写入失败不会拒绝场景或项目接纳。headless、smoke、导出及测试默认不访问此文件，验证读写使用临时目录。

自动回归覆盖真实三条分隔线拖动、标签重排、过期拾取拒绝、隐藏/恢复场景、已有编辑预览的提交边界、布局恢复和坏候选保护；完整 --native 验收显式执行调整布局后的 Metal 绘制并保存 editor-workspace.png。CPU 行为、GPU 离屏、真实窗口及三平台人工交互分别记录。当前只支持主窗口内停靠，独立浮动/多系统窗口、完整旧面板仍待迁移。

## 视口相机

在 Edit 的“投影：3D／2D”菜单切换“3D 透视／2D 正交”。3D 右键环绕，中键平移、滚轮缩放；2D 保持正面方向，滚轮改变可见高度，返回 3D 恢复原环绕方向。视口拥有鼠标或焦点时按 F（无修饰键），或点击“聚焦选中 F”，检查作者容器、Prefab、多选和当前蒙皮姿态均完整入镜；没有可绘制后代的实体使用自身枢轴。“聚焦全部”按全场景几何取景。改变面板宽高后仍应正确适配，浏览不改变作者状态、选择或历史；输入框、编辑预览、拖动、失活、后台和模态应阻止聚焦，Play 的投影/聚焦控件禁用。

CPU 回归覆盖两类投影下的边界适配、焦平面重投影和非法输入保持；正交平行射线、点选/框选裁剪一致性；环绕/平移/缩放与输入授权；子树及骨骼变换、当前蒙皮姿态、无关坏产品隔离和空实体枢轴；实际按钮/快捷键及作者隔离。完整 `--gpu` / `--native` 显式运行正交相机聚焦离屏测试，保存 `editor-camera.png` 和报告；已有视口、Gizmo 与原生窗口验收继续覆盖透视路径。Windows/Linux Vulkan 实机与各平台人工鼠标交互分别记录。

## 层级浏览

在“层级”点击箭头或双击名称展开/折叠，检查“全部展开”“全部折叠”和“定位选中”。搜索名称或 UUID（忽略大小写）应只显示匹配项与祖先路径，清除搜索恢复原折叠状态；搜索框内 Escape 清除条件并退出输入。折叠和过滤不清除选择，隐藏选择有数量提示。视口选择、新建子实体及粘贴后的主选应自动展开祖先并滚入视口；目标不在过滤结果中时清除条件。Shift 范围按全部展开/过滤行计算，包含屏幕外行，不包含折叠或过滤掉的后代；Ctrl/Cmd-A 仍全选场景，删除/复制仍处理完整子树。

CPU 回归覆盖不可变索引/投影复用、跨世界重置、UUID 与名称筛选、深层级迭代、2000 节点视口行绘制、选中定位、过滤范围及折叠状态恢复，核对作者内容、历史和磁盘未被浏览操作修改。完整 `--gpu` / `--native` 显式运行层级离屏测试，保存 `editor-hierarchy-browser.png` 与报告，核对匹配祖先路径、作者状态和实际场景绘制。三平台鼠标、键盘与窗口人工交互分别验收。

## 项目资源浏览

在“项目”中展开目录与模型子资产，用“搜索文件...”查找名称或相对路径，再按类型筛选。目录名称匹配会保留其后代，子资产匹配会显示所属源；清空条件恢复原来的展开状态。新文件或外部修改需“刷新资源”成功后接纳，失败保持当前资源。场景双击打开，原生 Prefab 通过“编辑源”打开，均复用未保存确认。

在 Edit 中拖动模型或 Prefab 到场景画布，松开在鼠标对应的相机目标焦平面放置资源原点，落点显示十字，并选择新实例；相对源坐标保持，物理根不会额外套父实体。双击和“添加到场景”按钮仍保留源坐标。撤销/重做应整份处理，作者源与 metadata 不变。拖到工具栏或其他面板不放置；Escape、失活、布局调整、模态窗口、文档/索引变化或 Play 应取消拖拽，恢复后不补发。搜索、子资产选择及资源刷新不增加文档历史。

合成 UI 回归覆盖目录与子资产搜索、类型组合、刷新保持条件、多根及嵌套资源的鼠标位置放置/撤销、位移与属性/Gizmo 覆盖坐标换算，以及失效上下文。完整 `--gpu` / `--native` 验收显式运行资源树离屏测试，保存带有已放置物理实例的 `editor-resource-tree.png` 与报告；GPU 绘制与三平台实际鼠标交互分别验收。文件操作、未构建资源及空目录尚未实现。

## 离线导航开发

运行 `./start.sh --scene navigation.scene` 可打开隔墙巡逻示例。非玩家胶囊 NPC 在 A/B 实体标记间绕行，到点等待 1 秒；WASD 靠近至水平距离小于 6 米且高度差在 2 米内，触发追逐，远离超过 11 米后返回 A 点再恢复巡逻。脚本 `npc_patrol.rhai` 的 player/patrol_a/patrol_b 为实体参数，detect_distance/forget_distance/wait_seconds 控制检测、忘记距离和等待；Edit 中移动标记可调整路线，不需重烘焙。暂停/单步不跳过等待，R 重新初始化脚本和运行意图，Stop 保留作者文档。

地面与墙使用真实 Fixed Box 碰撞，玩家仍用原有输入。移动固定几何或修改区域参数后，在区域属性面板指定 assets 内的 `.nav` 路径并重新烘焙，候选接纳后保存作者文档。未烘焙或陈旧配置可以保留和撤销，Play、换关与全部导出场景必须通过导航预检；运行中不允许移动区域或 Fixed 刚体。示例的巡逻/追逐/返回由 Rhai 的 `this` 编排，底层提供运行命令和路径状态，没有把玩法状态写进 Scene 或导航产品。

CLI 适合处理已保存、已配置区域的原生作者文件：

```sh
cargo run -p comet-assetc --locked -- bake-navigation /path/to/project main.scene --products target/products/project --output navigation.nav
cargo test --locked -p comet-scene -p comet-runtime -p comet-editor-core
cargo test --locked -p comet-asset-data -p comet-asset-pipeline
```

作者文件与输出 `.nav` 路径相对项目 assets 解析；产品目录相对调用目录解析。命令发布或更新 `.nav` 及其稳定 `.meta`，完成产品准备并打印 AssetId，**不自动修改 Scene/Prefab 的 data 绑定**；回到编辑器刷新资源后选择导航数据并保存。编辑器的烘焙操作则在完成候选验证后以单个事务更新绑定，有变化时形成一条历史。文件已发布而后续准备或接纳失败时会保留完整资产，修复后可重试，不自动删除或回滚作者文件。

Scene/Core 回归检查当前格式、可空资产依赖、目标复制/删除/Prefab 重映射、区域根约束和候选失败保持。Runtime 回归用真实 Rapier 胶囊验证脚底采样、绕墙到达、速度单位、无路、玩家隔离、暂停/单步/R/换关，以及脚本和公开命令对固定姿态的原子拒绝。Data/Pipeline 回归覆盖真实 rerecast 烘焙、SDK 绕序归一化、输入摘要、资源预算、稳定 metadata、CLI 不改绑定，以及外部修改、身份冲突时的发布保持与修复重试；通用读取边界由 Core 回归集中检查大小、读取中增长、I/O 失败和预算溢出。编辑器回归覆盖两阶段守卫、候选 Store 与临时文档预检、历史保留及空闲缓存。

运行命令回归另检查 Point/Entity 目标、父级目标的世界位置、真实蹲伏脚底、Stop 持续抑制作者目标、五态与离网格零速、同回调及同固定步的状态快照、R/候选切换重置，以及坏目标使字段、物理、导航、普通日志和换关请求整批失败。真实示例长步测试覆盖巡逻折返、接近追逐、离开返回、移走目标后的不可达、暂停/单步及重开；玩法回归与单独的静态几何绕墙测试分别维护。

完整验收还应在临时 demo 重新烘焙并比较正式产品、保持源和 metadata 身份，拒绝额外场景的陈旧几何，再把包含导航的游戏包搬离作者与开发产品目录运行。CPU 绕墙模拟、独立包离屏渲染、人工 UI 和各平台 GPU/窗口分别记录；Windows/Linux 交叉检查不代表对应 GPU 已运行，针对性测试也不替代完整验收。

## 蒙皮模型与动画验证

运行 `./start.sh --scene animation.scene`，Play 后观察蒙皮模型随骨骼弯曲，再检查暂停、单步、R 重开和 Stop 后作者姿态保持。该示例为单 Clip 的 STEP/LINEAR/CUBICSPLINE TRS 播放；参数状态机与具名事件使用下述双关示例，根运动、分层、重定向及 IK 尚未接入；角色胶囊仍独立于蒙皮几何。协议与数据所有权见[蒙皮与动画](architecture.md#蒙皮与动画)。

属性面板“动画”可选同骨架片段或“不播放片段”、循环和播放速度，“骨骼绑定”按 Node 0/1/2 等名称选择目标或“未绑定”；源键集合不匹配时可显式“重建绑定表”。“蒙皮”可选择产品与同骨架的动画实体。直接 Prefab 配置保留已有字段覆盖，结构在源中修改；嵌套实例和 Play 配置只读，Play 显示采样时间。控件事务、未绑定重开修复和撤销回归与实际鼠标操作分别验收。

官方 SimpleSkin 作者源只保留在 demo/assets/models/simple_skin，测试复制该目录到临时项目，不另存长期模型或二进制副本。来源为 [Khronos glTF Sample Assets](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models/SimpleSkin)，锁定提交 8f492390ddc7a2ae221a1369ef560589cf8c70fb；Marco Hutter 的模型以 [CC0](https://github.com/KhronosGroup/glTF-Sample-Assets/blob/8f492390ddc7a2ae221a1369ef560589cf8c70fb/Models/SimpleSkin/README.md#legal) 发布。原 glTF 与四份 buffer 共 3242 字节，保持上游内容。

资产回归覆盖稳定子身份、skin-local joint 顺序、逆绑定、四权重、骨架域、关键帧/时间/四元数、预算和不支持格式拒绝；失败保持索引与 metadata，包搬移并移走作者源后仍可读取完整产品。强引用有效时，明确未绑定及缺失/过期源键可构建并打开修复，原映射不会自动改写；“重建绑定表”是显式的一条可撤销操作。悬空 Local 强引用与失效 Prefab override 仍拒绝，不隐式清理或迁移。新增曲线回归检查非单位/零切线、非单位关键帧间隔、保留旋转符号、退化四元数失败及暂停/单步/重开。完整验收在临时项目中将同一 SimpleSkin 副本改为 CUBICSPLINE，导出搬移后移除作者源和开发产品，校验 Clip v1；`--gpu` / `--native` 另比较包内实际播放姿态与独立 Hermite 计算，并保存本次截图。Runtime、作者事务、真实着色与变形拾取分别验证；编译、CPU 数值、GPU 离屏、实际鼠标操作及三个平台运行不互相替代。

角色控制器验证使用 `./start.sh --scene character.scene`：观察待机、WASD 走路、Shift 跑步、Ctrl 蹲伏/蹲行、Space 起跳、下落和着地；靠墙持续前进应按实际速度停步，空中与原地不播放脚步。脚步使用本工程合成提示音，模型与动作来自旧项目保留的 Quaternius GLB，详见 [资源说明](../demo/ASSET-CREDITS.txt)。两关共用 `animations/character.ani`，通过实例覆盖选择控制器，不复制骨架。参数与状态图目前编辑 JSON；属性面板负责控制器选择、骨骼和角色驱动绑定，支持撤销、保存及运行只读状态。

CPU 回归复用真实双关和产品，覆盖走跑跳蹲、碰墙、脚底位置、脚步意图、收集/闸门/往返/检查点与作者隔离；数值回归覆盖阶段连续、一维混合、rest 回退、平滑、中断过渡、结束转换和坏参数整批拒绝。独立包检查控制器及七个动作的完整依赖、65 joints 蒙皮和 67 节点绑定；Metal GPU 与窗口截图验证实际消费，人工操作、试听以及 Linux/Windows GPU 仍分别待验收。

## 战斗样例验证

战斗回归使用 `./start.sh --scene combat.scene`：靠近并朝向守卫按 J / 鼠标左键，比较挥空、背对和隔墙；攻击过程移动被锁定，按住不连发。守卫绕墙接近，血量由攻击事件扣除，受击声音与短暂缩放同时反馈，击败守卫后 0.4 秒缩小、1.2 秒移除整个角色；胜败后 R 重开，Stop 保留作者场景。血量/伤害/距离/冷却沿用旧样例的 120/90、35/12、1.9/1.7、0.8/1.15，数值来自 `data/combat.data.json`，脚本参数 stats 指向该数据源，当前属性仍由脚本维护。骨架和八个动作共享唯一人形源，导航是独立 `combat.nav`，修改固定碰撞几何后通过既有导航烘焙流程重建。

自动回归在临时项目使用真实控制器、导航、物理和脚本，覆盖阶段命中、动作锁定、不连发、朝向/遮挡过滤、追逐、胜败、守卫消退后的碰撞/导航/动画/声音清理、重开及作者隔离。另检查物理/世界坐标查询的回调快照、传送后刷新、查询预算、坏方向与世界句柄、输入别名和命令整批拒绝。完整验收导出包含战斗的场景并集，移走作者目录后检查包内场景、事件、导航、HUD 与实际窗口截图。CPU、Metal、Linux/Windows 编译与目标设备运行分别记录；粒子、描边、倒地动作、手柄、人工交互及试听仍需迁移或验收。

## 音频开发与验证

运行 `./start.sh --scene audio.scene`，背景循环使用 `audio/ambient_loop.wav`，`scripts/audio_demo.rhai` 在启动时播放 `audio/play_chime.wav`，E 重播、Space 停止提示音。`./start.sh --scene spatial_audio.scene` 使用同一提示音循环，声源以 3 米半径移动，E 开关声音；监听器取当前成功视图的相机，可旋转视角比较声像与距离变化。R 新建运行世界并重新初始化；暂停、单步和音频源面板按钮用于检查播放意图，暂停时新播放保持待播。

音频作者源为 PCM16 WAV，只接收单/双声道、8–192 kHz；把 `.wav` 与其 `.meta` 一起维护。Pipeline 用 Hound 解码为 AudioData v1 的共享 i16 PCM，独立游戏只读产品。Scene、ContentSource、产品与索引仅支持[当前协议](architecture.md)；2D 与空间音效共用完整的当前配置字段，不进行旧格式补值或版本升级。重新构建保留 metadata 身份。

```sh
cargo test --locked -p comet-audio -p comet-runtime
cargo test --locked -p comet-scene -p comet-editor-core -p comet-asset-data -p comet-asset-pipeline
```

属性面板“音频源”可为作者实体添加/移除组件，选择已构建音频并编辑音量、循环、自动播放、空间开关与近远距离；直接 Prefab 使用源 UUID 字段覆盖，组件结构在源中维护。距离为世界米数，要求有限的 0≤近距<远距，跨度不能下溢为次正规数；实体缩放不改变距离参数。Play 配置只读，“播放声音／停止声音”仅修改当前运行世界。编辑器回归检查连续音量/距离事务、取消与撤销恢复、最小覆盖、错误资源拒绝与后台/模态守卫，实际鼠标操作单独验收。

Audio 包使用 Kira 官方 MockBackend 检查控制状态，并由测试专用 backend 接收官方 Renderer 生成实际离线 PCM，覆盖有限非零输出、声道和采样率转换、音量、循环、暂停、停止、同代次幂等及设备错误。空间声覆盖左右声像、监听旋转、近/中/远衰减、源移动及 stereo downmix；Kira 使用固定分贝线性曲线，中点幅度约 0.0316，到远距后静音。适配内以监听器为坐标原点，先用 f64 做差，检查 f32 中心与耳位距离运算的余量；可表示的大世界平移保持结果，危险相对跨度须在设备修改前拒绝，不允许截断后继续。

帧回归检查 64 voice、唯一 clip 总样本预算及冲突产品身份；单 clip 上限 8,388,608 样本，总上限 16,777,216 样本按唯一 clip 计一次。输出缓存和 Runtime 批次共享 Arc；仅有空间源且 Scene revision 改变时提取世界位置，位置未变复用批次，移动和相机旋转不重播同代声音或重扫 PCM。

Runtime 与脚本回归检查独立世界、自动播放、显式重播代次、停止与音量、暂停/单步新代等待恢复、重开及候选换关。候选仅准备资源与意图，不打开设备或发声；坏目标、音量或代次溢出必须使同回调的字段、物理、导航、声音、普通日志及换关请求一起失败。Scene/Core/Pipeline 分别覆盖当前格式、直接 Prefab 字段覆盖、历史、音频类型、WAV 格式与 metadata 保留。声音自然结束由输出消费同一代次，运行意图不会导致逐帧重播。

Host 在最终视图提取成功后核对 World，并从相机填入监听姿态再提交声音，失败视图与旧世界不能打开设备或发布新声音；停止 Play、换世界及退出会停止旧句柄。没有设备或空间数字范围不安全时报告 Unavailable 并继续图形流程，错误只提示一次；初次空闲不打开设备。报告的 `audio` 保存设备状态、监听器、空间源与相对位置、代次、待播数量及实际播放位置，停止后保留最后错误和观测。完整包验收移走作者 WAV 和开发产品，逐字节对照包内 PCM，并运行搬移后的游戏；2D 与空间音效分别验证。

headless 与 smoke 同时启用时，GPU 帧可能先于设备首次回调完成；退出前最多等待 500 ms，每 2 ms 读取真实设备报告，不推进模拟或重放声音意图。验收仍要求实际播放位置大于零，等待超时也判失败，不能仅凭 Ready 和已创建句柄认定回调发生。

离线 PCM、空间声像/衰减计算、设备回调进度、GPU/窗口与人工试听分别记录。纯 PCM 非零、交叉编译或句柄推进不代表实际已听到声音；Windows/Linux 原生设备与三平台人工试听仍待验收。流式播放、遮挡、多监听器及完整旧关卡的脚步/战斗声音尚未迁移。

## 渲染、脚本入口与独立包

Render 单元测试编译共享 Shader 的两种输出，检查帧布局、CPU 拾取、背压、失败反馈与关闭路径。

Application 的提取缓存回归检查同一场景/索引 Arc 命中、选择及后代高亮更新、新场景和产品版本、相同摘要的新 Store 隔离、旧批次保活，以及坏层级、变换或资产候选失败后的重试；宿主帧准备另检查复用 Draw 时，相机、世界、视口和拾取仍使用当前请求。Asset Runtime 的公共测试检查索引身份、驻留产品复用、淘汰后消费者有效及失败刷新不改变已接纳状态；索引未变时也必须完整验证磁盘候选。

CPU 提取计时诊断默认以 ignored 测试跳过，按需手动执行：

```sh
cargo test --locked -p comet-application extract::tests::extraction_cache_cpu_profile -- --ignored --exact --nocapture
```

诊断使用预热的产品 Store，分别测完整提取、静态命中和仅选择变化；不包含 GPU、Gizmo、UI 或物理工作，也不测总帧率。计时结果以本次实际运行和构建配置为准，不能由缓存正确性回归推导性能数字。

GPU 验收检查 9 次场景绘制（原示例 5 次，加上落体和角色的两组多材质网格）、专用渲染线程、零尺寸不呈现、目标代次变化、PNG 读回和窗口 resize。产品独立 App 的离屏烟测还检查角色确实受到重力推进。
原生编辑器测试使用 `start.sh`；它准备并启动 Debug Editor，同时固化 Release App。游戏测试启动编辑器导出的包，不再提供独立游戏构建启动脚本。

默认验证复制 demo 到临时项目，增加一个独立的空场景，通过编辑器同一导出实现打包主场景、空场景、关节、换关、导航和声音示例，核对实际场景名单及产品依赖闭包，并验证模板失配和已有输出拒绝。macOS 在临时 `.app` 完成签名与校验，Linux 验证目录包。验收将包搬移，移除本次临时作者源和开发产品，从空工作目录检查产品并启动包内游戏，验证没有作者源与 cwd 依赖。
另通过真实资产 CLI 创建全新空项目，经 Editor 导出一个入口 Scene 产品；搬移包并移除作者/开发产品后再次检查和展开。GPU 验收还运行空 Editor 与空 App，验证零实体绘制、渲染线程和有效 PNG；空场景不套用 demo 的角色或颜色丰富度断言。
可执行文件从 Cargo JSON 产物消息获取，兼容 Cargo 的 target 目录和构建目标配置，不手工拼接 Debug/Release 可执行文件路径。

缩放数值验收使用生成在 `target/verification` 的独立三角形项目：小缩放配大网格、大缩放配小网格、镜像、subnormal 法线、旋转非均匀缩放，以及极大/极小 glTF 缺失法线的完整导入。世界几何与法线方向保持相同，默认 CPU 验收只真实构建及检查产品；`--gpu` / `--native` 使用已准备 Release App 离屏运行，要求 Draw=1、中心区域有三角形，并比较等价几何的像素覆盖与颜色。测试数据不复制默认 demo，也不进入版本控制；这属于实际本机 GPU 回归，不替代目标平台窗口验收。

Application 的内容恢复回归直接执行生产 CPU 帧准备，检查一次恢复、默认错误传播、fallback 失败及字体 delta 在关闭/失败/恢复分支的所有权。Editor 回归保留作者事务、历史、文件和选择，停止出错 Play 并退休旧拾取。`--gpu` / `--native` 另运行需本机 GPU 的 ignored 回归：实际进入无法渲染的 Play，首帧仍提交完整 UI，随后返回正常作者场景，报告与截图在 `target/verification/editor-view-recovery.*`。默认 CPU 测试跳过此 GPU 用例。 同时执行局部旋转离屏用例，实际绘制选中模型和局部环，截图与报告保存为 editor-local-rotation.*，检查作者文件、选择和保存状态保持。GPU 用例要求精确执行一项测试并更新报告，不能把过滤到零测试或旧报告当作新证据。

## 原生 GPU 能力

```sh
cargo run -p comet-gpu-check --locked
MTL_CAPTURE_ENABLED=1 cargo run -p comet-gpu-check --locked -- --capture --output target/diagnostics/gpu-capture
```

Metal 工具执行两队列提交与事件同步/读回、同一 placement heap offset 的显存别名、硬件光追三角形命中/未命中及 refit 后读回。命令缓冲时间区间重叠不作为内核并行或整帧性能提升的证明。
抓帧目录必须没有旧 trace，工具不会覆盖已有 GPU trace。Vulkan 工具目前验证原生 loader、设备与队列创建，不宣称已有 Metal 同等的执行覆盖。
这些是独立能力检查；正式 Render 的多队列调度、显存寿命规划与光追 Pass 尚未接入。

## 平台与交互边界

界面变更按[中文术语对照](architecture.md#中文界面与旧版对照)核对旧版菜单、组件、属性和说明，检查普通 Inspector 与实例覆盖列表的一致性；同步更新既有交互测试中的控件定位文字。真实窗口或离屏截图另检查中文、单位和较长按钮的显示，CPU 交互测试通过不代表文字未被裁剪。持久字段 ID、协议键和用户命名不随显示文案重命名。

macOS、Windows、Linux 均为正式支持目标，新增功能需要维护相同的持久协议、编辑语义、Play 行为和独立包能力。当前验证基线为 macOS Apple Silicon / Metal、Windows x64 / MSVC / Vulkan、Linux x64 / Ubuntu 24.04 / X11 / Vulkan；其他发行版、架构与原生 Wayland 尚未验收。CI runner 仍只使用 Linux。

| 平台 | 编译与静态检查 | CPU 测试 / 原生链接 | GPU、窗口、独立游戏实测 |
| --- | --- | --- | --- |
| macOS / Metal | 本机执行 | 本机执行 | 本机 --native 验收 |
| Windows / Vulkan | --windows；Linux CI 同样执行 | 需 Windows 原生环境验收 | 待 Windows 设备验收 |
| Linux / Vulkan | --linux；Linux CI 原生执行 | Linux CI 执行，结果以对应提交的 CI 为准 | 待 Linux GPU/桌面验收 |

每个功能分别记录三平台的实际证据；只有编译检查通过时，不能标记为完整的平台验收。通用功能同步推进，原生系统或硬件能力的差异要明确说明，不静默跳过。Windows EXE 图标写入及读回已接入，原生资源 API 和资源管理器显示等待 Windows 实机验收；Linux 桌面菜单入口和系统 IME 等尚未完成的接入继续按功能补齐。

macOS Metal 已有实际窗口和 GPU 运行；Windows/Linux 的原生 Vulkan 设备运行、Windows 原生链接、实体手柄、系统中文输入候选窗和物理鼠标 Gizmo 拖动仍需设备验收。合成 egui 拖动测试与真实鼠标操作分开记录。
烟测的零尺寸帧由程序主动注入，不代表已经验收系统最小化、跨屏或设备丢失。视口 Gizmo 支持世界轴平移、旋转，以及本地轴和统一缩放；移动和旋转共用世界/局部选择，Scale 使用独立局部仿射基。
任何未执行、环境不具备或检查失败的项目，必须保留状态，不用编译通过或另一平台结果代替。
当前发布物支持可搬移目录与 macOS ad-hoc 签名 `.app`；开发者证书签名、公证与安装器尚未实现。

## 游戏脚本开发

将 `.rhai` 源与生成的 `.meta` 一起保存到项目 `assets/`，刷新资源后为作者实体选择脚本并配置参数。Play 启动重新编译已接纳的源码产品，运行状态不写回作者文件；修改源码后停止、刷新、重新运行。语法/签名错误阻止候选接纳，回调执行错误则停止对应绑定，在“日志”查看实体、资源、步数和阶段。导出会校验所选场景依赖闭包，包内携带源码产品，不依赖开发目录。

```rhai
fn on_start(ctx) { this.distance = 0.0; }
fn on_fixed_update(ctx, dt) {
    let movement = ctx.param("speed") * dt;
    this.distance += movement;
    ctx.translate_local(ctx.entity(), movement, 0, 0);
    ctx.rotate_local(ctx.entity(), 0, 1, 0, dt);
}
fn on_stop(ctx) { ctx.log("运行结束"); }
```

示例需给非物理作者实体配置数值 `speed`；物理体使用 `teleport`、`set_velocity` 或 `impulse`，不能普通写 Transform。`this` 的状态仅属于当前绑定。触发回调 `on_trigger(ctx, other, entered)` 在传感器实体自己的脚本上调用，动作输入使用 `pressed/held/released`。全部命令延后到回调成功校验后发布，位置查询返回当前回调开始时的局部坐标，停止回调只允许查询与日志。接口、预算和协议见[脚本生命周期](architecture.md#脚本生命周期)。

脚本相关 CPU 回归可运行 `cargo test --locked -p comet-runtime -p comet-scene -p comet-editor-core -p comet-asset-pipeline -p comet-editor`。完整验收同时检查源码产品闭包、搬移后 App 的启停日志及无脚本错误；GPU/窗口结果按实际运行环境记录。

脚本变换读写以父级局部坐标为准：`rotation(entity)` 返回 xyzw，`scale(entity)` 返回 XYZ；`set_rotation` 接收近单位 xyzw，`rotate_local` 接收单位局部轴及弧度，`set_scale` 接收有限绝对值（可为零或负）。多次 `rotate_local` 依次后乘暂存旋转；同回调的 getter 始终读取入口快照，因此连续增量应直接排队。物理实体允许查询，旋转和缩放写入需由物理系统控制；停止回调只读。错误输入、物理目标或最终世界矩阵溢出会丢弃整批世界修改并隔离该绑定。

换关回归分别覆盖 Runtime 请求事务及旧请求拒绝、Editor 原文档/历史隔离和 App 包场景名单。`travel.scene` / `travel_second.scene` 复用已有模型，`next_scene` 是 UUID 文本参数，导出时需明确包含两关；`auto_after` 默认 0，只在验收临时副本设为正数。完整验收移走作者目录和构建缓存后，从导出包启动第一关，离屏运行实际进入第二关，核对新 WorldId、入关时间、生命周期日志和绘制结果。同步加载可能停顿，真实按键与目标平台窗口交互继续单独验收。

### 游戏数据表

将 `*.data.json` 与 `.meta` 一起保留在 assets，使用旧版 `fields` / `rows` JSON 结构；刷新资源后，在脚本参数里选择“数据”并绑定数据源。`ctx.data_get(ctx.param("stats"), "player")` 读取独立记录，`ctx.data_ids(...)` 列出排序 ID；缺失记录为空值，未设置数据、缺失/错类型引用或查询超限会明确失败，不能以默认数值掩盖。修改副本仅改变自己的脚本状态，当前属性、Buff、冷却及存档仍由项目规则管理。格式、限制、Scene1 及引用语义见[游戏数据表](architecture.md#游戏数据表)。

CSV 可保存为 `stats.csv`，例如 `id,health` 表头和 `player,120` 记录；同目录的 `stats.csv.schema.json` 写入 `{"fields":{"health":{"type":"integer","min":1}}}`。表头需要 id 和全部声明字段；字符串保留空白，嵌套 JSON 按 CSV 双引号转义，CSV 与 schema 各限 256 KiB。schema 随源提交，身份仅由 `stats.csv.meta` 保存；重命名时三者一起移动。普通 JSON 和 schema 文件不单独作为数据源扫描。格式和严格失败规则见架构说明，不接受旧 64 位资产 ID 的隐式转换。

在“项目”面板选择“数据”资源，查看“记录”“字段”、ID 与各列，只读窗口支持横纵滚动和长值悬停。修改源文件或 schema 后停止 Play 并“刷新资源”；失败保留旧预览，成功后重新选择读取新版本。预览不生成场景撤销项，也不把源文件中的临时内容提前显示为已接纳产品。

修改 demo 的 `data/combat.data.json` 中 player/guard 的 health、damage、range 或 cooldown，刷新资源并重新运行战斗，观察血量上限和实际命中；R 重开复用当前世界接纳的基础表，Stop 不修改数据源。自动回归覆盖严格协议、CSV 引号/多行/类型与预算、重复键/ID、类型闭包、Prefab 覆盖、schema 指纹/失败发布、搬移包、独立查询副本、真实战斗数值变化及预览刷新隔离。完整验收检查无作者源包中的规范化表、Scene1 参数、实际 HUD 和窗口，并显式运行数据表预览的 GPU 回归。完整旧数据消费者仍待迁移，人工交互与各平台 GPU 分别验收。

### 运行实体销毁

脚本调用 `ctx.destroy_entity(target)` 删除完整子树，使用 `ctx.exists(cached_entity)` 检查缓存引用；同一回调、同一固定步的其他回调仍能访问待删除目标，下一步查询反映删除。移除前的 `on_stop` 只允许查询和日志，不能再发世界命令。完整约束与资源退出边界见[运行实体销毁](architecture.md#运行实体销毁)。

`despawn` 回归检查重复/重叠请求、失败回调整批拒绝、暂停/单步、只读退出、句柄与参数清理、物理/关节/声音/HUD 退出及旧快照保活；既有动画和导航夹具覆盖初始化与步末移除、部分骨架及烘焙几何拒绝、动态障碍/代理释放及跟随目标消失。真实战斗回归检查击败后的实际子树退出和新 Play 恢复，收集回归检查外观子树移除与继续时不重生。验证用独立项目和资源，不删除示例作者文件。

### 收集示例与 HUD

运行 `./start.sh --scene collect.scene`，Play 后旁边的蒙皮模型停在初始姿态；用 WASD 走近金色目标，观察 HUD 从 0/1 变为 1/1、目标外观消失、蒙皮动作开始及一次提示音。等非循环动作结束、HUD 显示完成，再重新按 E 进入现有第二关；收集前、动作中或动作结束边沿按 E，以及一直按住 E，都不会跳关。退出再进入目标不重复计数或重播，R 重开恢复本次入关进度；正常新 Play 从空进度开始，Stop 不修改作者文档。导出须明确包含 collect、travel_second 及它返回的 travel 三个场景。collected 通过显式进度记录，继续时已收集目标外观在初始化结束后移除、动作停在初始姿态，不重播提示音，HUD 可直接接受新的 E；示例不替代旧多目标关卡。

脚本用 `ctx.set_hud(text)` / `ctx.clear_hud()` 维护自身拥有者的一条纯文字；HUD 随成功回调发布，沿游戏视口裁剪，不消费游戏输入。世界、条目和文字预算及失败语义见[游戏 HUD](architecture.md#游戏-hud)。运行报告在停止前保存 HUD 观测，再记录 on_stop，不能用 Stop 后的空列表冒充运行显示结果。

`demo_collect` CPU 回归读取正式场景与脚本，验证初始暂停、真实胶囊移动进入、非玩家过滤、退出/重入幂等、完整动作结束门槛、持续 E 与新按下、换关清理及暂停/重开/Stop；动画键直接复用唯一 SimpleSkin 源，声音使用已准备的小 PCM，导入、解码和设备输出分别验证。搬移包验收仅在临时项目副本把玩家放入 Sensor，让短帧运行观察实际 entered 回调、动作播放中的 1/1 文字、减少的目标绘制、共享 Prefab 实例节点的蒙皮姿态和真实 Cue 播放进度；正式作者起点仍在 Sensor 外，不加入自动收集参数，不复制模型或 WAV。文字形状/字体/裁剪、设备回调、三平台实际输入及人工试听分别记录。

单片段脚本控制使用 `ctx.play_animation(entity)`（包含从头重播）、`pause_animation`、`resume_animation`、`set_animation_speed` 和 `animation_state`。状态字段与重播发布时点见[蒙皮与动画](architecture.md#蒙皮与动画)。collect 的首次触发在第一个固定步采样后请求播放，下一步发布 0，七步 smoke 观测到五个推进步；短帧验收不声称已验证完整动作结束或真实换关，完整门槛由 CPU 场景回归验证。独立 `animation.scene` 保持无脚本自动播放，继续验证源曲线插值；两个场景共享同一批产品，不新增模型或音频副本。

### 检查点与显式进度

运行 collect 后按 F5 保存，走到其他位置或换关后按 F9 继续：角色从保存关卡入口开始，已收集目标外观保持移除，提示音和收集动作不重播。R 仍恢复本次入关基线，不能当作读取最近存档；新 Play 不自动继续。Editor 的单槽仅存在于当前 Play 内存，Stop 后清空；Prefab 预览不支持。F5/F9 可在全局暂停时使用，输入被文字控件、模态、后台任务或失焦阻挡时，需先释放再按。

导出 App 使用包内稳定 GameId，用户存档位于包外：

| 平台 | 根目录，后接 `GameId/checkpoint.json` 或 `GameId/settings.json` |
| --- | --- |
| macOS | 系统用户 `Application Support/Comet/games` |
| Windows | 系统 `Saved Games/Comet` |
| Linux | 绝对 `XDG_DATA_HOME/comet/games`，否则 `HOME/.local/share/comet/games` |

App 的 `--data-dir <目录>` 将存档与设置共同的根替换为显式目录，仍按 GameId 隔离，适合测试或自选数据位置；需要导出包，单独指定产品没有持久游戏身份。headless/smoke 默认使用内存槽，测试磁盘时必须指向临时目录，不读写真实用户数据。缺档读取不创建目录；坏、超限或外来身份的文件不删除，失败后原运行世界保持。包外路径及完整协议见[游戏检查点与进度](architecture.md#游戏检查点与进度)。

脚本用 `ctx.progress("collected", false)` 读取，缺失只返回默认值；`ctx.set_progress("collected", true)` 与收集反馈同回调提交，必要时可 `remove_progress`。值按场景和脚本拥有者隔离，只接受四种叶子类型，this、Entity 句柄及任意容器不进入存档。CPU 回归分别检查事务失败、跨关与入关基线、collect 的继续行为；App/Editor 再检查单槽、候选失败及输入隔离，实际磁盘回归使用临时数据目录。GameId 回归覆盖当前格式拒绝、创建后身份稳定、失败重试、配置竞态及旧面板不可替换身份；产品和配置搬移不更换身份。目标平台实际按键、磁盘权限与完整旧关卡仍分别验收。

### 独立 App 菜单与设置

导出并运行游戏后，按 Esc 打开菜单：返回游戏、保存进度、继续存档、设置或退出。打开和关闭的当帧都不会推进游戏或消费角色输入，关闭后下一帧恢复；原有 Runtime 暂停和宿主冻结状态不会被菜单改写。保存进度后留在菜单，继续存档成功才返回游戏；失败保留原运行和菜单提示。菜单打开时 F5/F9 不穿透，可使用对应按钮。

“设置”页只修改草稿；点击“应用”成功后生效并保存，停留当前页，未再修改时按钮禁用。失败保留当前值和草稿供重试；返回或无编辑时按 Esc 会放弃未应用内容回主菜单。音量手势编辑中的 Esc 先取消该次修改，IME 输入中的 Esc 不关闭菜单。主音量为有限 0～1、默认 1，统一影响 2D/空间音效，不改各源音量或重播声音；方向光阴影默认开启，关闭后仍保留方向光照。此偏好不代表旧版 MSAA、渲染比例、TAA 或完整画质设置已迁移，Editor 也不加载这份 App 偏好。

启动读取独立 settings.json，私有 v1 封装校验 GameId 和全部字段，最多 4 KiB；缺文件只使用默认值，不创建目录。坏、超限、未知版本或外来身份保留文件、使用默认设置并提示，应用也不会覆盖错误文件，需修复或移走后重试。设置与 checkpoint.json 共用同一 Store 的路径和原子写入边界，但文件独立；修改设置、R、换关或继续都不改变检查点的含义。存储与设置单元回归分别检查严格格式、大小上限、两槽隔离、坏文件/符号链接与失败保留；真实 egui 回归检查草稿、应用/返回、焦点和按键生命周期。Audio 的真实 Kira 离线 PCM 回归检查主音量比例及代次/缓存保持，原生设备进度和三平台人工试听继续单独验收，AudioReport.master_gain 只说明已接纳目标。
