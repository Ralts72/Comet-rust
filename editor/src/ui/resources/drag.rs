use super::can_place;
use crate::{resources::ResourceVersion, session::Editor};
use anyhow::Context;
use comet_asset::AssetStore;

#[derive(Clone)]
pub(in crate::ui) struct ResourceDrag {
    pub(super) source: comet_asset_pipeline::SourceAsset,
    pub(super) version: ResourceVersion,
    pub(super) target: u64,
}
impl ResourceDrag {
    fn valid(&self, editor: &Editor, assets: &AssetStore) -> bool {
        can_place(editor, &self.source)
            && !editor.document.is_editing()
            && editor.drag_origin.is_none()
            && self.target == editor.target_generation
            && self.version.matches(&editor.document, assets)
    }
}

pub(in crate::ui) fn cancel_drag(ctx: &egui::Context) {
    if egui::DragAndDrop::has_payload_of_type::<ResourceDrag>(ctx) {
        egui::DragAndDrop::clear_payload(ctx);
    }
}

pub(in crate::ui) fn finish_drag(
    editor: &mut Editor,
    assets: &mut AssetStore,
    ctx: &egui::Context,
    active: bool,
    view: &mut crate::ui::viewport::ViewportFrame,
) {
    let Some(drag) = egui::DragAndDrop::payload::<ResourceDrag>(ctx) else {
        return;
    };
    view.pick = None;
    view.actions = Default::default();
    editor.picking.clear();
    if !active || !view.response.enabled() || !drag.valid(editor, assets) {
        cancel_drag(ctx);
        return;
    }
    if view.visible
        && ctx.input(|input| input.pointer.primary_released())
        && view
            .response
            .dnd_release_payload::<ResourceDrag>()
            .is_some()
    {
        let result = (|| {
            let pointer = ctx
                .input(|input| input.pointer.interact_pos())
                .context("缺少资源放置位置")?;
            let uv = [
                (pointer.x - view.viewport.left()) / view.viewport.width(),
                (pointer.y - view.viewport.top()) / view.viewport.height(),
            ];
            let position = editor
                .camera
                .focus_plane_point(uv, view.viewport.aspect_ratio())
                .context("当前相机无法确定资源放置位置")?;
            editor.place_resource(
                drag.source.id,
                &drag
                    .source
                    .source
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy(),
                position.to_array(),
                assets,
            )
        })();
        editor.notice(result);
    }
}
