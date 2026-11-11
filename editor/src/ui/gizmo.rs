use crate::session::Editor;
use crate::transform::{GizmoTool, TransformSpace, rigid_axes, scale_basis};
use comet_asset::AssetStore;
use comet_core::math::{Mat4, Quat, Vec3};
use egui::{Color32, Pos2, Rect, Stroke, Vec2};
pub(super) fn controls(editor: &mut Editor, ui: &mut egui::Ui, allow_start: bool) {
    let playing = editor.play.is_some();
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(editor.drag_origin.is_none() && allow_start, |ui| {
            ui.selectable_value(
                &mut editor.gizmo_tool,
                GizmoTool::Translate,
                if playing { "移动" } else { "移动 W" },
            );
            ui.selectable_value(
                &mut editor.gizmo_tool,
                GizmoTool::Rotate,
                if playing { "旋转" } else { "旋转 E" },
            );
            ui.selectable_value(
                &mut editor.gizmo_tool,
                GizmoTool::Scale,
                if playing { "缩放" } else { "缩放 R" },
            );
            if matches!(editor.gizmo_tool, GizmoTool::Translate | GizmoTool::Rotate) {
                ui.separator();
                ui.selectable_value(&mut editor.transform_space, TransformSpace::World, "世界");
                ui.selectable_value(&mut editor.transform_space, TransformSpace::Local, "局部")
                    .on_hover_text(
                        "使用主选实体的层级旋转方向，忽略缩放和镜像；移动整组，或绕主选实体旋转整组。",
                    );
            }
            ui.separator();
            let (snap, label, help, speed, maximum, suffix) = match editor.gizmo_tool {
                GizmoTool::Translate => (
                    &mut editor.translation_snap,
                    "移动吸附",
                    if editor.transform_space == TransformSpace::World {
                        "拖动坐标轴时吸附到绝对世界网格，步长使用世界单位。"
                    } else {
                        "沿主选实体的局部方向移动，从拖动起点按位移步长吸附。"
                    },
                    0.05,
                    1000.,
                    "",
                ),
                GizmoTool::Rotate => (
                    &mut editor.rotation_snap,
                    "旋转吸附",
                    "绕当前坐标轴旋转，从拖动起点按角度步长吸附。",
                    1.,
                    180.,
                    "°",
                ),
                GizmoTool::Scale => (
                    &mut editor.scale_snap,
                    "缩放吸附",
                    "1×为拖动前大小，按倍率步长吸附。拖动中心方块可统一缩放。",
                    0.01,
                    1.,
                    "×",
                ),
            };
            ui.checkbox(&mut snap.enabled, label).on_hover_text(help);
            let mut step = snap.step();
            ui.label("步长");
            let response = ui.add_enabled(
                snap.enabled,
                egui::DragValue::new(&mut step)
                    .speed(speed)
                    .range(0.001..=maximum)
                    .max_decimals(3)
                    .suffix(suffix),
            );
            if response.changed() {
                let result = snap.set_step(step);
                editor.notice(result);
            }
            if editor.gizmo_tool == GizmoTool::Scale {
                ui.weak("1×为原大小");
            }
        });
    });
    if editor.play.is_some() && editor.selection.len() > 1 {
        ui.weak("运行期间暂不支持成组变换。");
    }
}

pub(super) fn draw(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    viewport: Rect,
    allow_start: bool,
) -> bool {
    let gesture_id = ui.scope_id().with("transform-gesture");
    if editor.drag_origin.is_none() {
        ui.data_mut(|data| data.remove::<Gesture>(gesture_id));
    }
    if !ui.is_enabled() {
        // Freeze the document while a modal owns input; retire this gesture when input resumes.
        ui.data_mut(|data| {
            if let Some(mut gesture) = data.get_temp::<Gesture>(gesture_id) {
                gesture.suspended = true;
                data.insert_temp(gesture_id, gesture);
            }
        });
        return false;
    }
    let projection = editor.camera.view_projection(viewport.aspect_ratio());
    if ui.data(|data| {
        data.get_temp::<Gesture>(gesture_id).is_some_and(|gesture| {
            gesture.suspended
                || gesture.kind.tool() != editor.gizmo_tool
                || gesture
                    .transform_space
                    .is_some_and(|space| space != editor.transform_space)
                || gesture.view.projection != projection
                || gesture.view.viewport != viewport
        })
    }) {
        ui.data_mut(|data| data.remove::<Gesture>(gesture_id));
        let result = editor.cancel_transform();
        editor.notice(result);
        return true;
    }
    // egui ends a drag on Escape; cancel before that stop can commit the preview.
    if editor.drag_origin.is_some() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|data| data.remove::<Gesture>(gesture_id));
        let result = editor.cancel_transform();
        editor.notice(result);
        return true;
    }
    draw_handles(editor, ui, assets, viewport, gesture_id, allow_start)
}

