use egui::{Color32, CornerRadius, Stroke};

pub(super) fn apply(root: &mut egui::Ui) {
    let key = egui::Id::unique("editor-theme");
    if !root.data(|data| data.get_temp::<bool>(key).unwrap_or(false)) {
        let ctx = root.ctx().clone();
        ctx.set_theme(egui::Theme::Dark);
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = Color32::from_gray(24);
            style.visuals.window_fill = Color32::from_gray(20);
            style.visuals.extreme_bg_color = Color32::from_gray(16);
            style.visuals.override_text_color = Some(Color32::from_gray(235));
            style.visuals.window_corner_radius = CornerRadius::same(2);
            style.visuals.menu_corner_radius = CornerRadius::same(2);
            style.visuals.window_stroke = Stroke::new(1., Color32::from_gray(65));
            style.visuals.selection.bg_fill = Color32::from_rgb(41, 74, 112);
            for visuals in [
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
                &mut style.visuals.widgets.open,
                &mut style.visuals.widgets.noninteractive,
            ] {
                visuals.corner_radius = CornerRadius::same(2);
            }
            style.visuals.widgets.inactive.bg_fill = Color32::from_gray(43);
            style.visuals.widgets.inactive.weak_bg_fill = Color32::from_gray(35);
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(49, 75, 103);
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(47, 90, 135);
            style.spacing.item_spacing = egui::vec2(6., 4.);
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(16.));
        });
        root.set_style(ctx.style_of(egui::Theme::Dark));
        root.data_mut(|data| data.insert_temp(key, true));
    }
}
