use super::*;
use comet_core::{EntityId, HudLabel, WorldId};
use std::sync::Arc;

fn frame(text: &str) -> HudFrame {
    HudFrame {
        world: WorldId::new(),
        labels: Arc::from([HudLabel {
            owner: EntityId::new(),
            text: Arc::from(text),
        }]),
    }
}

#[test]
fn chinese_hud_wraps_in_logical_points_and_keeps_the_viewport_clip_at_each_dpi() {
    let viewport = Rect::from_min_size(Pos2::new(100., 80.), Vec2::new(220., 140.));
    let text = "收集任务：找到金色目标后按交互键继续。".repeat(12);
    let hud = frame(&text);
    hud.validate().unwrap();
    for ppp in [1., 1.5, 2.] {
        let ctx = egui::Context::default();
        crate::configure_ui(&ctx);
        ctx.set_pixels_per_point(ppp);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640., 480.))),
                ..Default::default()
            },
            |ui| paint(ui, viewport, &hud),
        );
        assert!((output.pixels_per_point - ppp).abs() < 1e-6);
        let shape = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(value) if value.galley.job.text == text))
            .expect("HUD text must be part of this UI pass");
        assert_eq!(shape.clip_rect, viewport);
        let egui::epaint::Shape::Text(shape) = &shape.shape else {
            unreachable!()
        };
        assert!(
            shape.galley.rows.len() > 1,
            "long Chinese text wraps within the view"
        );
        assert!(
            !output.textures_delta.is_empty(),
            "new Chinese glyphs belong to this pass"
        );
        output.textures_delta.clear();
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(!primitives.is_empty());
        assert!(
            primitives
                .iter()
                .all(|primitive| primitive.clip_rect == viewport)
        );
    }
}

#[test]
fn empty_outside_and_nonfinite_viewports_never_submit_hud_geometry() {
    let hud = frame("这段文字不应绘制");
    for viewport in [
        Rect::NOTHING,
        Rect::from_min_size(Pos2::ZERO, Vec2::ZERO),
        Rect::from_min_size(Pos2::new(900., 900.), Vec2::splat(100.)),
        Rect {
            min: Pos2::new(f32::NAN, 0.),
            max: Pos2::new(100., 100.),
        },
    ] {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640., 480.))),
                ..Default::default()
            },
            |ui| paint(ui, viewport, &hud),
        );
        output.textures_delta.clear();
        assert!(
            ctx.tessellate(output.shapes, output.pixels_per_point)
                .is_empty()
        );
    }
}
