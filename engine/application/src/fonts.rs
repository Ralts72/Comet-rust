//! Built-in fallback shared by Editor and App, including exported standalone games.
use std::sync::Arc;

const CHINESE_FONT: &[u8] = include_bytes!("../resources/fonts/NotoSansSC-Bold.otf");
const _: () = assert!(
    CHINESE_FONT.len() > 4
        && CHINESE_FONT[0] == b'O'
        && CHINESE_FONT[1] == b'T'
        && CHINESE_FONT[2] == b'T'
        && CHINESE_FONT[3] == b'O',
    "缺少中文字体数据，请执行 git lfs pull"
);

pub fn configure_ui(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "comet-chinese".into(),
        Arc::new(egui::FontData::from_static(CHINESE_FONT)),
    );
    // Keep the existing Latin/monospace metrics; use Noto for missing Chinese glyphs.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("comet-chinese".into());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
#[path = "../tests/unit/fonts.rs"]
mod tests;
