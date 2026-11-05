use super::directory::{DirectoryReader, RequestId, Scope};
use crate::documents::DocumentAction;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct ProjectBrowser {
    create: Option<bool>,
    root: String,
    scene: String,
    picked_root: Option<PathBuf>,
    picked_scene: Option<PathBuf>,
    picker: Option<Picker>,
    error: Option<String>,
}
impl ProjectBrowser {
    pub fn is_open(&self) -> bool {
        self.create.is_some()
    }
    pub fn open(&mut self, create: bool, current: &Path) {
        let path = if create {
            current.parent().unwrap_or(current).join("NewProject")
        } else {
            current.to_owned()
        };
        self.create = Some(create);
        self.root = path.display().to_string();
        self.picked_root = Some(path);
        self.scene = comet_asset_pipeline::DEFAULT_SCENE.into();
        self.picked_scene = None;
        self.picker = None;
        self.error = None;
    }
    fn root_path(&self) -> Option<PathBuf> {
        self.picked_root
            .clone()
            .or_else(|| (!self.root.trim().is_empty()).then(|| self.root.trim().into()))
    }
    fn scene_path(&self) -> Option<PathBuf> {
        self.picked_scene
            .clone()
            .or_else(|| (!self.scene.trim().is_empty()).then(|| self.scene.trim().into()))
    }
    fn browse(&mut self, reader: &mut DirectoryReader, kind: PickerKind) {
        let Some(root) = self.root_path() else {
            return;
        };
        let (scope, directory) = match kind {
            PickerKind::Project => (Scope::Directories, Some(root)),
            PickerKind::Parent => (
                Scope::Directories,
                Some(root.parent().unwrap_or(&root).to_owned()),
            ),
            PickerKind::Author => (Scope::Authors { project: root }, None),
        };
        match Picker::new(reader, kind, scope, directory) {
            Ok(picker) => {
                self.picker = Some(picker);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        reader: &mut DirectoryReader,
    ) -> Option<DocumentAction> {
        let create = self.create?;
        let mut action = None;
        let mut cancel =
            self.picker.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let mut picked = None;
        egui::Modal::new(egui::Id::unique("project-browser")).show(ctx, |ui| {
            ui.set_width(600.);
            if let Some(picker) = &mut self.picker {
                picked = picker.show(ui, reader);
                return;
            }
            ui.heading(if create {
                "新建项目"
            } else {
                "打开项目"
            });
            ui.label(if create {
                "新项目目录（必须尚不存在）"
            } else {
                "项目目录"
            });
            if ui.text_edit_singleline(&mut self.root).changed() {
                self.picked_root = None;
                self.picked_scene = None;
                self.scene = comet_asset_pipeline::DEFAULT_SCENE.into();
            }
            if ui
                .button(if create {
                    "选择父目录"
                } else {
                    "浏览目录"
                })
                .clicked()
            {
                self.browse(
                    reader,
                    if create {
                        PickerKind::Parent
                    } else {
                        PickerKind::Project
                    },
                );
            }
            if create {
                ui.weak(
                    "选择父目录后保留当前新项目名称。创建空项目和 main.scene，已有目录不会被覆盖。",
                );
            } else {
                ui.label("作者文件（相对项目 assets）");
                if ui.text_edit_singleline(&mut self.scene).changed() {
                    self.picked_scene = None;
                }
                if ui
                    .add_enabled(
                        self.root_path().is_some(),
                        egui::Button::new("浏览作者文件"),
                    )
                    .clicked()
                {
                    self.browse(reader, PickerKind::Author);
                }
                ui.weak("默认 main.scene，也可选择项目内的 .scene 或原生 .prefab 文件。");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.root_path().is_some() && (create || self.scene_path().is_some()),
                        egui::Button::new(if create { "创建并打开" } else { "打开" }),
                    )
                    .clicked()
                {
                    let root = self.root_path().unwrap();
                    action = Some(if create {
                        DocumentAction::NewProject(root)
                    } else {
                        DocumentAction::OpenProject {
                            root,
                            scene: self.scene_path().unwrap(),
                        }
                    });
                }
                if ui.button("取消").clicked() {
                    cancel = true;
                }
            });
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        if let Some(result) = picked {
            let picker = self.picker.take().unwrap();
            reader.cancel(picker.request);
            if let Picked::Path { path, assets } = result {
                match picker.kind {
                    PickerKind::Project | PickerKind::Parent => {
                        let path = if picker.kind == PickerKind::Parent {
                            let name = self
                                .root_path()
                                .and_then(|root| root.file_name().map(|name| name.to_owned()))
                                .unwrap_or_else(|| "NewProject".into());
                            path.join(name)
                        } else {
                            path
                        };
                        self.root = path.display().to_string();
                        self.picked_root = Some(path);
                        self.picked_scene = None;
                        self.scene = comet_asset_pipeline::DEFAULT_SCENE.into();
                    }
                    PickerKind::Author => {
                        if let Some(assets) = assets
                            && let Ok(relative) = path.strip_prefix(assets)
                        {
                            self.scene = relative.display().to_string();
                            self.picked_scene = Some(relative.to_owned());
                        }
                    }
                }
            }
        }
        if action.is_some() || cancel {
            if let Some(picker) = self.picker.take() {
                reader.cancel(picker.request);
            }
            self.create = None;
        }
        action
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    Project,
    Parent,
    Author,
}
struct Picker {
    kind: PickerKind,
    scope: Scope,
    request: RequestId,
    applied: Option<RequestId>,
    jump: String,
    jump_path: Option<PathBuf>,
    selected: Option<(PathBuf, bool)>,
    error: Option<String>,
}
enum Picked {
    Path {
        path: PathBuf,
        assets: Option<PathBuf>,
    },
    Cancel,
}
impl Picker {
    fn new(
        reader: &mut DirectoryReader,
        kind: PickerKind,
        scope: Scope,
        directory: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let request = reader.request(scope.clone(), directory.clone())?;
        Ok(Self {
            kind,
            scope,
            request,
            applied: None,
            jump: directory
                .as_ref()
                .map_or_else(String::new, |path| path.display().to_string()),
            jump_path: directory,
            selected: None,
            error: None,
        })
    }
    fn navigate(&mut self, reader: &mut DirectoryReader, directory: PathBuf) {
        match reader.request(self.scope.clone(), Some(directory.clone())) {
            Ok(request) => {
                self.request = request;
                self.applied = None;
                self.selected = None;
                self.error = None;
                self.jump = directory.display().to_string();
                self.jump_path = Some(directory);
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }
    fn show(&mut self, ui: &mut egui::Ui, reader: &mut DirectoryReader) -> Option<Picked> {
        let mut cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
        ui.heading(match self.kind {
            PickerKind::Project => "选择项目目录",
            PickerKind::Parent => "选择新项目父目录",
            PickerKind::Author => "选择作者文件",
        });
        let listing = reader.listing(self.request).map(|result| {
            result
                .as_ref()
                .map(std::sync::Arc::clone)
                .map_err(Clone::clone)
        });
        if let Some(Ok(listing)) = &listing
            && self.applied != Some(self.request)
        {
            self.jump = listing.directory.display().to_string();
            self.jump_path = Some(listing.directory.clone());
            self.applied = Some(self.request);
        }
        let mut navigate = None;
        ui.horizontal(|ui| {
            if ui.text_edit_singleline(&mut self.jump).changed() {
                self.jump_path = None;
            }
            if ui.button("跳转").clicked() {
                navigate = self
                    .jump_path
                    .clone()
                    .or_else(|| (!self.jump.trim().is_empty()).then(|| self.jump.trim().into()));
            }
        });
        let mut chosen = None;
        match &listing {
            None => {
                ui.spinner();
                ui.label("正在读取目录…");
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }
            Some(Err(error)) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            Some(Ok(listing)) => {
                ui.horizontal(|ui| {
                    let parent = listing.directory.parent().filter(|parent| {
                        listing
                            .assets
                            .as_deref()
                            .is_none_or(|assets| parent.starts_with(assets))
                    });
                    if ui
                        .add_enabled(parent.is_some(), egui::Button::new("上一级"))
                        .clicked()
                    {
                        navigate = parent.map(Path::to_owned);
                    }
                    if ui.button("刷新").clicked() {
                        navigate = Some(listing.directory.clone());
                    }
                    if listing.assets.is_none() {
                        if let Some(home) =
                            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                            && ui.button("主目录").clicked()
                        {
                            navigate = Some(home.into());
                        }
                        for root in &listing.roots {
                            if ui.button(root.display().to_string()).clicked() {
                                navigate = Some(root.clone());
                            }
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(280.)
                    .show(ui, |ui| {
                        for entry in &listing.entries {
                            let response = ui
                                .selectable_label(
                                    self.selected
                                        .as_ref()
                                        .is_some_and(|(path, _)| *path == entry.path),
                                    format!(
                                        "{}  {}",
                                        if entry.directory {
                                            "目录"
                                        } else {
                                            "作者文件"
                                        },
                                        entry.name.to_string_lossy()
                                    ),
                                )
                                .on_hover_text(entry.path.display().to_string());
                            if response.clicked() {
                                self.selected = Some((entry.path.clone(), entry.directory));
                            }
                            if response.double_clicked() {
                                if entry.directory {
                                    navigate = Some(entry.path.clone());
                                } else {
                                    chosen = Some(entry.path.clone());
                                }
                            }
                        }
                    });
                if listing.truncated {
                    ui.weak("目录条目过多，仅显示部分结果；请跳转到更具体的目录。");
                }
                if listing.entries.is_empty() {
                    ui.weak("此目录没有可选择的条目。");
                }
                ui.horizontal(|ui| {
                    if let Some((path, true)) = &self.selected
                        && ui.button("进入目录").clicked()
                    {
                        navigate = Some(path.clone());
                    }
                    let selection =
                        if self.kind == PickerKind::Author {
                            self.selected
                                .as_ref()
                                .filter(|(_, directory)| !directory)
                                .map(|(path, _)| path.clone())
                        } else {
                            Some(self.selected.as_ref().map_or_else(
                                || listing.directory.clone(),
                                |(path, _)| path.clone(),
                            ))
                        };
                    if ui
                        .add_enabled(
                            selection.is_some(),
                            egui::Button::new(if self.kind == PickerKind::Author {
                                "选择作者文件"
                            } else {
                                "选择目录"
                            }),
                        )
                        .clicked()
                    {
                        chosen = selection;
                    }
                });
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if ui.button("返回").clicked() {
            cancel = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if cancel {
            return Some(Picked::Cancel);
        }
        if let Some(directory) = navigate {
            self.navigate(reader, directory);
            return None;
        }
        chosen.map(|path| Picked::Path {
            path,
            assets: listing
                .and_then(|result| result.ok())
                .and_then(|listing| listing.assets.clone()),
        })
    }
}

#[cfg(test)]
mod tests;
