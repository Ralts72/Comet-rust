//! Export orchestration owns background CPU work; document and UI mutation stay on the main thread.
use crate::jobs::Job;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_pipeline::{AppTemplate, PackageOutput, PackageRequest};
use comet_core::{AssetId, ProjectPaths};
use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    process::Child,
    sync::Arc,
};

use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::SourceCatalog;
pub struct ExportAction {
    pub entry: AssetId,
    pub additional: Vec<AssetId>,
    pub output: PathBuf,
    pub run: bool,
}
pub struct ExportIdentity {
    pub game_id: comet_core::GameId,
    pub document: crate::resources::DocumentVersion,
}
struct ExportJob {
    task: Job<(PackageOutput, crate::resources::ResourceCandidate)>,
    run: bool,
    paths: ProjectPaths,
    source: PathBuf,
}
pub(crate) struct CompletedExport {
    pub output: PackageOutput,
    pub run: bool,
    pub resources: crate::resources::ResourceCandidate,
    pub paths: ProjectPaths,
    pub source: PathBuf,
}
pub struct ExportController {
    pub open: bool,
    pub template: Option<AppTemplate>,
    pub template_error: Option<String>,
    entry: Option<AssetId>,
    additional: BTreeSet<AssetId>,
    output: String,
    job: Option<ExportJob>,
    children: Vec<Child>,
    logs: VecDeque<String>,
    stage: String,
    pub last_output: Option<PackageOutput>,
    failure: Option<String>,
}
impl ExportController {
    fn default_output() -> Result<String> {
        let name = if cfg!(target_os = "macos") {
            "CometGame.app"
        } else {
            "CometGame"
        };
        Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("workspace directory")?
            .join("target/exports")
            .join(name)
            .to_string_lossy()
            .into_owned())
    }
    pub fn new(entry: Option<AssetId>) -> Result<Self> {
        Ok(Self {
            open: false,
            template: None,
            template_error: None,
            entry,
            additional: BTreeSet::new(),
            output: Self::default_output()?,
            job: None,
            children: vec![],
            logs: VecDeque::new(),
            stage: "就绪".into(),
            last_output: None,
            failure: None,
        })
    }
    pub fn fail(&mut self, error: String) {
        self.stage = "导出失败".into();
        self.log(error.clone());
        self.failure = Some(error);
    }
    pub fn entry_scene(&self) -> Option<AssetId> {
        self.entry
    }
    pub fn select_scene(&mut self, entry: Option<AssetId>) {
        self.entry = entry;
        self.additional.clear();
        self.open = false;
        self.last_output = None;
        self.failure = None;
        self.stage = "就绪".into();
    }
    pub(crate) fn reset_project(&mut self, entry: Option<AssetId>) -> Result<()> {
        ensure!(!self.is_running(), "请等待导出完成，再切换项目");
        let output = Self::default_output()?;
        self.select_scene(entry);
        self.output = output;
        self.open = false;
        self.logs.clear();
        Ok(())
    }
    pub(crate) fn retain_scenes(&mut self, sources: &SourceCatalog) {
        let first = sources
            .of_kind(AssetKind::Scene)
            .next()
            .map(|source| source.id);
        let scenes: BTreeSet<_> = sources
            .of_kind(AssetKind::Scene)
            .map(|source| source.id)
            .collect();
        if self.entry.is_none_or(|entry| !scenes.contains(&entry)) {
            self.entry = first;
        }
        self.additional
            .retain(|id| scenes.contains(id) && Some(*id) != self.entry);
    }
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }
    fn log(&mut self, line: String) {
        if self.logs.len() == 256 {
            self.logs.pop_front();
        }
        self.logs.push_back(line.chars().take(4096).collect());
    }
    pub fn start(
        &mut self,
        paths: &ProjectPaths,
        products: &Path,
        source: &Path,
        action: ExportAction,
        retained: Vec<Arc<comet_scene::ContentSource>>,
        identity: ExportIdentity,
    ) -> Result<()> {
        let ExportIdentity {
            game_id: expected_game_id,
            document,
        } = identity;
        let prefab_asset = document.prefab_asset;
        ensure!(!self.is_running(), "已有导出任务正在进行");
        let template = self
            .template
            .clone()
            .context("缺少 App 模板，请重新运行 ./start.sh 准备程序")?;
        template.verify()?;
        let output = paths.export_destination(&action.output, products)?;
        ensure!(
            std::fs::symlink_metadata(&output)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "输出位置已存在，请选择一个新位置"
        );
        let source = paths.resolve_asset_path(source)?;
        let job_source = source.clone();
        let job_paths = paths.clone();
        let origin = paths.clone();
        let saved_digest = comet_core::digest(&std::fs::read(&source)?);
        let project = paths.root().to_owned();
        let settings = comet_asset_pipeline::ProjectSettings::load(paths)?;
        ensure!(
            settings.game_id == Some(expected_game_id),
            "项目游戏身份已变化，请重新打开项目"
        );
        let products = products.to_owned();
        let task = Job::spawn("comet-export", move |sender| {
            crate::project::verify_prefab_identity(&origin, &source, prefab_asset)?;
            let _ = sender.send("正在构建项目资源".into());
            ensure!(
                comet_core::digest(&std::fs::read(&source)?) == saved_digest,
                "作者文件在保存后发生变化，请重新导出"
            );
            let report =
                comet_asset_pipeline::prepare(&project, &products).context("项目资源构建失败")?;
            ensure!(
                comet_core::digest(&std::fs::read(&source)?) == saved_digest,
                "作者文件在导出期间发生变化，请重新导出"
            );
            crate::project::verify_prefab_identity(&origin, &source, prefab_asset)?;
            let _ = sender.send(format!(
                "资源构建完成：{} 个产品，{} 个更新，{} 个复用",
                report.products, report.changed, report.reused
            ));
            let _ = sender.send("正在打包所选场景与 Release App".into());
            let mut assets = AssetStore::open(&products).context("读取资源产品失败")?;
            let snapshot = crate::validation::validate_sources(
                retained.iter().map(|source| source.as_ref()),
                &mut assets,
                prefab_asset,
            )?;
            ensure!(
                comet_asset_pipeline::ProjectSettings::load(&ProjectPaths::open(&project)?)?
                    == settings,
                "项目设置在导出期间发生变化，请重新导出"
            );
            let sources = SourceCatalog::read(&origin, assets.index())?;
            let revision = assets.revision().to_owned();
            let output = package_export(
                &mut assets,
                PackageRequest {
                    products: &products,
                    game_id: expected_game_id,
                    index_revision: Some(&revision),
                    entry_scene: action.entry,
                    additional_scenes: &action.additional,
                    app: template.app(),
                    runtime: Some(template.identity().clone()),
                    app_digest: Some(template.app_digest()),
                    notices: template.notices(),
                    app_icon: Some(template.app_icon()),
                    game_icon: settings.game_icon,
                    output: &output,
                },
            )?;
            Ok((
                output,
                crate::resources::ResourceCandidate {
                    document,
                    assets,
                    sources,
                    snapshot: Arc::new(snapshot),
                },
            ))
        })?;
        self.logs.clear();
        self.failure = None;
        self.last_output = None;
        self.stage = "正在开始导出".into();
        self.job = Some(ExportJob {
            task,
            run: action.run,
            paths: job_paths,
            source: job_source,
        });
        Ok(())
    }
    pub fn poll(&mut self) -> Option<CompletedExport> {
        self.children
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
        let mut job = self.job.take()?;
        let result = job.task.poll(|message| {
            self.stage = message.clone();
            self.log(message);
        });
        match result {
            None => {
                self.job = Some(job);
                None
            }
            Some(Err(error)) => {
                self.fail(error);
                None
            }
            Some(Ok((output, resources))) => Some(CompletedExport {
                output,
                run: job.run,
                resources,
                paths: job.paths,
                source: job.source,
            }),
        }
    }
    pub fn finish(&mut self, output: PackageOutput, run: bool, accepted: Result<()>) {
        self.stage = "导出完成".into();
        self.log(format!("已导出：{}", output.root.display()));
        self.last_output = Some(output.clone());
        if let Err(error) = accepted {
            self.stage = "导出完成，资源未接纳".into();
            let message = format!("游戏包已生成，但当前资源未切换：{error:#}");
            self.log(message.clone());
            self.failure = Some(message);
            return;
        }
        if run {
            match std::process::Command::new(&output.executable)
                .current_dir(&output.root)
                .spawn()
            {
                Ok(child) => {
                    self.children.push(child);
                    self.log("已启动导出的游戏".into());
                }
                Err(error) => self.failure = Some(format!("导出已完成，但启动游戏失败：{error}")),
            }
        }
    }
    pub fn completed_output(&self) -> Result<PackageOutput> {
        ensure!(
            self.failure.is_none(),
            "{}",
            self.failure.as_deref().unwrap_or("导出失败")
        );
        self.last_output.clone().context("导出未生成游戏包")
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        sources: &SourceCatalog,
        blocked: bool,
    ) -> Option<ExportAction> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let busy = self.is_running() || blocked;
        let mut action = None;
        egui::Window::new("导出 App")
            .open(&mut open)
            .default_width(560.)
            .show(ctx, |ui| {
                ui.label("保存当前作者文件、构建资源并导出独立游戏。");
                if self.entry.is_none() {
                    ui.weak("项目中没有可导出的场景，请先新建 .scene 文件。");
                }
                if let Some(error) = &self.template_error {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        "App 模板不可用，请重新运行 ./start.sh。",
                    );
                    ui.collapsing("模板诊断详情", |ui| {
                        ui.label(error);
                    });
                } else if self.template.is_none() {
                    ui.label("缺少 App 模板，请重新运行 ./start.sh 准备程序。");
                }
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_label("启动场景")
                        .selected_text(
                            sources
                                .of_kind(AssetKind::Scene)
                                .find(|s| Some(s.id) == self.entry)
                                .map_or_else(
                                    || "请选择启动场景".to_owned(),
                                    |s| s.source.display().to_string(),
                                ),
                        )
                        .show_ui(ui, |ui| {
                            for scene in sources.of_kind(AssetKind::Scene) {
                                ui.selectable_value(
                                    &mut self.entry,
                                    Some(scene.id),
                                    scene.source.display().to_string(),
                                );
                            }
                        });
                    ui.label("额外包含的场景");
                    for scene in sources.of_kind(AssetKind::Scene) {
                        if Some(scene.id) == self.entry {
                            continue;
                        }
                        let mut included = self.additional.contains(&scene.id);
                        if ui
                            .checkbox(&mut included, scene.source.display().to_string())
                            .changed()
                        {
                            if included {
                                self.additional.insert(scene.id);
                            } else {
                                self.additional.remove(&scene.id);
                            }
                        }
                    }
                    ui.label("输出位置（新目录或 macOS .app）");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output).desired_width(f32::INFINITY),
                    );
                    ui.horizontal(|ui| {
                        for (label, run) in [("导出", false), ("导出并运行", true)] {
                            if ui
                                .add_enabled(
                                    self.template.is_some() && self.entry.is_some(),
                                    egui::Button::new(label),
                                )
                                .clicked()
                            {
                                action = Some(ExportAction {
                                    entry: self.entry.unwrap(),
                                    additional: self
                                        .additional
                                        .iter()
                                        .copied()
                                        .filter(|id| Some(*id) != self.entry)
                                        .collect(),
                                    output: self.output.clone().into(),
                                    run,
                                });
                            }
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                    }
                    ui.label(&self.stage);
                });
                if let Some(error) = &self.failure {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        "导出未完成，请检查路径、资源或 App 模板。",
                    );
                    ui.collapsing("错误详情", |ui| {
                        ui.label(error);
                    });
                }
                egui::ScrollArea::vertical()
                    .max_height(220.)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.logs {
                            ui.monospace(line);
                        }
                    });
            });
        self.open = open;
        action
    }
}

