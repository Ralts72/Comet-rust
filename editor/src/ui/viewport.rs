use super::{Editor, SelectionPick, gizmo};
use crate::{
    selection::SelectMode,
    transform::{GizmoTool, TransformSpace},
};
use anyhow::Context;
use comet_asset::AssetStore;
use comet_core::{ActionInput, WorldId, math::Mat4};
use comet_render::PickRequest;
use egui::{Color32, Pos2, Rect, Vec2};

const DRAG_THRESHOLD: f32 = 3.;

fn gesture_id() -> egui::Id {
    egui::Id::unique("viewport-selection-gesture")
}

#[derive(Clone)]
struct GestureContext {
    world: WorldId,
    target: u64,
    selection: u64,
    document: u64,
    products: String,
    tool: GizmoTool,
    space: TransformSpace,
    projection: Mat4,
    viewport: Rect,
}

impl GestureContext {
    fn capture(editor: &Editor, assets: &AssetStore, viewport: Rect) -> Self {
        Self {
            world: editor.world(),
            target: editor.target_generation,
            selection: editor.selection.generation(),
            document: editor.document.revision(),
            products: assets.revision().to_owned(),
            tool: editor.gizmo_tool,
            space: editor.transform_space,
            projection: editor.camera.view_projection(viewport.aspect_ratio()),
            viewport,
        }
    }

    fn valid(&self, editor: &Editor, assets: &AssetStore, viewport: Rect, active: bool) -> bool {
        active
            && viewport.width() > 0.
            && viewport.height() > 0.
            && editor.play.is_none()
            && !editor.is_busy()
            && !editor.documents.is_blocking()
            && !editor.document.is_editing()
            && editor.drag_origin.is_none()
            && self.world == editor.world()
            && self.target == editor.target_generation
            && self.selection == editor.selection.generation()
            && self.document == editor.document.revision()
            && self.products == assets.revision()
            && self.tool == editor.gizmo_tool
            && self.space == editor.transform_space
            && self.viewport == viewport
            && self.projection == editor.camera.view_projection(viewport.aspect_ratio())
    }
}

#[derive(Clone)]
enum PointerGesture {
    Selection {
        context: GestureContext,
        start: Pos2,
        current: Pos2,
        dragged: bool,
    },
    Canceled,
}

pub(crate) fn cancel_gesture(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if data.get_temp::<PointerGesture>(gesture_id()).is_some() {
            data.insert_temp(gesture_id(), PointerGesture::Canceled);
        }
    });
}

pub(super) struct ViewportResponse {
    viewport: Rect,
    pick: Option<SelectionPick>,
    actions: ActionInput,
    pub owns_pointer: bool,
    rectangle_context: Option<GestureContext>,
    response: Option<egui::Response>,
}

pub(crate) struct ViewportFrame {
    pub viewport: Rect,
    pub pick: Option<SelectionPick>,
    pub actions: ActionInput,
    pub response: egui::Response,
}

impl ViewportResponse {
    pub fn finish(
        mut self,
        editor: &Editor,
        assets: &AssetStore,
        ctx: &egui::Context,
        active: bool,
    ) -> anyhow::Result<ViewportFrame> {
        // Shortcuts can change selection or open a modal after the viewport was drawn.
        let valid = |context: &GestureContext| context.valid(editor, assets, self.viewport, active);
        if self
            .rectangle_context
            .as_ref()
            .is_some_and(|context| !valid(context))
        {
            self.pick = None;
        }
        if ctx.data(|data| {
            matches!(data.get_temp::<PointerGesture>(gesture_id()),
                Some(PointerGesture::Selection { context, .. }) if !valid(&context))
        }) {
            cancel_gesture(ctx);
        }
        Ok(ViewportFrame {
            viewport: self.viewport,
            pick: self.pick,
            actions: self.actions,
            response: self.response.context("缺少视口输入响应")?,
        })
    }
}

