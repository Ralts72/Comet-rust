use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::{
    game_data::{DataValue, GameData},
    product::{AssetKind, Product},
};

pub(super) fn show(editor: &mut Editor, ctx: &egui::Context, assets: &mut AssetStore) {
    let Some(source) = editor.resource_selection.and_then(|id| {
        editor
            .sources
            .of_kind(AssetKind::Data)
            .find(|source| source.id == id)
    }) else {
        return;
    };
    let product = match assets.load(source.id) {
        Ok(product) => product,
        Err(error) => {
            editor.notice(Err(error));
            return;
        }
    };
    let Product::Data(table) = &product.payload else {
        return;
    };
    let mut open = true;
    egui::Window::new(format!("数据 · {}", source.source.display()))
        .id(egui::Id::unique(("data-preview", editor.document.world_id(), source.id)))
        .default_pos([240., 100.])
        .default_size([820., 400.])
        .min_height(220.)
        .fade_in(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("记录：{}  字段：{}", table.rows.len(), table.fields.len()));
            ui.weak("只读预览。修改源文件后刷新资源，再重新进入 Play。CSV 字段说明保存在同名 .schema.json 中。");
            draw(ui, table);
        });
    if !open {
        editor.resource_selection = None;
    }
}

fn draw(ui: &mut egui::Ui, table: &GameData) {
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let cell = |ui: &mut egui::Ui, text: &str| {
        ui.add_sized([150., height], egui::Label::new(text).truncate())
            .on_hover_text(text);
    };
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            cell(ui, "ID");
            for key in table.fields.keys() {
                cell(ui, key);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height(300.).show_rows(
            ui,
            height,
            table.rows.len(),
            |ui, range| {
                for (id, row) in table.rows.iter().skip(range.start).take(range.len()) {
                    ui.horizontal(|ui| {
                        cell(ui, id);
                        for key in table.fields.keys() {
                            match &row[key] {
                                DataValue::Text(value) => cell(ui, value),
                                value => cell(
                                    ui,
                                    &serde_json::to_string(value).expect("validated data value"),
                                ),
                            }
                        }
                    });
                }
            },
        );
    });
}