const TRANSLATION_LENGTH: f32 = 0.9;
// A press alone must not snap an off-grid object before the user starts moving.
const TRANSLATION_THRESHOLD: f32 = 3.;
const RING_RADIUS: f32 = 66.;
const RING_SEGMENTS: usize = 64;
const HIT_RADIUS: f32 = 7.;
const SCALE_LENGTH: f32 = 80.;
// Interaction limits keep a single drag positive and bounded; authored scale signs stay intact.
const MIN_SCALE_FACTOR: f64 = 1e-4;
const MAX_SCALE_FACTOR: f64 = 1e4;

#[derive(Clone, Copy)]
struct Gesture {
    kind: DragKind,
    transform_space: Option<TransformSpace>,
    view: HandleView,
    suspended: bool,
}

#[derive(Clone, Copy)]
struct HandleView {
    pivot: Vec3,
    projection: Mat4,
    viewport: Rect,
    length: f32,
    axes: Option<[Vec3; 3]>,
}

#[derive(Clone, Copy)]
enum DragKind {
    Translation {
        axis: usize,
        start: Pos2,
        direction: Vec2,
        reference: f32,
        moved: bool,
    },
    Rotation {
        axis: Vec3,
        start: Vec3,
        previous_angle: f32,
        angle: f32,
    },
    Scale {
        axis: Option<usize>,
        start: Pos2,
        direction: Vec2,
        reference: f32,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    Translation(usize),
    Rotation(Vec3),
    Scale(Option<usize>),
}

impl DragKind {
    fn tool(self) -> GizmoTool {
        match self {
            Self::Translation { .. } => GizmoTool::Translate,
            Self::Rotation { .. } => GizmoTool::Rotate,
            Self::Scale { .. } => GizmoTool::Scale,
        }
    }
    fn handle(self) -> Handle {
        match self {
            Self::Translation { axis, .. } => Handle::Translation(axis),
            Self::Rotation { axis, .. } => Handle::Rotation(axis),
            Self::Scale { axis, .. } => Handle::Scale(axis),
        }
    }
}

fn project(projection: Mat4, viewport: Rect, point: Vec3) -> Option<Pos2> {
    let clip = projection * point.extend(1.);
    if !clip.is_finite() || clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !(0. ..=1.).contains(&ndc.z) {
        return None;
    }
    let point = Pos2::new(
        viewport.center().x + ndc.x * viewport.width() * 0.5,
        viewport.center().y - ndc.y * viewport.height() * 0.5,
    );
    point.is_finite().then_some(point)
}

fn handle_length(projection: Mat4, viewport: Rect, pivot: Vec3, pixels: f32) -> Option<f32> {
    project(projection, viewport, pivot)?;
    let inverse = projection.inverse();
    if !inverse.is_finite() || viewport.width() <= 0. || viewport.height() <= 0. {
        return None;
    }
    let ndc = projection.project_point3(pivot);
    let offset = inverse.project_point3(ndc + Vec3::X * (2. * pixels / viewport.width()));
    let radius = offset.distance(pivot);
    (radius.is_finite() && radius > 1e-6).then_some(radius)
}

fn tool_length(tool: GizmoTool, projection: Mat4, viewport: Rect, pivot: Vec3) -> Option<f32> {
    match tool {
        GizmoTool::Translate => Some(TRANSLATION_LENGTH),
        GizmoTool::Rotate => handle_length(projection, viewport, pivot, RING_RADIUS),
        GizmoTool::Scale => handle_length(projection, viewport, pivot, SCALE_LENGTH),
    }
}

fn plane_vector(
    projection: Mat4,
    viewport: Rect,
    position: Pos2,
    pivot: Vec3,
    axis: Vec3,
) -> Option<Vec3> {
    if !position.is_finite() || viewport.width() <= 0. || viewport.height() <= 0. {
        return None;
    }
    let inverse = projection.inverse();
    let ndc = Vec3::new(
        (position.x - viewport.min.x) / viewport.width() * 2. - 1.,
        1. - (position.y - viewport.min.y) / viewport.height() * 2.,
        0.,
    );
    let near = inverse.project_point3(ndc);
    let far = inverse.project_point3(ndc.with_z(1.));
    let direction = (far - near).try_normalize()?;
    let denominator = direction.dot(axis);
    if !near.is_finite() || denominator.abs() < 1e-4 {
        return None;
    }
    let distance = (pivot - near).dot(axis) / denominator;
    if !distance.is_finite() || distance < 0. {
        return None;
    }
    (near + direction * distance - pivot).try_normalize()
}

fn segment_distance(position: Pos2, start: Pos2, end: Pos2) -> f32 {
    let direction = end - start;
    let length = direction.length_sq();
    if length <= 1e-6 {
        return position.distance(start);
    }
    let amount = ((position - start).dot(direction) / length).clamp(0., 1.);
    position.distance(start + direction * amount)
}

fn draw_handles(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    viewport: Rect,
    gesture_id: egui::Id,
    allow_start: bool,
) -> bool {
    let mut gesture = ui.data(|data| data.get_temp::<Gesture>(gesture_id));
    let view = if let Some(drag) = gesture {
        drag.view
    } else {
        let Some(entity) = editor.selected_entity() else {
            return false;
        };
        if editor.play.is_some() && (entity.physics.is_some() || editor.selection.len() > 1) {
            return false;
        }
        let Ok(transforms) = comet_scene::world_transforms(&editor.last_snapshot) else {
            return false;
        };
        let pivot = transforms[&entity.id].transform_point3(Vec3::ZERO);
        let axes = if editor.gizmo_tool == GizmoTool::Scale {
            let parent = entity.parent.map_or(Mat4::IDENTITY, |id| transforms[&id]);
            scale_basis(parent, Quat::from_array(entity.transform.rotation))
                .ok()
                .map(|basis| {
                    [
                        basis.x_axis.truncate(),
                        basis.y_axis.truncate(),
                        basis.z_axis.truncate(),
                    ]
                })
        } else {
            rigid_axes(&editor.last_snapshot, entity.id, editor.transform_space).ok()
        };
        let projection = editor.camera.view_projection(viewport.aspect_ratio());
        let Some(length) = tool_length(editor.gizmo_tool, projection, viewport, pivot) else {
            return false;
        };
        HandleView {
            pivot,
            projection,
            viewport,
            length,
            axes,
        }
    };
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let hover_pointer =
        pointer.filter(|point| viewport.contains(*point) && ui.rect_contains_pointer(viewport));
    let active = gesture.map(|drag| drag.kind.handle());
    // Arrows follow the moving object, while input retains its initial view even offscreen.
    let painting = if gesture.is_some() && editor.gizmo_tool == GizmoTool::Translate {
        editor.selection.active().and_then(|id| {
            let transforms = comet_scene::world_transforms(&editor.last_snapshot).ok()?;
            Some(HandleView {
                pivot: transforms.get(&id)?.transform_point3(Vec3::ZERO),
                ..view
            })
        })
    } else {
        Some(view)
    };
    let hovered = painting.and_then(|painting| match editor.gizmo_tool {
        GizmoTool::Translate => translation_handles(ui, painting, active, hover_pointer),
        GizmoTool::Rotate => rotation_handles(ui, painting, active, hover_pointer),
        GizmoTool::Scale => scale_handles(ui, painting, active, hover_pointer),
    });
    if gesture.is_none()
        && allow_start
        && editor.drag_origin.is_none()
        && let Some(handle) = hovered
        && ui.input(|input| {
            input.pointer.primary_pressed()
                && !input.pointer.secondary_down()
                && !input.pointer.button_down(egui::PointerButton::Middle)
        })
        && let Some(pointer) = pointer
    {
        let result = match handle {
            Handle::Translation(_) => editor.begin_translation(assets),
            Handle::Rotation(_) => editor.begin_rotation(assets),
            Handle::Scale(_) => editor.begin_scale(assets),
        };
        if result.is_ok() {
            gesture = editor.drag_origin.as_ref().and_then(|drag| {
                let view = HandleView {
                    pivot: drag.anchor,
                    axes: drag.axes,
                    length: tool_length(editor.gizmo_tool, view.projection, viewport, drag.anchor)?,
                    ..view
                };
                let kind = start_drag(view, handle, pointer)?;
                Some(Gesture {
                    kind,
                    transform_space: (editor.gizmo_tool != GizmoTool::Scale)
                        .then_some(editor.transform_space),
                    view,
                    suspended: false,
                })
            });
            if gesture.is_none() {
                let cancel = editor.cancel_transform();
                editor.notice(cancel);
            }
        }
        editor.notice(result);
    }
    let Some(mut drag) = gesture else {
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        return hovered.is_some();
    };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let released = ui.input(|input| input.pointer.primary_released());
    let pressed = ui.input(|input| input.pointer.primary_down());
    let Some(pointer) = pointer.filter(|point| point.is_finite() && (pressed || released)) else {
        ui.data_mut(|data| data.remove::<Gesture>(gesture_id));
        let result = editor.cancel_transform();
        editor.notice(result);
        return true;
    };
    let result = drag.preview(editor, assets, pointer);
    if result.is_err() {
        ui.data_mut(|data| data.remove::<Gesture>(gesture_id));
        let cancel = editor.cancel_transform();
        editor.notice(cancel);
        editor.notice(result);
    } else if released {
        ui.data_mut(|data| data.remove::<Gesture>(gesture_id));
        let result = editor.commit();
        editor.notice(result);
    } else {
        ui.data_mut(|data| data.insert_temp(gesture_id, drag));
    }
    true
}

fn translation_handles(
    ui: &mut egui::Ui,
    view: HandleView,
    active: Option<Handle>,
    pointer: Option<Pos2>,
) -> Option<Handle> {
    let axes = view.axes?;
    let start = project(view.projection, view.viewport, view.pivot)?;
    let painter = ui.painter().with_clip_rect(view.viewport);
    let mut hovered = None;
    let mut closest = HIT_RADIUS;
    for (index, (axis, (color, label))) in axes
        .into_iter()
        .zip([
            (Color32::from_rgb(240, 101, 108), "X"),
            (Color32::from_rgb(122, 211, 144), "Y"),
            (Color32::from_rgb(109, 165, 244), "Z"),
        ])
        .enumerate()
    {
        let Some(end) = project(
            view.projection,
            view.viewport,
            view.pivot + axis * view.length,
        ) else {
            continue;
        };
        let direction = end - start;
        let length = direction.length();
        if !length.is_finite() || length < 5. {
            continue;
        }
        let handle = Handle::Translation(index);
        let hit = pointer.is_some_and(|pointer| {
            let distance = segment_distance(pointer, start + direction * 0.25, end);
            if distance < closest {
                closest = distance;
                hovered = Some(handle);
                true
            } else {
                false
            }
        });
        painter.arrow(
            start,
            direction,
            Stroke::new(
                if hit || active == Some(handle) {
                    4.
                } else {
                    2.5
                },
                color,
            ),
        );
        painter.text(
            end + direction / length * 10.,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.),
            color,
        );
    }
    hovered
}

