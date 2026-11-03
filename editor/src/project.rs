use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::{ProjectSettings, SourceCatalog};
use comet_core::ProjectPaths;

pub(crate) struct ProjectPanel {
    pub open: bool,
    settings: ProjectSettings,
    error: Option<String>,
}
impl ProjectPanel {
    pub fn new(paths: &ProjectPaths) -> Result<Self> {
        Ok(Self {
            open: false,
            settings: ProjectSettings::load(paths)?,
            error: None,
        })
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        paths: &ProjectPaths,
        assets: &mut AssetStore,
        sources: &SourceCatalog,
    ) {
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
                    .selected_text(selected.map_or_else(
                        || "Comet 默认图标".to_owned(),
                        |id| {
                            sources
                                .of_kind(AssetKind::Texture)
                                .find(|icon| icon.id == id)
                                .map_or_else(
                                    || "图标资源已丢失".to_owned(),
                                    |icon| icon.source.display().to_string(),
                                )
                        },
                    ))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, None, "Comet 默认图标");
                        for icon in sources.of_kind(AssetKind::Texture) {
                            ui.selectable_value(
                                &mut selected,
                                Some(icon.id),
                                icon.source.display().to_string(),
                            );
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
