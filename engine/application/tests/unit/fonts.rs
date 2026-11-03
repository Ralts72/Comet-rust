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
