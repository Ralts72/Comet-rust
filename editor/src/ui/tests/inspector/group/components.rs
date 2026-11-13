use super::*;
use crate::components::ComponentResources;
use comet_editor_core::ComponentKind;

fn click(f: &mut Group, label: &str) -> Result<()> {
    let output = f.frame(vec![]);
    let pos = texts(&output)
        .into_iter()
        .rev()
        .find(|(text, _)| text == label)
        .with_context(|| format!("component menu item {label}: {:?}", texts(&output)))?
        .1;
    f.frame(vec![Event::PointerMoved(pos), button(pos, true)]);
    f.frame(vec![button(pos, false)]);
    Ok(())
}

#[test]
fn group_component_menus_add_only_missing_values_and_remove_as_one_saved_undoable_change()
-> Result<()> {
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    let first = source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[0])
        .unwrap();
    first.physics.as_mut().unwrap().mass = 7.;
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .physics = None;
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.frame(vec![]);
    click(&mut f, "添加组件")?;
    click(&mut f, "刚体")?;

    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let added = f.editor.document.shared_source();
    assert!(added.base.entities.iter().all(|e| e.physics.is_some()));
    assert_eq!(
        added
            .base
            .entities
            .iter()
            .find(|e| e.id == f.ids[0])
            .unwrap(),
        original
            .base
            .entities
            .iter()
            .find(|e| e.id == f.ids[0])
            .unwrap()
    );
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), added);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), added);
    click(&mut f, "移除组件")?;
    click(&mut f, "刚体")?;
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let removed = f.editor.document.shared_source();
    assert!(removed.base.entities.iter().all(|e| e.physics.is_none()));
    assert_eq!(f.editor.document.history_len(), 2);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), added);
    assert!(!f.editor.document.is_dirty());
    Ok(())
}

#[test]
fn component_resource_drafts_preserve_existing_script_parameters_and_validate_before_publishing()
-> Result<()> {
    let mut f = Group::new()?;
    let (_, scripts) = f.resource_components()?;
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .script = None;
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    let invalid = ComponentResources {
        asset: Some(original.base.entities[0].model.as_ref().unwrap().mesh),
        material: None,
    };
    assert!(
        f.editor
            .add_components(&f.ids, ComponentKind::Script, invalid, &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    f.frame(vec![]);
    click(&mut f, "添加组件")?;
    click(&mut f, "脚本")?;
    click(&mut f, "请选择资源")?;
    let label = f.asset_label(scripts[1]);
    click(&mut f, &label)?;
    assert_eq!(f.editor.document.shared_source(), original);
    // Choosing a resource is a draft; closing the menu must not create history.
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.history_len(), 0);
    click(&mut f, "添加组件")?;
    click(&mut f, "脚本")?;
    click(&mut f, "请选择资源")?;
    click(&mut f, &label)?;
    click(&mut f, "添加到所选实体")?;
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let added = f.editor.document.shared_source();
    assert_eq!(
        added
            .base
            .entities
            .iter()
            .find(|e| e.id == f.ids[0])
            .unwrap(),
        original
            .base
            .entities
            .iter()
            .find(|e| e.id == f.ids[0])
            .unwrap()
    );
    let script = added
        .base
        .entities
        .iter()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .script
        .as_ref()
        .unwrap();
    assert_eq!(script.asset, scripts[1]);
    assert!(script.parameters.is_empty());
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn structural_component_rejections_keep_active_previews_and_refuse_prefab_play_and_modal_targets()
-> Result<()> {
    let mut f = Group::new()?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .add_components(
                &f.ids,
                ComponentKind::Physics,
                Default::default(),
                &mut f.assets
            )
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    f.editor.edit(
        f.ids[0],
        comet_scene::schema::ROUGHNESS,
        comet_scene::schema::FieldValue::Scalar(0.6),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    assert!(
        f.editor
            .add_components(
                &f.ids,
                ComponentKind::Physics,
                Default::default(),
                &mut f.assets
            )
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), preview);
    assert!(f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.cancel()?;
    assert!(
        f.editor
            .remove_components(
                &[f.ids[0], comet_core::EntityId::new()],
                ComponentKind::Model,
                &mut f.assets
            )
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    assert!(
        f.editor
            .remove_components(&f.ids, ComponentKind::Model, &mut f.assets)
            .is_err()
    );
    f.editor.documents = Default::default();
    f.editor.start(&mut f.assets)?;
    assert!(
        f.editor
            .remove_components(&f.ids, ComponentKind::Model, &mut f.assets)
            .is_err()
    );
    f.editor.stop();
    let prefab = f
        .assets
        .index()
        .products
        .iter()
        .find_map(|(&id, e)| (e.kind == comet_asset_data::product::AssetKind::Prefab).then_some(id))
        .context("fixture prefab")?;
    let instance = comet_core::InstanceId::new();
    let source_entity = comet_scene::PrefabResolver::prefab(&mut f.assets, prefab)?
        .base
        .entities[0]
        .id;
    let mut source = (*original).clone();
    source.instances.push(comet_scene::PrefabInstance {
        id: instance,
        prefab,
        parent: None,
        translation: [0.; 3],
        overrides: vec![],
    });
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .remove_components(
                &[f.ids[0], source_entity.in_instance(instance)],
                ComponentKind::Model,
                &mut f.assets
            )
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_component_menus_preserve_authoring_and_draw_selected_models() -> Result<()> {
    let mut f = Group::new()?;
    let output = f.frame(vec![]);
    let fields = ["添加组件", "移除组件"];
    for label in fields {
        assert!(texts(&output).iter().any(|(text, _)| text == label));
    }
    capture_common(
        f,
        "editor-components",
        serde_json::json!({"component_fields":fields,"source_unchanged":true}),
    )
}

#[test]
fn adding_animation_validates_expanded_skin_references_but_preserves_the_authored_instance_scope()
-> Result<()> {
    use comet_scene::{ContentSource, EntityRef, PrefabInstance, PrefabResolver};
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    let prefab = f.assets.prefab(rig.prefab)?;
    let owner = prefab
        .base
        .entities
        .iter()
        .find(|e| e.animator.is_some())
        .unwrap()
        .id;
    let mut mesh = prefab
        .base
        .entities
        .iter()
        .find(|e| e.skin.is_some())
        .unwrap()
        .clone();
    let instance = comet_core::InstanceId::new();
    mesh.id = comet_core::EntityId::new();
    mesh.parent = None;
    mesh.skin.as_mut().unwrap().animator = Some(EntityRef::in_instance(instance, owner));
    let id = mesh.id;
    f.editor.document = Document::open_source(ContentSource {
        base: SceneSnapshot {
            entities: vec![mesh],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            id: instance,
            prefab: rig.prefab,
            parent: None,
            translation: [0.; 3],
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    let original = f.editor.document.shared_source();
    f.editor.add_components(
        &[id],
        ComponentKind::Animation,
        ComponentResources {
            asset: Some(rig.skeleton),
            material: None,
        },
        &mut f.assets,
    )?;
    f.editor.commit()?;
    let added = f.editor.document.shared_source();
    assert_eq!(added.base.entities[0].skin, original.base.entities[0].skin);
    assert_eq!(added.instances, original.instances);
    assert_eq!(
        added.base.entities[0].animator.as_ref().unwrap().skeleton,
        rig.skeleton
    );
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}