fn package_export(assets: &mut AssetStore, request: PackageRequest<'_>) -> Result<PackageOutput> {
    validate_export_scenes(assets, request.entry_scene, request.additional_scenes)?;
    comet_asset_pipeline::package_scenes(request).context("游戏打包失败")
}

fn validate_export_scenes(
    assets: &mut AssetStore,
    entry: AssetId,
    additional: &[AssetId],
) -> Result<()> {
    let mut closure = BTreeSet::new();
    for scene in std::iter::once(entry).chain(additional.iter().copied()) {
        ensure!(
            assets
                .index()
                .products
                .get(&scene)
                .is_some_and(|product| product.kind == AssetKind::Scene),
            "导出资源 {scene} 不是场景"
        );
        closure.extend(assets.dependency_closure(scene)?);
    }
    let mut validator = comet_runtime::ScriptValidator::default();
    for id in closure {
        if assets.index().products[&id].kind != AssetKind::Scene {
            continue;
        }
        let snapshot = assets.expand_scene(id)?;
        comet_application::extract(&snapshot, assets, &[])
            .with_context(|| format!("导出场景 {id} 的绘制或骨骼绑定校验失败"))?;
        let resources = assets
            .scene_resources(&snapshot)
            .with_context(|| format!("导出场景 {id} 的资源校验失败"))?;
        comet_runtime::validate_animations(
            &snapshot,
            &resources.skeletons,
            &resources.animations,
            &resources.controllers,
        )
        .with_context(|| format!("导出场景 {id} 的动画校验失败"))?;
        comet_runtime::validate_navigation(&snapshot, resources.navigation.as_deref())
            .with_context(|| format!("导出场景 {id} 的导航校验失败，请先烘焙匹配的导航数据"))?;
        comet_runtime::validate_audio(&snapshot, &resources.audio)
            .with_context(|| format!("导出场景 {id} 的音频校验失败"))?;
        validator
            .validate(&snapshot, &resources.scripts)
            .with_context(|| format!("导出场景 {id} 的脚本校验失败"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