fn rotation_handles(
    ui: &mut egui::Ui,
    view: HandleView,
    active: Option<Handle>,
    pointer: Option<Pos2>,
) -> Option<Handle> {
    let mut hovered = None;
    let mut closest = HIT_RADIUS;
    let painter = ui.painter().with_clip_rect(view.viewport);
    let [x, y, z] = view.axes?;
    for (axis, u, v, color) in [
        (x, y, z, Color32::from_rgb(240, 101, 108)),
        (y, z, x, Color32::from_rgb(122, 211, 144)),
        (z, x, y, Color32::from_rgb(109, 165, 244)),
    ] {
        let accepts_pointer = pointer.is_some_and(|pointer| {
            plane_vector(view.projection, view.viewport, pointer, view.pivot, axis).is_some()
        });
        let stroke = Stroke::new(
            if active == Some(Handle::Rotation(axis)) {
                3.5
            } else {
                2.
            },
            color,
        );
        let mut previous = None;
        for index in 0..=RING_SEGMENTS {
            let angle = std::f32::consts::TAU * index as f32 / RING_SEGMENTS as f32;
            let point = project(
                view.projection,
                view.viewport,
                view.pivot + view.length * (u * angle.cos() + v * angle.sin()),
            );
            if let (Some(start), Some(end)) = (previous, point) {
                painter.line_segment([start, end], stroke);
                if accepts_pointer && let Some(pointer) = pointer {
                    let distance = segment_distance(pointer, start, end);
                    if distance < closest {
                        closest = distance;
                        hovered = Some(Handle::Rotation(axis));
                    }
                }
            }
            previous = point;
        }
    }
    hovered
}

