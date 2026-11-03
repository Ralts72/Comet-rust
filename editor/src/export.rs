//! Export orchestration owns background CPU work; document and UI mutation stay on the main thread.
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_pipeline::{AppTemplate, PackageOutput, PackageRequest};
use comet_core::{AssetId, ProjectPaths};
use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    process::Child,
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
};

use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::SourceCatalog;
pub struct ExportAction {
    pub entry: AssetId,
    pub additional: Vec<AssetId>,
    pub output: PathBuf,
    pub run: bool,
}
enum Event {
    Stage(String),
    Finished(std::result::Result<(PackageOutput, AssetStore), String>),
}
struct Job {
    receiver: Receiver<Event>,
    worker: JoinHandle<()>,
    run: bool,
}
pub struct ExportController {
    pub open: bool,
    pub template: Option<AppTemplate>,
    pub template_error: Option<String>,
    entry: AssetId,
    additional: BTreeSet<AssetId>,
    output: String,
    job: Option<Job>,
    children: Vec<Child>,
    logs: VecDeque<String>,
    stage: String,
    pub last_output: Option<PackageOutput>,
    failure: Option<String>,
}
impl ExportController {
    pub fn new(entry: AssetId) -> Result<Self> {
        let name = if cfg!(target_os = "macos") {
            "CometGame.app"
        } else {
            "CometGame"
        };
        Ok(Self {
            open: false,
            template: None,
            template_error: None,
            entry,
            additional: BTreeSet::new(),
            output: Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .context("workspace directory")?
                .join("target/exports")
                .join(name)
                .to_string_lossy()
                .into_owned(),
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
    pub fn entry_scene(&self) -> AssetId {
        self.entry
    }
    pub fn select_scene(&mut self, entry: AssetId) {
        self.entry = entry;
        self.additional.clear();
        self.open = false;
        self.last_output = None;
        self.failure = None;
        self.stage = "就绪".into();
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
    ) -> Result<()> {
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
        let saved_digest = comet_core::digest(&std::fs::read(&source)?);
        let project = paths.root().to_owned();
        let settings = comet_asset_pipeline::ProjectSettings::load(paths)?;
        let products = products.to_owned();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("comet-export".into())
            .spawn(move || {
                let result = (|| -> Result<_> {
                    let _ = sender.send(Event::Stage("正在构建项目资源".into()));
                    ensure!(
                        comet_core::digest(&std::fs::read(&source)?) == saved_digest,
                        "作者场景在保存后发生变化，请重新导出"
                    );
                    let report = comet_asset_pipeline::prepare(&project, &products)
                        .context("项目资源构建失败")?;
                    ensure!(
                        comet_core::digest(&std::fs::read(&source)?) == saved_digest,
                        "作者场景在导出期间发生变化，请重新导出"
                    );
                    let _ = sender.send(Event::Stage(format!(
                        "资源构建完成：{} 个产品，{} 个更新，{} 个复用",
                        report.products, report.changed, report.reused
                    )));
                    let _ = sender.send(Event::Stage("正在打包所选场景与 Release App".into()));
                    let assets = AssetStore::open(&products).context("读取资源产品失败")?;
                    ensure!(
                        comet_asset_pipeline::ProjectSettings::load(&ProjectPaths::open(
                            &project
                        )?)? == settings,
                        "项目设置在导出期间发生变化，请重新导出"
                    );
                    let output = comet_asset_pipeline::package_scenes(PackageRequest {
                        products: &products,
                        index_revision: Some(assets.revision()),
                        entry_scene: action.entry,
                        additional_scenes: &action.additional,
                        app: template.app(),
                        runtime: Some(template.identity().clone()),
                        app_digest: Some(template.app_digest()),
                        notices: template.notices(),
                        app_icon: Some(template.app_icon()),
                        game_icon: settings.game_icon,
                        output: &output,
                    })
                    .context("游戏打包失败")?;
                    Ok((output, assets))
                })();
                let _ = sender.send(Event::Finished(result.map_err(|e| format!("{e:#}"))));
            })?;
        self.logs.clear();
        self.failure = None;
        self.last_output = None;
        self.stage = "正在开始导出".into();
        self.job = Some(Job {
            receiver,
            worker,
            run: action.run,
        });
        Ok(())
    }
    pub fn poll(&mut self, assets: &mut AssetStore) -> Result<()> {
        while let Some(job) = &self.job {
            match job.receiver.try_recv() {
                Ok(Event::Stage(message)) => {
                    self.stage = message.clone();
                    self.log(message);
                }
                Ok(Event::Finished(result)) => {
                    let job = self.job.take().context("export job")?;
                    job.worker
                        .join()
                        .map_err(|_| anyhow::anyhow!("导出任务发生异常"))?;
                    match result {
                        Ok((output, candidate)) => {
                            *assets = candidate;
                            self.stage = "导出完成".into();
                            self.log(format!("已导出：{}", output.root.display()));
                            self.last_output = Some(output.clone());
                            if job.run {
                                match std::process::Command::new(&output.executable)
                                    .current_dir(&output.root)
                                    .spawn()
                                {
                                    Ok(child) => {
                                        self.children.push(child);
                                        self.log("已启动导出的游戏".into());
                                    }
                                    Err(error) => {
                                        self.failure =
                                            Some(format!("导出已完成，但启动游戏失败：{error}"))
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            self.stage = "导出失败".into();
                            self.log(error.clone());
                            self.failure = Some(error);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let job = self.job.take().unwrap();
                    let _ = job.worker.join();
                    self.stage = "导出失败".into();
                    self.failure = Some("导出任务异常退出，未返回结果".into());
                }
            }
        }
        self.children
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
        Ok(())
    }
    pub fn wait(&mut self, assets: &mut AssetStore) -> Result<PackageOutput> {
        while self.is_running() {
            self.poll(assets)?;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ensure!(
            self.failure.is_none(),
            "{}",
            self.failure.as_deref().unwrap_or("导出失败")
        );
        self.last_output.clone().context("导出未生成游戏包")
    }
    pub fn show(&mut self, ctx: &egui::Context, sources: &SourceCatalog) -> Option<ExportAction> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let busy = self.is_running();
        let mut action = None;
        egui::Window::new("导出游戏")
            .open(&mut open)
            .default_width(560.)
            .show(ctx, |ui| {
                ui.label("保存作者场景、构建资源并导出独立游戏。");
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
                                .find(|s| s.id == self.entry)
                                .map_or_else(
                                    || "未知场景".to_owned(),
                                    |s| s.source.display().to_string(),
                                ),
                        )
                        .show_ui(ui, |ui| {
                            for scene in sources.of_kind(AssetKind::Scene) {
                                ui.selectable_value(
                                    &mut self.entry,
                                    scene.id,
                                    scene.source.display().to_string(),
                                );
                            }
                        });
                    ui.label("额外包含的场景");
                    for scene in sources.of_kind(AssetKind::Scene) {
                        if scene.id == self.entry {
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
                                .add_enabled(self.template.is_some(), egui::Button::new(label))
                                .clicked()
                            {
                                action = Some(ExportAction {
                                    entry: self.entry,
                                    additional: self
                                        .additional
                                        .iter()
                                        .copied()
                                        .filter(|id| *id != self.entry)
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
impl Drop for ExportController {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            let _ = job.worker.join();
        }
    }
}
