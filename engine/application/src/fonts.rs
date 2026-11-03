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
mod tests {
    use super::*;
    #[test]
    fn bundled_fonts_cover_chinese_ui_and_authoring_names_in_both_families() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut output=ctx.run_ui(egui::RawInput::default(),|ui| {
            ui.ctx().fonts_mut(|fonts| {
                for family in [egui::FontFamily::Proportional,egui::FontFamily::Monospace] {
                    let font=egui::FontId::new(14.,family);
                    assert!(fonts.has_glyphs(&font,"场景编辑器保存撤销重做运行暂停继续单步停止导出游戏启动场景额外场景输出位置物理长方体胶囊体角色中文名字材质粗糙度质量摩擦系数路径检查失败诊断详情龘"));
                }
            });
            ui.label("场景编辑器：中文字体渲染");
        });
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
    }
}
