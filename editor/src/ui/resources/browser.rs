use super::drag::ResourceDrag;
use super::tree::{Directory, Entry, kind_label};
use super::{ResourceAction, can_place, is_authoring_source};
use crate::{resources::ResourceVersion, session::Editor};
use comet_asset::AssetStore;
use comet_asset_data::product::{AssetKind, ProductIndex};
use std::{path::PathBuf, sync::Arc};

fn browser_id() -> egui::Id {
    egui::Id::unique("project-resource-browser")
}

#[derive(Clone, Default)]
pub(super) struct Browser {
    project: PathBuf,
    index: Option<Arc<ProductIndex>>,
    tree: Arc<Directory>,
    visible: Arc<Directory>,
    query: String,
    kind: Option<AssetKind>,
}
impl Browser {
    pub fn load(editor: &Editor, ui: &egui::Ui, assets: &AssetStore) -> Self {
        let mut browser = ui
            .data_mut(|data| data.remove_temp::<Self>(browser_id()))
            .unwrap_or_default();
        browser.sync(editor, assets);
        browser
    }
    pub fn controls(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.separator();
        changed |= ui
            .add(
                egui::TextEdit::singleline(&mut self.query)
                    .desired_width(140.)
                    .hint_text("搜索文件..."),
            )
            .changed();
        if ui.small_button("清除").clicked() {
            self.query.clear();
            changed = true;
        }
        egui::ComboBox::from_id_salt("resource-kind")
            .width(84.)
            .selected_text(self.kind.map_or("全部类型", kind_label))
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut self.kind, None, "全部类型")
                    .changed();
                for kind in [
                    AssetKind::Scene,
                    AssetKind::Prefab,
                    AssetKind::Mesh,
                    AssetKind::Material,
                    AssetKind::Texture,
                    AssetKind::Script,
                    AssetKind::Data,
                    AssetKind::Navigation,
                    AssetKind::Audio,
                    AssetKind::Skeleton,
                    AssetKind::Skin,
                    AssetKind::AnimationClip,
                    AssetKind::AnimationController,
                ] {
                    changed |= ui
                        .selectable_value(&mut self.kind, Some(kind), kind_label(kind))
                        .changed();
                }
            });
        if changed {
            self.filter();
        }
    }
    pub fn show(
        self,
        editor: &mut Editor,
        ui: &mut egui::Ui,
        assets: &AssetStore,
        action: &mut Option<ResourceAction>,
    ) {
        let filtered = !self.query.trim().is_empty() || self.kind.is_some();
        ui.push_id(&self.project, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("resources-list")
                .show(ui, |ui| {
                    draw_directory(editor, ui, assets, &self.visible, filtered, action);
                    if self.visible.is_empty() {
                        ui.weak(if self.tree.is_empty() {
                            "项目中没有已构建的资源。"
                        } else {
                            "没有匹配的资源。"
                        });
                    }
                });
        });
        ui.data_mut(|data| data.insert_temp(browser_id(), self));
    }

    fn sync(&mut self, editor: &Editor, assets: &AssetStore) {
        let index = assets.shared_index();
        if self.project != editor.project_root() {
            *self = Self {
                project: editor.project_root().into(),
                ..Default::default()
            };
        }
        if self
            .index
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &index))
        {
            self.tree = Arc::new(Directory::build(&editor.sources, &index));
            self.index = Some(index);
            self.filter();
        }
    }
    fn filter(&mut self) {
        self.visible = if self.query.trim().is_empty() && self.kind.is_none() {
            self.tree.clone()
        } else {
            Arc::new(self.tree.filter(&self.query, self.kind))
        };
    }
}

fn draw_directory(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &AssetStore,
    directory: &Directory,
    filtered: bool,
    action: &mut Option<ResourceAction>,
) {
    for (name, child) in &directory.directories {
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id(("resource-directory", &child.path, filtered)),
            true,
        );
        if filtered {
            state.set_open(true);
        }
        state
            .show_header(ui, |ui| {
                ui.label(name.to_string_lossy());
            })
            .body(|ui| draw_directory(editor, ui, assets, child, filtered, action));
    }
    for entry in &directory.entries {
        if entry.children.is_empty() {
            draw_entry(editor, ui, assets, entry, action);
        } else {
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id(("resource-source", entry.id, filtered)),
                false,
            );
            if filtered {
                state.set_open(true);
            }
            state
                .show_header(ui, |ui| draw_entry(editor, ui, assets, entry, action))
                .body(|ui| {
                    for child in &entry.children {
                        draw_entry(editor, ui, assets, child, action);
                    }
                });
        }
    }
}

