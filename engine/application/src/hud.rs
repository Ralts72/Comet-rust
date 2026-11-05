//! Runtime text uses the accepted viewport and the current UI pass's font atlas.
use comet_core::HudFrame;
use egui::{Color32, FontId, Pos2, Rect, Vec2};

pub(crate) fn paint(ui: &egui::Ui, viewport: Rect, frame: &HudFrame) {
    if !viewport.is_finite() || !viewport.is_positive() || frame.labels.is_empty() {
        return;
    }
    let clip = viewport.intersect(ui.clip_rect());
    if !clip.is_positive() {
        return;
    }
    let painter = ui.painter_at(clip);
    let margin = 12.;
    // Leave the editor's view label visible; all positions remain in logical UI points.
    let mut position = viewport.min + Vec2::new(margin, 40.);
    let width = (viewport.width() - 2. * margin).max(0.);
    if width <= 0. {
        return;
    }
    for label in frame.labels.iter() {
        if position.y >= clip.max.y {
            break;
        }
        let text = painter.layout(
            label.text.to_string(),
            FontId::proportional(18.),
            Color32::WHITE,
            width,
        );
        let background =
            Rect::from_min_size(position - Vec2::splat(6.), text.size() + Vec2::splat(12.));
        painter.rect_filled(background, 4., Color32::from_black_alpha(190));
        let height = text.size().y;
        painter.galley(Pos2::new(position.x, position.y), text, Color32::WHITE);
        position.y += height + 16.;
    }
}

#[cfg(test)]
mod tests;