fn scale_handles(
    ui: &mut egui::Ui,
    view: HandleView,
    active: Option<Handle>,
    pointer: Option<Pos2>,
) -> Option<Handle> {
    let start = project(view.projection, view.viewport, view.pivot)?;
    let painter = ui.painter().with_clip_rect(view.viewport);
    let center = Rect::from_center_size(start, Vec2::splat(11.));
    let center_hovered = pointer.is_some_and(|point| center.expand(3.).contains(point));
    let mut hovered = center_hovered.then_some(Handle::Scale(None));
    let mut closest = HIT_RADIUS;
    if let Some(axes) = view.axes {
        for (index, color) in [
            Color32::from_rgb(240, 101, 108),
            Color32::from_rgb(122, 211, 144),
            Color32::from_rgb(109, 165, 244),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(end) = project(
                view.projection,
                view.viewport,
                view.pivot + axes[index] * view.length,
            ) else {
                continue;
            };
            let direction = end - start;
            if direction.length() < 5. {
                continue;
            }
            let line_start = start + direction * 0.25;
            let selected = active == Some(Handle::Scale(Some(index)));
            painter.line_segment(
                [line_start, end],
                Stroke::new(if selected { 3.5 } else { 2. }, color),
            );
            painter.rect_filled(
                Rect::from_center_size(end, Vec2::splat(if selected { 10. } else { 8. })),
                0.,
                color,
            );
            painter.text(
                end + direction.normalized() * 12.,
                egui::Align2::CENTER_CENTER,
                ["X", "Y", "Z"][index],
                egui::FontId::proportional(12.),
                color,
            );
            if !center_hovered && let Some(pointer) = pointer {
                let distance = segment_distance(pointer, line_start, end);
                if distance < closest {
                    closest = distance;
                    hovered = Some(Handle::Scale(Some(index)));
                }
            }
        }
    }
    let center_color = if active == Some(Handle::Scale(None)) || center_hovered {
        Color32::from_rgb(236, 185, 99)
    } else {
        Color32::from_gray(220)
    };
    painter.rect_filled(center, 0., center_color);
    hovered
}

