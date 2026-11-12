use crate::session::Editor;
use anyhow::Context;
use comet_application::{Camera, Projection};
use comet_asset::AssetStore;
use comet_core::{WorldId, math::Vec3};

pub(super) enum Action {
    Projection(bool),
    Selection,
    All,
}
#[derive(Clone, Copy)]
struct PerspectiveDirection {
    world: WorldId,
    direction: Vec3,
}
fn direction_id() -> egui::Id {
    egui::Id::unique("viewport-perspective-direction")
}

pub(super) fn controls(editor: &Editor, ui: &mut egui::Ui, allowed: bool) -> Option<Action> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(allowed && editable(editor), |ui| {
            let orthographic = matches!(editor.camera.projection, Projection::Orthographic { .. });
            ui.menu_button(
                if orthographic {
                    "投影：2D"
                } else {
                    "投影：3D"
                },
                |ui| {
                    for (value, label) in [(true, "2D 正交"), (false, "3D 透视")] {
                        if ui.selectable_label(orthographic == value, label).clicked() {
                            action = Some(Action::Projection(value));
                            ui.close();
                        }
                    }
                },
            );
            if ui
                .add_enabled(
                    !editor.selection.is_empty(),
                    egui::Button::new("聚焦选中 F"),
                )
                .clicked()
            {
                action = Some(Action::Selection);
            }
            if ui.small_button("聚焦全部").clicked() {
                action = Some(Action::All);
            }
        });
    });
    action
}

fn editable(editor: &Editor) -> bool {
    editor.play.is_none()
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && !editor.document.is_editing()
        && editor.drag_origin.is_none()
}

pub(super) fn apply(
    editor: &mut Editor,
    ui: &egui::Ui,
    assets: &mut AssetStore,
    response: &egui::Response,
    mut action: Option<Action>,
    allowed: bool,
) {
    if !allowed || !ui.is_enabled() || !editable(editor) {
        return;
    }
    if !ui.ctx().text_edit_focused()
        && (response.hovered() || response.has_focus())
        && !ui.ctx().egui_is_using_pointer()
    {
        ui.ctx().input_mut(|input| {
            input.events.retain(|event| {
                let focus = matches!(event, egui::Event::Key {
                    key: egui::Key::F, pressed: true, repeat: false, modifiers, ..
                } if modifiers.is_none());
                if focus && action.is_none() {
                    action = Some(Action::Selection);
                }
                !focus
            });
        });
    }
    let Some(action) = action else {
        return;
    };
    let result = match action {
        Action::Projection(orthographic) => set_projection(editor, ui.ctx(), orthographic),
        Action::Selection | Action::All => {
            let ids = match action {
                Action::All => editor
                    .last_snapshot
                    .entities
                    .iter()
                    .map(|entity| entity.id)
                    .collect(),
                _ => editor.selection.ids(),
            };
            comet_application::selection_bounds(&editor.last_snapshot, assets, &ids).and_then(
                |bounds| {
                    if let Some(bounds) = bounds {
                        let next = editor
                            .camera
                            .fit_bounds(bounds, response.rect.aspect_ratio())?;
                        editor.camera = next;
                        editor.picking.clear();
                    }
                    Ok(())
                },
            )
        }
    };
    editor.notice(result.context("无法调整场景相机"));
}

fn set_projection(
    editor: &mut Editor,
    ctx: &egui::Context,
    orthographic: bool,
) -> anyhow::Result<()> {
    let mut next = editor.camera;
    let was_orthographic = matches!(next.projection, Projection::Orthographic { .. });
    if was_orthographic == orthographic {
        return Ok(());
    }
    let offset = next.eye - next.target;
    let direction = if orthographic {
        next.eye = next.target + Vec3::Z * offset.length();
        next.projection = Projection::Orthographic {
            height: next.visible_height(),
        };
        Some(PerspectiveDirection {
            world: editor.world(),
            direction: offset.normalize(),
        })
    } else {
        let direction = ctx
            .data(|data| data.get_temp::<PerspectiveDirection>(direction_id()))
            .filter(|state| state.world == editor.world())
            .map_or_else(
                || (Camera::default().eye - Camera::default().target).normalize(),
                |state| state.direction,
            );
        next.eye = next.target + direction * offset.length();
        next.projection = Projection::Perspective;
        None
    };
    next.validate()?;
    if let Some(direction) = direction {
        ctx.data_mut(|data| data.insert_temp(direction_id(), direction));
    }
    editor.camera = next;
    editor.picking.clear();
    Ok(())
}
