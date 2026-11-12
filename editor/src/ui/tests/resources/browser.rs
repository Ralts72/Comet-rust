use super::*;
use crate::ui::resources::ResourceDrag;
use comet_asset_data::product::AssetKind;

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    }
}

fn begin_drag(c: &mut Case) -> Pos2 {
    let (view, output) = c.frame_result(vec![]);
    let from = position(&output, c.resource, resources());
    let to = view.viewport.lerp_inside(egui::vec2(0.7, 0.4));
    c.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
    c.frame(vec![egui::Event::PointerMoved(from + Vec2::new(8., -8.))]);
    assert!(egui::DragAndDrop::has_payload_of_type::<ResourceDrag>(
        &c.ctx
    ));
    c.frame(vec![egui::Event::PointerMoved(to)]);
    to
}

#[test]
fn project_resource_drag_places_real_roots_once_and_rejects_changed_contexts() -> Result<()> {
    let mut c = Case::new(true)?;
    let authoring = c.editor.document.shared_source();
    let to = begin_drag(&mut c);
    let view = c.frame_result(vec![button(to, false)]).0;
    assert!(view.pick.is_none());
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let placed = c.editor.document.shared_source();
    let expected = c
        .editor
        .camera
        .focus_plane_point([0.7, 0.4], view.viewport.aspect_ratio())
        .unwrap();
    assert!(
        comet_core::math::Vec3::from(placed.instances[0].translation).distance(expected) < 1e-3
    );
    assert_eq!(placed.version, 2);
    assert_eq!(placed.instances.len(), 1);
    assert!(placed.base.entities.is_empty());
    assert_eq!(c.editor.document.history_len(), 1);
    assert_eq!(
        c.editor
            .selection
            .ids()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        c.roots(placed.instances[0].id).into_iter().collect()
    );
    c.frame(vec![button(to, false)]);
    assert_eq!(c.editor.document.history_len(), 1);
    c.editor.document.undo()?;
    assert_eq!(c.editor.document.shared_source(), authoring);
    c.editor.document.redo()?;
    assert_eq!(c.editor.document.shared_source(), placed);
    for (path, bytes) in &c.sources {
        assert_eq!(fs::read(path)?, *bytes);
    }

    for rejected in [
        "document",
        "index",
        "play",
        "modal",
        "inactive",
        "escape",
        "outside",
        "target",
        "layout",
        "secondary",
    ] {
        let mut c = Case::new(false)?;
        let to = begin_drag(&mut c);
        match rejected {
            "document" => c.editor.document = Document::open(Default::default())?,
            // Identical product bytes in another Store are still another accepted resource view.
            "index" => c.assets = AssetStore::open(&c.editor.products)?,
            "play" => c.editor.start(&mut c.assets)?,
            "modal" => c
                .editor
                .documents
                .open_browser(crate::documents::BrowserMode::Open),
            "escape" => {
                c.key(egui::Key::Escape, Default::default());
            }
            "layout" => c
                .editor
                .workspace
                .focus(crate::ui::workspace::Pane::Inspector),
            _ => {}
        }
        let accepted = c.editor.document.shared_source();
        let history = c.editor.document.history_len();
        let destination = if rejected == "outside" {
            Pos2::new(750., 670.)
        } else {
            to
        };
        let release = if rejected == "secondary" {
            egui::Event::PointerButton {
                pos: destination,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: Default::default(),
            }
        } else {
            button(destination, false)
        };
        let (view, _) = frame_with_output(
            &c.ctx,
            &mut c.editor,
            &mut c.assets,
            FrameContext {
                number: c.number,
                dt: 1. / 60.,
                active: rejected != "inactive",
                activity_changed: rejected == "inactive",
                target_generation: if rejected == "target" { 2 } else { 1 },
                size: [1280, 800],
                smoke: false,
            },
            vec![egui::Event::PointerMoved(destination), release],
        );
        c.number += 1;
        assert!(view.pick.is_none(), "{rejected}");
        assert!(
            Arc::ptr_eq(&accepted, &c.editor.document.shared_source()),
            "{rejected}"
        );
        assert_eq!(c.editor.document.history_len(), history, "{rejected}");
        assert!(
            !egui::DragAndDrop::has_payload_of_type::<ResourceDrag>(&c.ctx),
            "{rejected}"
        );
    }
    Ok(())
}

fn nested_model() -> Result<Case> {
    let mut c = Case::new(false)?;
    let assets = c.editor.project_root().join("assets");
    let directory = assets.join("模型 集");
    fs::create_dir(&directory)?;
    for name in ["showcase.gltf", "showcase.gltf.meta", "showcase.bin"] {
        fs::rename(assets.join(name), directory.join(name))?;
    }
    c.editor.refresh_resources()?;
    c.wait_resources()?;
    assert!(
        c.editor.resource_build.failure.is_none(),
        "{:?}",
        c.editor.error
    );
    Ok(c)
}