fn start_drag(view: HandleView, handle: Handle, pointer: Pos2) -> Option<DragKind> {
    match handle {
        Handle::Translation(index) => {
            let start = project(view.projection, view.viewport, view.pivot)?;
            let end = project(
                view.projection,
                view.viewport,
                view.pivot + view.axes?[index] * view.length,
            )?;
            let direction = end - start;
            let reference = direction.length();
            if !reference.is_finite() || reference < 5. {
                return None;
            }
            Some(DragKind::Translation {
                axis: index,
                start: pointer,
                direction: direction / reference,
                reference,
                moved: false,
            })
        }
        Handle::Rotation(axis) => Some(DragKind::Rotation {
            axis,
            start: plane_vector(view.projection, view.viewport, pointer, view.pivot, axis)?,
            previous_angle: 0.,
            angle: 0.,
        }),
        Handle::Scale(axis) => {
            let (direction, reference) = if let Some(index) = axis {
                let start = project(view.projection, view.viewport, view.pivot)?;
                let end = project(
                    view.projection,
                    view.viewport,
                    view.pivot + view.axes?[index] * view.length,
                )?;
                let direction = end - start;
                let reference = direction.length();
                if reference < 5. {
                    return None;
                }
                (direction / reference, reference)
            } else {
                (Vec2::new(1., -1.).normalized(), SCALE_LENGTH)
            };
            Some(DragKind::Scale {
                axis,
                start: pointer,
                direction,
                reference,
            })
        }
    }
}

