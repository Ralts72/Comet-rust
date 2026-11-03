use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_pipeline::{ProjectSettings, SourceMeta};
use comet_core::{AssetId, ProjectPaths};
use std::path::Path;

pub(crate) struct ProjectPanel {
    pub open: bool,
    settings: ProjectSettings,
    icons: Vec<(AssetId, String)>,
    error: Option<String>,
}
impl ProjectPanel {
    pub fn new(paths: &ProjectPaths) -> Result<Self> {
        let mut icons = vec![];
        collect(paths.assets(), paths, &mut icons)?;
        icons.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(Self {
            open: false,
            settings: ProjectSettings::load(paths)?,
            icons,
            error: None,
        })
    }
    pub fn show(&mut self, ctx: &egui::Context, paths: &ProjectPaths, assets: &mut AssetStore) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("项目设置")
            .open(&mut open)
            .default_width(480.)
            .show(ctx, |ui| {
                ui.heading("游戏图标");
                ui.label("从项目 PNG 资源中选择正方形图标，建议使用 1024 × 1024 像素。");
                let mut selected = self.settings.game_icon;
                egui::ComboBox::from_label("图标资源")
                    .selected_text(selected.map_or("Comet 默认图标", |id| {
                        self.icons
                            .iter()
                            .find(|(icon, _)| *icon == id)
                            .map_or("图标资源已丢失", |(_, label)| label.as_str())
                    }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, None, "Comet 默认图标");
                        for (id, label) in &self.icons {
                            ui.selectable_value(&mut selected, Some(*id), label);
                        }
                    });
                if selected != self.settings.game_icon {
                    let candidate = ProjectSettings {
                        game_icon: selected,
                        ..self.settings.clone()
                    };
                    let result = (|| -> Result<()> {
                        let latest = ProjectSettings::load(paths)?;
                        if latest != self.settings {
                            self.settings = latest;
                            anyhow::bail!("项目设置已被外部修改，请重新选择。");
                        }
                        candidate.save(paths, assets)
                    })();
                    match result {
                        Ok(()) => {
                            self.settings = candidate;
                            self.error = None;
                        }
                        Err(error) => self.error = Some(format!("{error:#}")),
                    }
                }
                ui.weak("选择后自动保存。重新导出游戏后生效，编辑器图标保持不变。");
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            });
        self.open = open;
    }
}
fn collect(
    directory: &Path,
    paths: &ProjectPaths,
    icons: &mut Vec<(AssetId, String)>,
) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        ensure!(
            !entry.file_type()?.is_symlink(),
            "source symlinks are not supported"
        );
        let path = paths.resolve_asset_path(entry.path())?;
        if path.is_dir() {
            collect(&path, paths, icons)?;
        } else if path.extension().is_some_and(|e| e == "png") {
            let meta: SourceMeta =
                serde_json::from_slice(&std::fs::read(format!("{}.meta", path.display()))?)?;
            icons.push((
                meta.id,
                path.strip_prefix(paths.assets())
                    .context("图标资源路径")?
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
    }
    Ok(())
}
