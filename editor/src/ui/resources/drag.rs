use super::{can_place, place};
use crate::{resources::ResourceVersion, session::Editor};
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
        let result = place(editor, &drag.source, assets);
        editor.notice(result);
    }
}
