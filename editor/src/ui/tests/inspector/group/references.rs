use super::*;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::InstanceId;
use comet_scene::{
    Animator, BodyMotion, Character, CharacterAnimation, ColliderShape, ContentSource, EntityRef,
    NavigationAgent, PhysicsBody, PrefabInstance,
    schema::{self, FieldValue},
};

#[test]
fn navigation_bindings_intersect_scopes_and_persist_authored_and_prefab_references_atomically()
-> Result<()> {
    let mut f = Group::new()?;
    f.physics(true)?;
    let mut source = f.editor.document.source();
    for entity in &mut source.base.entities {
        entity.character.as_mut().unwrap().player_controlled = false;
        entity.navigation_agent = Some(NavigationAgent::default());
    }
    let mut goal = EntityData::new("共同目标");
    goal.transform.translation = [4., 0., 0.];
    let goal_id = goal.id;
    let mut npc = source.base.entities.pop().unwrap();
    let source_npc = npc.id;
    npc.navigation_agent.as_mut().unwrap().target = None;
    let path = f.editor.project_root().join("assets/bindings.prefab");
    std::fs::write(
        &path,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![npc, goal],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let prefab = SourceMeta::load(&path)?.id;
    let outside = EntityData::new("作用域外目标");
    let outside_id = outside.id;
    source.base.entities.push(outside);
    let instance = InstanceId::new();
    source.instances.push(PrefabInstance {
        id: instance,
        prefab,
        parent: None,
        translation: [0.; 3],
        overrides: vec![],
    });
    f.ids = [source.base.entities[0].id, source_npc.in_instance(instance)];
    f.editor.document = Document::open_source(source)?;
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    let bytes = std::fs::read(&path)?;
    let metadata = std::fs::read(path.with_extension("prefab.meta"))?;
    f.open_picker("导航目标")?;
    let labels = texts(&f.frame(vec![]));
    assert!(
        !labels
            .iter()
            .any(|(text, pos)| pos.x > 970. && text == "作用域外目标")
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.select_reference("导航目标", "共同目标")?;
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let edited = f.editor.document.shared_source();
    assert_eq!(
        edited
            .base
            .entities
            .iter()
            .find(|e| e.id == f.ids[0])
            .unwrap()
            .navigation_agent
            .as_ref()
            .unwrap()
            .target,
        Some(EntityRef::in_instance(instance, goal_id))
    );
    assert_eq!(edited.instances[0].overrides.len(), 1);
    assert_eq!(
        edited.instances[0].overrides[0].field,
        schema::NAVIGATION_TARGET
    );
    assert_eq!(
        edited.instances[0].overrides[0].value,
        FieldValue::Entity(Some(EntityRef::local(goal_id)))
    );
    assert_eq!(f.editor.document.history_len(), 1);
    f.select_reference("导航目标", "共同目标")?;
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), edited);
    f.editor.edit(
        f.ids[0],
        schema::NAVIGATION_ARRIVAL_DISTANCE,
        FieldValue::Scalar(0.4),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    for target in [outside_id, f.ids[0], EntityId::new()] {
        assert!(
            f.editor
                .replace_entity_bindings(
                    &f.ids,
                    schema::NAVIGATION_TARGET,
                    Some(target),
                    &mut f.assets
                )
                .is_err()
        );
        assert_eq!(f.editor.document.shared_source(), preview);
        assert!(f.editor.document.is_editing());
    }
    f.editor.cancel()?;
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(std::fs::read(path.with_extension("prefab.meta"))?, metadata);
    Ok(())
}

#[test]
fn skin_binding_choices_validate_every_skin_and_keep_previews_saved_undo_and_play_isolated()
-> Result<()> {
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    let (owner, foreign) = f.skin_bindings(&rig)?;
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .skin
        .as_mut()
        .unwrap()
        .animator = None;
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.open_picker("动画实体")?;
    let labels = texts(&f.frame(vec![]));
    assert!(
        labels
            .iter()
            .any(|(text, pos)| pos.x > 970. && text == "兼容的动画实体")
    );
    assert!(
        !labels
            .iter()
            .any(|(text, pos)| pos.x > 970. && text == "不兼容的动画实体")
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    f.select_reference("动画实体", "兼容的动画实体")?;
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let edited = f.editor.document.shared_source();
    for &id in &f.ids {
        let before = original.base.entities.iter().find(|e| e.id == id).unwrap();
        let mut expected = before.clone();
        expected.skin.as_mut().unwrap().animator = Some(EntityRef::local(owner));
        assert_eq!(
            edited.base.entities.iter().find(|e| e.id == id).unwrap(),
            &expected
        );
    }
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), edited);
    f.editor.edit(
        f.ids[0],
        schema::ROUGHNESS,
        FieldValue::Scalar(0.3),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    for target in [foreign, f.ids[0]] {
        assert!(
            f.editor
                .replace_entity_bindings(&f.ids, schema::SKIN_ANIMATOR, Some(target), &mut f.assets)
                .is_err()
        );
        assert_eq!(f.editor.document.shared_source(), preview);
        assert!(f.editor.document.is_editing());
    }
    f.editor.cancel()?;
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    assert!(
        f.editor
            .replace_entity_bindings(&f.ids, schema::SKIN_ANIMATOR, None, &mut f.assets)
            .is_err()
    );
    f.select_reference("动画实体", "未绑定")
        .expect_err("Play selector is disabled");
    assert_eq!(f.editor.document.shared_source(), edited);
    f.editor.stop();
    f.editor.selection.replace(f.ids);
    f.select_reference("动画实体", "未绑定")?;
    assert_eq!(f.editor.document.history_len(), 2);
    f.editor.save(&mut f.assets)?;
    assert!(
        f.editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .filter(|e| f.ids.contains(&e.id))
            .all(|e| e.skin.as_ref().unwrap().animator.is_none())
    );
    Ok(())
}

#[test]
fn character_bindings_filter_parent_and_descendant_roles_and_reject_a_later_invalid_owner()
-> Result<()> {
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    let mut source = f.editor.document.source();
    let mut parents = Vec::new();
    let mut animators = Vec::new();
    for (index, &id) in f.ids.iter().enumerate() {
        let visual = source
            .base
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .unwrap();
        let mut parent = EntityData::new(format!("角色 {index}"));
        parent.physics = Some(PhysicsBody {
            motion: BodyMotion::Kinematic,
            shape: ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.3,
            },
            ..Default::default()
        });
        parent.character = Some(Character {
            player_controlled: false,
            ..Default::default()
        });
        visual.parent = Some(parent.id);
        visual.character_animation = Some(CharacterAnimation {
            character: Some(EntityRef::local(parent.id)),
            animator: None,
        });
        let mut animator = EntityData::new(format!("外观动画 {index}"));
        animator.parent = Some(visual.id);
        animator.animator = Some(Animator::new(rig.skeleton));
        parents.push(parent);
        animators.push(animator);
    }
    let parent = parents[0].id;
    let animator = animators[0].id;
    source.base.entities.extend(parents);
    source.base.entities.extend(animators);
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    for (field, target) in [
        (schema::CHARACTER_ANIMATION_CHARACTER, parent),
        (schema::CHARACTER_ANIMATION_ANIMATOR, animator),
    ] {
        assert!(
            f.editor
                .replace_entity_bindings(&f.ids, field, Some(target), &mut f.assets)
                .is_err()
        );
        assert_eq!(f.editor.document.shared_source(), original);
        assert_eq!(f.editor.document.history_len(), 0);
    }
    f.open_picker("角色实体")?;
    assert!(
        !texts(&f.frame(vec![]))
            .iter()
            .any(|(text, pos)| pos.x > 970. && text == "角色 0")
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    f.select_reference("角色实体", "未绑定")?;
    assert_eq!(f.editor.document.history_len(), 1);
    let cleared = f.editor.document.shared_source();
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), cleared);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), cleared);
    f.editor.replace_entity_bindings(
        &f.ids[..1],
        schema::CHARACTER_ANIMATION_ANIMATOR,
        Some(animator),
        &mut f.assets,
    )?;
    f.editor.commit()?;
    assert_eq!(f.editor.document.history_len(), 2);
    assert_eq!(
        f.editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|e| e.id == f.ids[0])
            .unwrap()
            .character_animation
            .as_ref()
            .unwrap()
            .animator,
        Some(EntityRef::local(animator))
    );
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_common_entity_bindings_preserve_authoring() -> Result<()> {
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    f.skin_bindings(&rig)?;
    let output = f.frame(vec![]);
    for label in ["蒙皮", "动画实体", "兼容的动画实体"] {
        assert!(texts(&output).iter().any(|(text, _)| text == label));
    }
    capture_common(
        f,
        "editor-common-entity-bindings",
        serde_json::json!({"entity_fields":["动画实体"], "source_unchanged":true}),
    )
}