pub(super) fn show(
    editor: &mut Editor,
    root: &mut egui::Ui,
    assets: &mut AssetStore,
    active: bool,
) -> ViewportResponse {
    let mut result = ViewportResponse {
        viewport: Rect::NOTHING,
        pick: None,
        actions: ActionInput::default(),
        owns_pointer: false,
        rectangle_context: None,
        response: None,
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(root, |ui| {
            let mut gesture = ui.data(|data| data.get_temp::<PointerGesture>(gesture_id()));
            gizmo::controls(editor, ui, gesture.is_none());
            let viewport = ui.available_rect_before_wrap();
            result.viewport = viewport;
            let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
            if response.clicked() {
                response.request_focus();
            }
            result.response = Some(response.clone());
            comet_application::orbit(&mut editor.camera, &response);
            if editor.play.is_some() {
                result.actions =
                    comet_application::sample_actions(&response, active, editor.camera);
            }
            ui.painter().text(
                viewport.left_top() + Vec2::new(18., 16.),
                egui::Align2::LEFT_TOP,
                "透视 / 光照",
                egui::FontId::proportional(12.),
                Color32::from_gray(190),
            );
            ui.painter().text(
                viewport.left_bottom() + Vec2::new(18., -18.),
                egui::Align2::LEFT_BOTTOM,
                if editor.play.is_some() {
                    "WASD：移动 · 空格：跳跃 · Shift：跑步 · Ctrl：蹲伏 · 右键：环绕"
                } else {
                    "右键拖动：环绕 · 滚轮：缩放 · 拖动手柄：变换 · 左键拖动：框选"
                },
                egui::FontId::proportional(11.),
                Color32::from_gray(160),
            );
            if let Some(play) = &editor.play {
                ui.painter().text(
                    viewport.left_bottom() + Vec2::new(18., -34.),
                    egui::Align2::LEFT_BOTTOM,
                    if play.scene_asset().is_some() {
                        "F5：保存预览检查点 · F9：继续 · 停止后清空"
                    } else {
                        "Prefab 预览不支持检查点"
                    },
                    egui::FontId::proportional(11.),
                    Color32::from_gray(160),
                );
            }
            let enabled = active && ui.is_enabled();
            if let Some(PointerGesture::Selection { context, .. }) = &gesture
                && (!context.valid(editor, assets, viewport, enabled)
                    || ui.input(|input| input.key_pressed(egui::Key::Escape)))
            {
                gesture = Some(PointerGesture::Canceled);
            }
            let gizmo = gizmo::draw(editor, ui, assets, viewport, gesture.is_none() && enabled);
            let (pressed, held, released, pointer) = ui.input(|input| {
                (
                    input.pointer.primary_pressed(),
                    input.pointer.primary_down(),
                    input.pointer.primary_released(),
                    input.pointer.interact_pos(),
                )
            });
            if gesture.is_none()
                && !gizmo
                && enabled
                && viewport.width() > 0.
                && viewport.height() > 0.
                && editor.play.is_none()
                && !editor.documents.is_blocking()
                && !editor.document.is_editing()
                && editor.drag_origin.is_none()
                && pressed
                && (response.contains_pointer() || response.is_pointer_button_down_on())
                && !ui.input(|input| {
                    input.pointer.secondary_down()
                        || input.pointer.button_down(egui::PointerButton::Middle)
                })
                && let Some(start) = ui.input(|input| input.pointer.press_origin())
                && start.is_finite()
                && viewport.contains(start)
            {
                gesture = Some(PointerGesture::Selection {
                    context: GestureContext::capture(editor, assets, viewport),
                    start,
                    current: start,
                    dragged: false,
                });
            }
            result.owns_pointer = gesture.is_some();
            match &mut gesture {
                Some(PointerGesture::Selection {
                    context,
                    start,
                    current,
                    dragged,
                }) => {
                    if let Some(pointer) =
                        pointer.filter(|point| point.is_finite() && (held || released))
                    {
                        *current = pointer;
                        if !*dragged && current.distance(*start) >= DRAG_THRESHOLD {
                            *dragged = true;
                            editor.picking.clear();
                        }
                        if *dragged {
                            let rect =
                                Rect::from_two_pos(*start, *current).intersect(context.viewport);
                            let painter = ui.painter().with_clip_rect(context.viewport);
                            painter.rect_filled(
                                rect,
                                0.,
                                Color32::from_rgba_unmultiplied(97, 174, 241, 32),
                            );
                            painter.rect_stroke(
                                rect,
                                0.,
                                egui::Stroke::new(1., Color32::from_rgb(97, 174, 241)),
                                egui::StrokeKind::Inside,
                            );
                            if released && rect.width() > 0. && rect.height() > 0. {
                                result.pick = Some(selection_pick(
                                    editor,
                                    ui,
                                    PickRequest::Rect {
                                        min: uv(rect.min, context.viewport),
                                        max: uv(rect.max, context.viewport),
                                    },
                                ));
                                result.rectangle_context = Some(context.clone());
                            }
                        } else if released && response.clicked() {
                            result.pick = Some(selection_pick(
                                editor,
                                ui,
                                PickRequest::Point(uv(*current, viewport)),
                            ));
                        }
                        if released {
                            gesture = None;
                        }
                    } else {
                        gesture = if held {
                            Some(PointerGesture::Canceled)
                        } else {
                            None
                        };
                    }
                }
                Some(PointerGesture::Canceled) => {
                    if released || !held {
                        gesture = None;
                    }
                }
                None => {
                    if response.clicked()
                        && !gizmo
                        && editor.play.is_some()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        result.pick = Some(selection_pick(
                            editor,
                            ui,
                            PickRequest::Point(uv(pointer, viewport)),
                        ));
                    }
                }
            }
            ui.data_mut(|data| {
                if let Some(gesture) = gesture {
                    data.insert_temp(gesture_id(), gesture);
                } else {
                    data.remove::<PointerGesture>(gesture_id());
                }
            });
        });
    result
}

fn uv(point: Pos2, viewport: Rect) -> [f32; 2] {
    [
        ((point.x - viewport.min.x) / viewport.width()).clamp(0., 1.),
        ((point.y - viewport.min.y) / viewport.height()).clamp(0., 1.),
    ]
}

fn selection_pick(editor: &Editor, ui: &egui::Ui, request: PickRequest) -> SelectionPick {
    let modifiers = click_modifiers(ui, egui::PointerButton::Primary);
    SelectionPick {
        request,
        mode: if modifiers.command || modifiers.ctrl || modifiers.shift {
            SelectMode::Toggle
        } else {
            SelectMode::Replace
        },
        selection_generation: editor.selection.generation(),
    }
}

pub(super) fn click_modifiers(ui: &egui::Ui, button: egui::PointerButton) -> egui::Modifiers {
    ui.input(|input| {
        input
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::PointerButton {
                    button: released,
                    pressed: false,
                    modifiers,
                    ..
                } if *released == button => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(input.modifiers)
    })
}