impl Gesture {
    fn preview(
        &mut self,
        editor: &mut Editor,
        assets: &mut AssetStore,
        pointer: Pos2,
    ) -> anyhow::Result<()> {
        match &mut self.kind {
            DragKind::Translation {
                axis,
                start,
                direction,
                reference,
                moved,
            } => {
                let dx = f64::from(pointer.x) - f64::from(start.x);
                let dy = f64::from(pointer.y) - f64::from(start.y);
                if !*moved && dx.hypot(dy) < f64::from(TRANSLATION_THRESHOLD) {
                    return Ok(());
                }
                *moved = true;
                let axis = self
                    .view
                    .axes
                    .ok_or_else(|| anyhow::anyhow!("移动坐标轴已失效"))?[*axis];
                let mut amount = (dx * f64::from(direction.x) + dy * f64::from(direction.y))
                    / f64::from(*reference)
                    * f64::from(self.view.length);
                if editor.translation_snap.enabled {
                    let space = self
                        .transform_space
                        .ok_or_else(|| anyhow::anyhow!("移动坐标系已失效"))?;
                    let origin = if space == TransformSpace::World {
                        self.view
                            .pivot
                            .to_array()
                            .into_iter()
                            .zip(axis.to_array())
                            .map(|(position, axis)| f64::from(position) * f64::from(axis))
                            .sum()
                    } else {
                        0.
                    };
                    let step = f64::from(editor.translation_snap.step());
                    amount = ((origin + amount) / step).round() * step - origin;
                }
                let amount = amount as f32;
                anyhow::ensure!(amount.is_finite(), "移动位移超出有效范围");
                editor.preview_translation(axis * amount, assets)
            }
            DragKind::Rotation {
                axis,
                start,
                previous_angle,
                angle,
            } => {
                let direction = plane_vector(
                    self.view.projection,
                    self.view.viewport,
                    pointer,
                    self.view.pivot,
                    *axis,
                )
                .ok_or_else(|| anyhow::anyhow!("旋转平面已退化，请重新拖动"))?;
                let current = axis.dot(start.cross(direction)).atan2(start.dot(direction));
                // The ray always uses the starting plane; unwrap only its ±π discontinuity.
                let difference = current - *previous_angle;
                *angle += difference.sin().atan2(difference.cos());
                *previous_angle = current;
                let applied = if editor.rotation_snap.enabled {
                    let step = editor.rotation_snap.step() as f64;
                    (((*angle as f64).to_degrees() / step).round() * step).to_radians() as f32
                } else {
                    *angle
                };
                editor.preview_rotation(*axis, applied, assets)
            }
            DragKind::Scale {
                axis,
                start,
                direction,
                reference,
            } => {
                let distance = (pointer.x as f64 - start.x as f64) * direction.x as f64
                    + (pointer.y as f64 - start.y as f64) * direction.y as f64;
                let mut factor =
                    (1. + distance / *reference as f64).clamp(MIN_SCALE_FACTOR, MAX_SCALE_FACTOR);
                if editor.scale_snap.enabled {
                    let step = editor.scale_snap.step() as f64;
                    // Snap around 1× and choose only positive grid points, never the zero crossing.
                    let minimum = ((MIN_SCALE_FACTOR - 1.) / step).ceil();
                    let maximum = ((MAX_SCALE_FACTOR - 1.) / step).floor();
                    let steps = ((factor - 1.) / step).round().clamp(minimum, maximum);
                    factor = (1. + steps * step).clamp(MIN_SCALE_FACTOR, MAX_SCALE_FACTOR);
                }
                let mut factors = Vec3::ONE;
                if let Some(index) = axis {
                    factors[*index] = factor as f32;
                } else {
                    factors = Vec3::splat(factor as f32);
                }
                editor.preview_scale(factors, assets)
            }
        }
    }
}
