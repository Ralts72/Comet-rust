use super::*;

struct Case {
    ctx: egui::Context,
    camera: Camera,
    number: u32,
}
impl Case {
    fn frame(&mut self, events: Vec<egui::Event>, active: bool, pan: bool) {
        self.number += 1;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                time: Some(f64::from(self.number) / 60.),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                ui.spacing_mut().window_margin = egui::Margin::ZERO;
                let response = ui.allocate_rect(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
                    egui::Sense::click_and_drag(),
                );
                navigate_camera(&mut self.camera, &response, active, pan);
            },
        );
        output.textures_delta.clear();
    }
    fn drag(&mut self, button: egui::PointerButton, active: bool, pan: bool) {
        let start = egui::pos2(400., 300.);
        let event = |pressed| egui::Event::PointerButton {
            pos: start,
            button,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(
            vec![egui::Event::PointerMoved(start), event(true)],
            active,
            pan,
        );
        self.frame(
            vec![egui::Event::PointerMoved(start + egui::vec2(100., 50.))],
            active,
            pan,
        );
        self.frame(vec![event(false)], active, pan);
    }
}

#[test]
fn navigation_keeps_idle_views_and_uses_projection_scaled_pan_zoom_and_input_authorization() {
    let mut case = Case {
        ctx: Default::default(),
        camera: Camera {
            eye: Vec3::Z * 0.2,
            target: Vec3::ZERO,
            ..Default::default()
        },
        number: 0,
    };
    let before = case.camera;
    case.frame(vec![], true, true);
    assert_eq!(
        case.camera, before,
        "idle input must preserve a fitted distance below the old clamp"
    );
    case.camera = Camera {
        eye: Vec3::Z * 5.,
        target: Vec3::ZERO,
        projection: Projection::Orthographic { height: 6. },
        ..Default::default()
    };
    let before = case.camera;
    case.drag(egui::PointerButton::Middle, false, true);
    assert_eq!(case.camera, before);
    case.drag(egui::PointerButton::Middle, true, false);
    assert_eq!(case.camera, before);
    case.drag(egui::PointerButton::Secondary, true, true);
    assert_eq!(case.camera, before, "2D orientation stays fixed");
    case.drag(egui::PointerButton::Middle, true, true);
    assert!(case.camera.target.distance(Vec3::new(-1., 0.5, 0.)) < 1e-5);
    assert_eq!(case.camera.eye - case.camera.target, Vec3::Z * 5.);
    let before = case.camera;
    case.frame(
        vec![
            egui::Event::PointerMoved(egui::pos2(400., 300.)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., 50.),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: Default::default(),
            },
        ],
        true,
        true,
    );
    let Projection::Orthographic { height } = case.camera.projection else {
        panic!("orthographic camera")
    };
    assert!(height < 6. && height > 0.);
    assert_eq!(case.camera.eye, before.eye);
    assert_eq!(case.camera.target, before.target);
    case.camera.projection = Projection::Perspective;
    let before = case.camera;
    let primary = |pressed| egui::Event::PointerButton {
        pos: egui::pos2(400., 300.),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    case.frame(vec![primary(true)], true, false);
    case.drag(egui::PointerButton::Secondary, true, false);
    case.frame(vec![primary(false)], true, false);
    assert_ne!(case.camera.eye, before.eye);
    case.camera.validate().unwrap();
}
