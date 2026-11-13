use super::*;
use comet_core::AssetId;
use comet_scene::schema::{self, FieldKey, FieldValue};

pub(super) struct Group {
    pub(super) _temp: tempfile::TempDir,
    pub(super) editor: Editor,
    pub(super) assets: AssetStore,
    pub(super) ctx: egui::Context,
    pub(super) ids: [EntityId; 2],
    pub(super) number: u64,
}
impl Group {
    pub(super) fn bodies(&mut self) -> Result<[comet_scene::PhysicsBody; 2]> {
        let snapshot = self.editor.snapshot(&mut self.assets)?;
        Ok(self.ids.map(|id| {
            snapshot
                .entities
                .iter()
                .find(|e| e.id == id)
                .unwrap()
                .physics
                .clone()
                .unwrap()
        }))
    }
    pub(super) fn reveal(
        &mut self,
        locate: impl Fn(&egui::FullOutput) -> Option<Pos2>,
    ) -> Result<Pos2> {
        let wheel = |phase, delta| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase,
            source: egui::MouseWheelSource::Trackpad,
            delta: Vec2::new(0., delta),
            modifiers: Default::default(),
        };
        for delta in std::iter::once(0.)
            .chain(std::iter::repeat_n(-240., 8))
            .chain(std::iter::repeat_n(240., 8))
        {
            if delta != 0. {
                // Finish the gesture before locating controls so wheel smoothing cannot move the click target.
                self.frame(vec![
                    Event::PointerMoved(Pos2::new(1270., 400.)),
                    wheel(egui::TouchPhase::Start, 0.),
                    wheel(egui::TouchPhase::Move, delta),
                ]);
                self.frame(vec![wheel(egui::TouchPhase::End, 0.)]);
            }
            let output = self.frame(vec![]);
            if let Some(position) =
                locate(&output).filter(|position| (80. ..735.).contains(&position.y))
            {
                return Ok(position);
            }
        }
        anyhow::bail!("inspector control is not visible after scrolling")
    }
    pub(super) fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        let context = FrameContext {
            number: self.number,
            dt: 1. / 60.,
            active: true,
            activity_changed: false,
            target_generation: 1,
            size: [1280, 800],
            smoke: false,
        };
        self.number += 1;
        crate::ui::tests::frame_with_output(
            &self.ctx,
            &mut self.editor,
            &mut self.assets,
            context,
            events,
        )
        .1
    }
    pub(super) fn transforms(&self) -> [Transform; 2] {
        let source = self.editor.document.shared_source();
        self.ids.map(|id| {
            source
                .base
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .unwrap()
                .transform
                .clone()
        })
    }
    pub(super) fn drag(&mut self, field: &str, axis: &str, distance: f32) -> Result<(Pos2, Pos2)> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let start = control(&output, field, axis)?;
        let end = start + Vec2::new(distance, 0.);
        self.frame(vec![Event::PointerMoved(start), button(start, true)]);
        self.frame(vec![Event::PointerMoved(end)]);
        Ok((start, end))
    }
    pub(super) fn type_value(&mut self, field: &str, axis: &str, value: &str) -> Result<()> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let pos = control(&output, field, axis)?;
        self.type_at(pos, value);
        Ok(())
    }
    pub(super) fn type_at(&mut self, pos: Pos2, value: &str) {
        self.frame(vec![Event::PointerMoved(pos), button(pos, true)]);
        self.frame(vec![button(pos, false)]);
        self.frame(vec![
            key(egui::Key::A, true, egui::Modifiers::COMMAND),
            key(egui::Key::A, false, egui::Modifiers::COMMAND),
            Event::Text(value.into()),
        ]);
    }
}
pub(super) fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    pub(super) fn collect(shape: &egui::epaint::Shape, result: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => result.push((
                text.galley.job.text.clone(),
                text.pos + text.galley.rect.center().to_vec2(),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().for_each(|shape| collect(shape, result))
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    output
        .shapes
        .iter()
        .for_each(|shape| collect(&shape.shape, &mut result));
    result
}
fn control(output: &egui::FullOutput, field: &str, axis: &str) -> Result<Pos2> {
    let values = texts(output);
    let heading = values
        .iter()
        .find(|(text, pos)| text.starts_with(field) && pos.x > 970.)
        .context("common property heading")?
        .1;
    values
        .iter()
        .filter(|(text, pos)| {
            text.starts_with(&format!("{axis} ")) && pos.x > 970. && pos.y > heading.y
        })
        .min_by(|(_, a), (_, b)| a.y.total_cmp(&b.y))
        .map(|(_, pos)| *pos)
        .context("common property control")
}
pub(super) fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
pub(super) fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

impl Group {
    pub(super) fn scalar_values(&mut self, field: FieldKey) -> Result<[f32; 2]> {
        let snapshot = self.editor.snapshot(&mut self.assets)?;
        Ok(self.ids.map(|id| {
            let entity = snapshot
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .unwrap();
            let Some(FieldValue::Scalar(value)) = schema::descriptor(field).unwrap().read(entity)
            else {
                panic!("scalar fixture")
            };
            value
        }))
    }
    pub(super) fn type_scalar(&mut self, field: &str, value: &str) -> Result<()> {
        self.frame(vec![]);
        let position = self.reveal(|output| scalar_control(output, field).ok())?;
        self.type_at(position, value);
        Ok(())
    }
    pub(super) fn confirm(&mut self) {
        self.frame(vec![key(egui::Key::Enter, true, Default::default())]);
        self.frame(vec![key(egui::Key::Enter, false, Default::default())]);
    }
    pub(super) fn drag_scalar(&mut self, field: &str, distance: f32) -> Result<Pos2> {
        self.frame(vec![]);
        let start = self.reveal(|output| scalar_control(output, field).ok())?;
        let end = start + Vec2::new(distance, 0.);
        self.frame(vec![Event::PointerMoved(start), button(start, true)]);
        self.frame(vec![Event::PointerMoved(end)]);
        Ok(end)
    }
    pub(super) fn asset_label(&self, asset: AssetId) -> String {
        let metadata = self.assets.index().products[&asset]
            .editor
            .as_ref()
            .unwrap();
        if metadata.source == asset {
            metadata.label.clone()
        } else {
            let source = self.assets.index().products[&metadata.source]
                .editor
                .as_ref()
                .unwrap();
            format!("{} / {}", source.label, metadata.label)
        }
    }
    pub(super) fn open_picker(&mut self, field: &str) -> Result<Pos2> {
        self.frame(vec![]);
        let anchor = self
            .reveal(|output| {
                let values = texts(output);
                values
                    .iter()
                    .filter(|(text, pos)| {
                        pos.x > 970. && (text == field || text == &format!("{field}（多值）"))
                    })
                    .find_map(|(_, row)| {
                        values
                            .iter()
                            .find(|(_, pos)| (pos.y - row.y).abs() < 2. && pos.x > row.x + 20.)
                            .map(|(_, pos)| *pos)
                    })
            })
            .with_context(|| format!("common selector {field}"))?;
        self.frame(vec![Event::PointerMoved(anchor), button(anchor, true)]);
        self.frame(vec![button(anchor, false)]);
        Ok(anchor)
    }
    pub(super) fn select_asset(&mut self, field: &str, asset: AssetId) -> Result<()> {
        let label = self.asset_label(asset);
        let anchor = self.open_picker(field)?;
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .find(|(text, pos)| text == &label && pos.y > anchor.y + 3.)
            .with_context(|| format!("common model option {label}"))?
            .1;
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }
}
pub(super) fn scalar_control(output: &egui::FullOutput, field: &str) -> Result<Pos2> {
    texts(output)
        .into_iter()
        .find(|(text, pos)| pos.x > 970. && text.starts_with(field))
        .map(|(_, pos)| pos)
        .context("common scalar control")
}

impl Group {
    pub(super) fn select_reference(&mut self, field: &str, option: &str) -> Result<()> {
        let anchor = self.open_picker(field)?;
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .rev()
            .find(|(text, pos)| text.ends_with(option) && pos.x > 970. && pos.y > anchor.y + 3.)
            .with_context(|| format!("entity choice {option}: {:?}", texts(&output)))?
            .1;
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }

    pub(super) fn click_flag(&mut self, label: &str) -> Result<()> {
        self.frame(vec![]);
        let position = self.reveal(|output| {
            texts(output)
                .into_iter()
                .find(|(text, pos)| pos.x > 970. && text.starts_with(label))
                .map(|(_, pos)| pos)
        })?;
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }

    pub(super) fn click_text(&mut self, label: &str) -> Result<()> {
        let pos = self
            .reveal(|output| {
                texts(output)
                    .into_iter()
                    .rev()
                    .find(|(text, _)| text == label)
                    .map(|(_, pos)| pos)
            })
            .with_context(|| format!("menu item {label}"))?;
        self.frame(vec![Event::PointerMoved(pos), button(pos, true)]);
        self.frame(vec![button(pos, false)]);
        Ok(())
    }
}