fn draw_entry(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &AssetStore,
    entry: &Entry,
    action: &mut Option<ResourceAction>,
) {
    let draggable = !entry.derived
        && can_place(editor, &entry.source)
        && !editor.document.is_editing()
        && editor.drag_origin.is_none();
    let kind =
        if !entry.derived && entry.kind == AssetKind::Prefab && !is_authoring_source(&entry.source)
        {
            "模型"
        } else {
            kind_label(entry.kind)
        };
    let response = ui
        .push_id(entry.id, |ui| {
            ui.add(
                egui::Button::selectable(
                    editor.resource_selection == Some(entry.id),
                    format!("{kind}  {}", entry.label),
                )
                .sense(if draggable {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::click()
                }),
            )
        })
        .inner;
    if response.clicked() || response.drag_started() {
        editor.resource_selection = Some(entry.id);
    }
    if draggable && response.drag_started() {
        response.dnd_set_drag_payload(ResourceDrag {
            source: entry.source.clone(),
            version: ResourceVersion::capture(&editor.document, assets),
            target: editor.target_generation,
        });
    }
    if response.double_clicked() && !entry.derived {
        if can_place(editor, &entry.source) {
            *action = Some(ResourceAction::Place(entry.source.clone()));
        } else if is_authoring_source(&entry.source)
            && !editor.is_busy()
            && !editor.documents.is_blocking()
        {
            *action = Some(ResourceAction::EditSource(entry.source.source.clone()));
        }
    }
    response
        .on_hover_ui(|ui| {
            ui.label(format!(
                "{} · {}",
                kind_label(entry.kind),
                entry.source.source.display()
            ));
            ui.label(entry.id.to_string());
            if draggable {
                ui.small("双击保留源坐标；拖到场景将资源原点放在鼠标对应的相机焦平面上。");
            }
            if entry.derived {
                ui.small("由源资源生成；刷新资源后同步更新。");
            }
        })
        .context_menu(|ui| {
            if ui.button("复制 UUID").clicked() {
                ui.ctx().copy_text(entry.id.to_string());
                ui.close();
            }
            if ui.button("复制源路径").clicked() {
                ui.ctx()
                    .copy_text(entry.source.source.to_string_lossy().into());
                ui.close();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use comet_application::{Application, FrameContext, ViewRequest};
    use std::fs;

    struct Capture(Editor);
    impl Application for Capture {
        fn frame(
            &mut self,
            ui: &mut egui::Ui,
            assets: &mut AssetStore,
            input: &FrameContext,
        ) -> anyhow::Result<ViewRequest> {
            if input.number == 0 {
                let mut browser = Browser::load(&self.0, ui, assets);
                browser.query = "Amber".into();
                browser.kind = Some(AssetKind::Material);
                browser.filter();
                let model =
                    &browser.visible.directories[std::ffi::OsStr::new("模型 集")].entries[0];
                assert_eq!(model.children.len(), 1);
                assert_eq!(model.children[0].label, "Amber");
                ui.data_mut(|data| data.insert_temp(browser_id(), browser));
            }
            self.0.frame(ui, assets, input)
        }
        fn feedback(&mut self, feedback: &comet_render::FrameFeedback) {
            self.0.feedback(feedback);
        }
        fn report(&mut self) -> serde_json::Value {
            self.0.report()
        }
    }

    #[test]
    #[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
    fn headless_project_tree_draws_linked_prefab_copies_and_filtered_material() -> anyhow::Result<()>
    {
        let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
        let directory = editor.project_root().join("assets/模型 集");
        fs::create_dir(&directory)?;
        for name in ["showcase.gltf", "showcase.gltf.meta", "showcase.bin"] {
            fs::rename(
                editor.project_root().join("assets").join(name),
                directory.join(name),
            )?;
        }
        let model = comet_asset_pipeline::SourceMeta::load(&directory.join("showcase.gltf"))?.id;
        let mut body = comet_scene::EntityData::new("放置实例");
        body.physics = Some(comet_scene::PhysicsBody {
            motion: comet_scene::BodyMotion::Fixed,
            ..Default::default()
        });
        let attached = body.id;
        let physical = editor.project_root().join("assets/placed.prefab");
        fs::write(
            &physical,
            comet_scene::ContentSource {
                base: comet_scene::SceneSnapshot {
                    entities: vec![body],
                    ..Default::default()
                },
                instances: vec![comet_scene::PrefabInstance {
                    id: comet_core::InstanceId::new(),
                    prefab: model,
                    parent: Some(attached),
                    translation: [0.; 3],
                    overrides: vec![],
                }],
                ..Default::default()
            }
            .to_json()?,
        )?;
        comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
        assets.refresh()?;
        editor.sources = comet_asset_pipeline::SourceCatalog::read(
            &comet_core::ProjectPaths::open(editor.project_root())?,
            assets.index(),
        )?;
        let id = comet_asset_pipeline::SourceMeta::load(&physical)?.id;
        let position = editor
            .camera
            .focus_plane_point([0.75, 0.4], 780. / 520.)
            .unwrap();
        editor.place_resource(id, "放置实例", position.to_array(), &mut assets)?;
        let root = editor.selection.active().unwrap();
        editor.place_copies(root, 2, [-3., 0., 0.], &mut assets)?;
        assert_eq!(editor.document.shared_source().instances.len(), 4);
        assert_eq!(editor.selection.len(), 2);
        let physical_bytes = fs::read(&physical)?;
        for (_, tile) in editor.workspace.tree.tiles.iter_mut() {
            if let egui_tiles::Tile::Container(egui_tiles::Container::Linear(split)) = tile
                && split.dir == egui_tiles::LinearDir::Vertical
            {
                split.shares[split.children[0]] = 520.;
                split.shares[split.children[1]] = 209.;
            }
        }
        let source = editor.source.clone();
        let bytes = fs::read(&source)?;
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/verification");
        let mut options = comet_application::HostOptions::new("Comet · 项目资源");
        options.headless = true;
        options.smoke = true;
        options.capture = Some(out.join("editor-resource-tree.png"));
        options.report = Some(out.join("editor-resource-tree.json"));
        comet_application::run(options, assets, Capture(editor))?;
        assert_eq!(fs::read(source)?, bytes);
        assert_eq!(fs::read(physical)?, physical_bytes);
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("editor-resource-tree.json"))?)?;
        assert_eq!(report["application"]["dirty"], true);
        assert_eq!(report["application"]["history"], 2);
        assert!(
            report["frames"]
                .as_array()
                .unwrap()
                .iter()
                .all(|frame| frame["draw_count"] == 20)
        );
        assert!(report["application"]["error"].is_null());
        assert_eq!(report["submitted_frames"], 8);
        Ok(())
    }
}