fn search(c: &mut Case, query: &str) {
    c.press("清除", resources());
    let output = c.frame(vec![]);
    c.click(
        position(&output, "搜索文件...", resources()),
        Default::default(),
    );
    c.frame(vec![egui::Event::Text(query.into())]);
    c.frame(vec![]);
}

#[test]
fn project_tree_filters_paths_and_derived_assets_and_refreshes_without_editing() -> Result<()> {
    let mut c = nested_model()?;
    let authoring = c.editor.document.shared_source();
    let material = c
        .assets
        .index()
        .products
        .iter()
        .find(|(_, entry)| {
            entry.kind == AssetKind::Material
                && entry
                    .editor
                    .as_ref()
                    .is_some_and(|origin| origin.label == "Amber")
        })
        .unwrap()
        .0
        .to_owned();
    search(&mut c, "aMbEr");
    let output = c.frame(vec![]);
    position(&output, "模型 集", Rect::EVERYTHING);
    position(&output, "showcase.gltf", Rect::EVERYTHING);
    c.press("材质  Amber", Rect::EVERYTHING);
    assert_eq!(c.editor.resource_selection, Some(material));
    assert!(Arc::ptr_eq(&authoring, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), 0);
    // A directory match retains its source assets and their generated children.
    search(&mut c, "模型 集");
    c.press("全部类型", Rect::EVERYTHING);
    c.press("材质", Rect::EVERYTHING);
    position(&c.frame(vec![]), "材质  Amber", Rect::EVERYTHING);
    c.editor.refresh_resources()?;
    c.wait_resources()?;
    assert!(
        c.editor.resource_build.failure.is_none(),
        "{:?}",
        c.editor.error
    );
    assert_eq!(c.editor.resource_selection, None);
    position(&c.frame(vec![]), "材质  Amber", Rect::EVERYTHING);
    search(&mut c, "missing-resource");
    position(&c.frame(vec![]), "没有匹配的资源。", Rect::EVERYTHING);
    assert!(Arc::ptr_eq(&authoring, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn translated_root_fields_and_gizmo_use_source_overrides_without_reapplying_placement() -> Result<()>
{
    use comet_scene::{schema, schema::FieldValue};
    let mut c = Case::new(false)?;
    let prefab = c
        .editor
        .sources
        .of_kind(AssetKind::Prefab)
        .find(|s| s.source == std::path::Path::new(c.resource))
        .unwrap()
        .id;
    let offset = [8., 2., -3.];
    c.editor
        .place_resource(prefab, "placed", offset, &mut c.assets)?;
    let id = c.editor.selection.active().unwrap();
    let entity = c
        .editor
        .snapshot(&mut c.assets)?
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .clone();
    let placed = c.editor.document.shared_source();
    c.editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3(entity.transform.translation),
        &mut c.assets,
    )?;
    c.editor.commit()?;
    assert_eq!(c.editor.document.shared_source(), placed);
    c.editor.begin_translation(&mut c.assets)?;
    c.editor
        .preview_translation(comet_core::math::Vec3::new(2., 0., 0.), &mut c.assets)?;
    let translated = c.editor.snapshot(&mut c.assets)?;
    assert!(
        translated
            .entities
            .iter()
            .all(|e| e.transform.translation[0] >= 8.)
    );
    assert_eq!(
        c.editor.document.shared_source().instances[0].translation,
        offset
    );
    assert_eq!(
        c.editor.document.shared_source().instances[0]
            .overrides
            .len(),
        2
    );
    c.editor
        .preview_translation(comet_core::math::Vec3::ZERO, &mut c.assets)?;
    assert_eq!(c.editor.document.shared_source(), placed);
    c.editor.commit()?;
    assert_eq!(c.editor.document.history_len(), 1);
    let mut value = entity.transform.translation;
    value[1] += 3.;
    c.editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3(value),
        &mut c.assets,
    )?;
    c.editor.commit()?;
    assert_eq!(
        c.editor
            .snapshot(&mut c.assets)?
            .entities
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .transform
            .translation,
        value
    );
    c.editor
        .revert_prefab_fields(&[id], Some(&[schema::TRANSLATION]), &mut c.assets)?;
    assert_eq!(c.editor.document.shared_source(), placed);
    assert_eq!(
        c.editor
            .snapshot(&mut c.assets)?
            .entities
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .transform
            .translation,
        entity.transform.translation
    );
    for (path, bytes) in c.sources {
        assert_eq!(fs::read(path)?, bytes);
    }
    Ok(())
}
