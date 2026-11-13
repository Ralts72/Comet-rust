use super::*;

use comet_core::InstanceId;
use comet_scene::{ContentSource, PrefabInstance, PrefabResolver};

impl Group {
    fn select_empty_animation_asset(&mut self, field: &str, empty: &str) -> Result<()> {
        let anchor = self.open_asset_picker(field)?;
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .find(|(text, point)| text == empty && point.y > anchor.y + 3.)
            .context("no clip option")?
            .1;
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }
}

#[test]
fn common_animation_clip_choices_filter_by_every_skeleton_and_preserve_binding_configuration()
-> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_animations()?;
    f.animators(&clips, false)?;
    let original = f.editor.document.shared_source();
    f.open_asset_picker("播放片段")?;
    let output = f.frame(vec![]);
    let labels = texts(&output);
    assert!(
        labels
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.clips[0]))
    );
    assert!(
        labels
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.clips[1]))
    );
    assert!(
        !labels
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.foreign_clip))
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.select_asset("播放片段", clips.clips[0])?;
    assert_eq!(f.editor.document.history_len(), 1);
    let edited = f.editor.document.shared_source();
    for (before, after) in original.base.entities.iter().zip(&edited.base.entities) {
        let mut expected = before.animator.clone().unwrap();
        expected.clip = Some(clips.clips[0]);
        assert_eq!(after.animator, Some(expected));
    }
    f.select_asset("播放片段", clips.clips[0])?;
    assert_eq!(f.editor.document.history_len(), 1);
    f.select_empty_animation_asset("播放片段", "不播放片段")?;
    assert!(
        f.editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .all(|entity| entity.animator.as_ref().unwrap().clip.is_none())
    );
    assert_eq!(f.editor.document.history_len(), 2);
    f.editor.document.undo()?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn different_skeletons_allow_only_no_clip_and_reject_incompatible_or_wrong_kind_commands()
-> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_animations()?;
    f.animators(&clips, true)?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_animation_clips(&f.ids, Some(clips.clips[0]), &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .replace_animation_clips(&f.ids, Some(clips.skeleton), &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .replace_animation_controllers(&f.ids, Some(clips.controllers[0]), &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    f.open_asset_picker("播放片段")?;
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "没有匹配的资产")
    );
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.clips[0])
                || text == &f.asset_label(clips.foreign_clip))
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    f.select_empty_animation_asset("播放片段", "不播放片段")?;
    assert_eq!(f.editor.document.history_len(), 1);
    assert!(
        f.editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .all(|entity| entity.animator.as_ref().unwrap().clip.is_none())
    );
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn idle_common_asset_menus_use_metadata_without_loading_rig_products() -> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_animations()?;
    f.animators(&clips, false)?;
    let snapshot = f.editor.snapshot(&mut f.assets)?;
    let entities: Vec<_> = f
        .ids
        .iter()
        .map(|id| {
            snapshot
                .entities
                .iter()
                .find(|entity| entity.id == *id)
                .unwrap()
        })
        .collect();
    for id in f
        .assets
        .index()
        .products
        .keys()
        .copied()
        .collect::<Vec<_>>()
    {
        f.assets.evict(id);
    }
    let original = f.editor.document.shared_source();
    for _ in 0..3 {
        let mut output = f.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(1280., 800.),
                )),
                ..Default::default()
            },
            |ui| crate::ui::inspector::common::draw(&mut f.editor, ui, &mut f.assets, &entities),
        );
        output.textures_delta.clear();
    }
    assert_eq!(f.assets.resident_count(), 0);
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn direct_prefab_clip_choices_keep_source_identity_and_remain_read_only_in_play() -> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_animations()?;
    let prefab = f.assets.prefab(clips.prefab)?;
    let controller = prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .context("imported animator")?
        .id;
    let current = prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.id == controller)
        .unwrap()
        .animator
        .as_ref()
        .unwrap()
        .clip;
    let replacement = clips
        .clips
        .into_iter()
        .find(|id| Some(*id) != current)
        .unwrap();
    let instances = [InstanceId::new(), InstanceId::new()];
    f.editor.document = Document::open_source(ContentSource {
        instances: instances
            .map(|id| PrefabInstance {
                translation: [0.; 3],
                id,
                prefab: clips.prefab,
                parent: None,
                overrides: vec![],
            })
            .to_vec(),
        ..Default::default()
    })?;
    f.ids = instances.map(|instance| controller.in_instance(instance));
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    f.select_asset("播放片段", replacement)?;
    let edited = f.editor.document.shared_source();
    for instance in &edited.instances {
        assert_eq!(instance.overrides.len(), 1);
        assert_eq!(instance.overrides[0].entity, controller);
        assert_eq!(
            instance.overrides[0].field,
            comet_scene::schema::ANIMATOR_CLIP
        );
    }
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    assert_eq!(f.assets.prefab(clips.prefab)?, prefab);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.select_asset("动画控制器", clips.controllers[0])?;
    let controlled = f.editor.document.shared_source();
    for instance in &controlled.instances {
        assert_eq!(instance.overrides.len(), 2);
        assert!(
            instance
                .overrides
                .iter()
                .all(|value| value.entity == controller)
        );
        assert!(
            instance
                .overrides
                .iter()
                .any(|value| value.field == comet_scene::schema::ANIMATOR_CONTROLLER)
        );
        assert!(
            instance
                .overrides
                .iter()
                .any(|value| value.field == comet_scene::schema::ANIMATOR_CLIP)
        );
    }
    f.editor.save(&mut f.assets)?;
    assert_eq!(
        Document::load(&f.editor.source)?.shared_source(),
        controlled
    );
    assert_eq!(f.assets.prefab(clips.prefab)?, prefab);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    f.open_asset_picker("播放片段")?;
    assert!(
        f.editor
            .replace_animation_clips(&f.ids, None, &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .replace_animation_controllers(&f.ids, Some(clips.controllers[0]), &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.stop();
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_common_animation_clip_menu_keeps_bindings_and_authoring_unchanged() -> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_animations()?;
    let prefab = f.assets.prefab(clips.prefab)?;
    let controller = prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .context("imported animator")?
        .id;
    let instances = [InstanceId::new(), InstanceId::new()];
    f.editor.document = Document::open_source(ContentSource {
        instances: instances
            .iter()
            .enumerate()
            .map(|(index, &id)| PrefabInstance {
                translation: [0.; 3],
                id,
                prefab: clips.prefab,
                parent: None,
                overrides: if index == 0 {
                    vec![]
                } else {
                    vec![comet_scene::FieldOverride {
                        entity: controller,
                        field: comet_scene::schema::ANIMATOR_CLIP,
                        value: comet_scene::schema::FieldValue::OptionalAsset(None),
                    }]
                },
            })
            .collect(),
        ..Default::default()
    })?;
    f.ids = instances.map(|instance| controller.in_instance(instance));
    f.editor.selection.replace(f.ids);
    let output = f.frame(vec![]);
    let labels = texts(&output);
    for field in ["骨架", "动画控制器", "重建绑定表"] {
        assert!(labels.iter().any(|(text, _)| text == field));
    }
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "播放片段（多值）")
    );
    capture_common(
        f,
        "editor-common-animation-clips",
        serde_json::json!({"asset_fields": ["骨架", "动画控制器", "播放片段（多值）"], "binding_action": "重建绑定表", "source_unchanged": true}),
    )
}

#[test]
fn common_controllers_clear_clips_atomically_and_preserve_bindings_history_and_failed_previews()
-> Result<()> {
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    f.animators(&rig, false)?;
    let original = f.editor.document.shared_source();
    f.open_asset_picker("动画控制器")?;
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == &f.asset_label(rig.controllers[0]))
    );
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text == &f.asset_label(rig.foreign_controller))
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.select_asset("动画控制器", rig.controllers[0])?;
    let changed = f.editor.document.shared_source();
    for (before, after) in original.base.entities.iter().zip(&changed.base.entities) {
        let mut expected = before.animator.clone().unwrap();
        expected.controller = Some(rig.controllers[0]);
        expected.clip = None;
        assert_eq!(after.animator, Some(expected));
    }
    assert_eq!(f.editor.document.history_len(), 1);
    f.select_asset("动画控制器", rig.controllers[0])?;
    assert_eq!(f.editor.document.history_len(), 1);
    f.select_empty_animation_asset("动画控制器", "单片段播放")?;
    assert!(
        f.editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .all(|e| {
                let a = e.animator.as_ref().unwrap();
                a.controller.is_none() && a.clip.is_none()
            })
    );
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), changed);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), changed);
    f.editor.edit(
        f.ids[0],
        comet_scene::schema::ANIMATOR_SPEED,
        comet_scene::schema::FieldValue::Scalar(2.),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    for controller in [rig.foreign_controller, rig.skeleton] {
        assert!(
            f.editor
                .replace_animation_controllers(&f.ids, Some(controller), &mut f.assets)
                .is_err()
        );
        assert_eq!(f.editor.document.shared_source(), preview);
        assert!(f.editor.document.is_editing());
        assert_eq!(f.editor.document.history_len(), 1);
    }
    f.editor.cancel()?;
    Ok(())
}

#[test]
fn common_skin_replacement_preserves_bindings_and_rejects_a_later_incompatible_mesh_atomically()
-> Result<()> {
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    let static_mesh = f.editor.document.shared_source().base.entities[0]
        .model
        .as_ref()
        .unwrap()
        .mesh;
    let mut source = f.assets.prefab(rig.prefab)?;
    let mesh = source
        .base
        .entities
        .iter()
        .find(|e| e.skin.is_some())
        .unwrap()
        .clone();
    let mut second = mesh.clone();
    second.id = comet_core::EntityId::new();
    second.skin.as_mut().unwrap().skin = rig.skins[1];
    f.ids = [mesh.id, second.id];
    source.base.entities.push(second);
    f.editor.document = Document::open_source(source)?;
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    f.select_asset("蒙皮资源", rig.skins[0])?;
    let changed = f.editor.document.shared_source();
    for &id in &f.ids {
        let before = original.base.entities.iter().find(|e| e.id == id).unwrap();
        let after = changed.base.entities.iter().find(|e| e.id == id).unwrap();
        let mut expected = before.clone();
        expected.skin.as_mut().unwrap().skin = rig.skins[0];
        assert_eq!(after, &expected);
    }
    assert_eq!(f.editor.document.history_len(), 1);
    assert!(
        f.editor
            .replace_skin_assets(&f.ids, rig.foreign_skin, &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), changed);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    let mut invalid = (*original).clone();
    invalid
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .mesh = static_mesh;
    f.editor.document = Document::open_source(invalid)?;
    f.editor.edit(
        f.ids[0],
        comet_scene::schema::ROUGHNESS,
        comet_scene::schema::FieldValue::Scalar(0.3),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_skin_assets(&f.ids, rig.skins[0], &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), preview);
    assert!(f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.cancel()?;
    Ok(())
}

#[test]
fn skeleton_choices_replace_the_group_atomically_and_clear_only_incompatible_playback() -> Result<()>
{
    use comet_scene::schema;
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    f.animators(&rig, false)?;
    let mut source = f.editor.document.source();
    let first = source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[0])
        .unwrap()
        .animator
        .as_mut()
        .unwrap();
    first.clip = None;
    first.controller = Some(rig.controllers[0]);
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == f.ids[1])
        .unwrap()
        .animator
        .as_mut()
        .unwrap()
        .clip = Some(rig.clips[1]);
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.select_asset("骨架", rig.foreign_skeleton)?;
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let changed = f.editor.document.shared_source();
    for &id in &f.ids {
        let before = original.base.entities.iter().find(|e| e.id == id).unwrap();
        let after = changed.base.entities.iter().find(|e| e.id == id).unwrap();
        let product = f.assets.load(rig.foreign_skeleton)?;
        let comet_asset_data::product::Product::Skeleton(skeleton) = &product.payload else {
            unreachable!()
        };
        let mut expected = before.clone();
        let a = expected.animator.as_mut().unwrap();
        a.skeleton = rig.foreign_skeleton;
        a.clip = None;
        a.controller = None;
        a.bindings = skeleton.nodes.iter().map(|node| (node.id, None)).collect();
        assert_eq!(after, &expected);
    }
    assert_eq!(f.editor.document.history_len(), 1);
    f.select_asset("骨架", rig.foreign_skeleton)?;
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), changed);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), changed);
    f.editor.edit(
        f.ids[0],
        schema::ANIMATOR_SPEED,
        schema::FieldValue::Scalar(3.),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    for (ids, asset) in [(f.ids, rig.clips[0]), ([f.ids[0], f.ids[0]], rig.skeleton)] {
        assert!(
            f.editor
                .replace_animation_skeletons(&ids, asset, &mut f.assets)
                .is_err()
        );
        assert_eq!(f.editor.document.shared_source(), preview);
        assert!(f.editor.document.is_editing());
    }
    f.editor.cancel()?;
    Ok(())
}

#[test]
fn skeleton_changes_validate_unselected_skin_consumers_and_keep_valid_previews_on_rejection()
-> Result<()> {
    use comet_scene::schema;
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
    let instances = [InstanceId::new(), InstanceId::new()];
    f.editor.document = Document::open_source(ContentSource {
        instances: instances
            .map(|id| PrefabInstance {
                id,
                prefab: rig.prefab,
                parent: None,
                translation: [0.; 3],
                overrides: vec![],
            })
            .to_vec(),
        ..Default::default()
    })?;
    f.ids = instances.map(|instance| owner.in_instance(instance));
    let original = f.editor.document.shared_source();
    f.editor.edit(
        f.ids[0],
        schema::ANIMATOR_SPEED,
        schema::FieldValue::Scalar(2.),
        &mut f.assets,
    )?;
    let preview = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_animation_skeletons(&f.ids, rig.foreign_skeleton, &mut f.assets)
            .is_err()
    );
    let mut after = f
        .editor
        .snapshot(&mut f.assets)?
        .entities
        .iter()
        .find(|e| e.id == f.ids[0])
        .unwrap()
        .animator
        .clone()
        .unwrap();
    after.skeleton = rig.foreign_skeleton;
    after.clip = None;
    after.controller = None;
    after.bindings.clear();
    assert!(
        f.editor
            .edit_animation(f.ids[0], Some(after), None, &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), preview);
    assert!(f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.cancel()?;
    assert_eq!(f.editor.document.shared_source(), original);
    let mut source = prefab;
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == owner)
        .unwrap()
        .animator = None;
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .add_components(
                &[owner],
                comet_editor_core::ComponentKind::Animation,
                crate::components::ComponentResources {
                    asset: Some(rig.foreign_skeleton),
                    material: None
                },
                &mut f.assets
            )
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    Ok(())
}

#[test]
fn mixed_skeleton_binding_rebuilds_preserve_lexical_references_prefab_overrides_and_saved_undo()
-> Result<()> {
    use comet_scene::{
        Animator, EntityRef, FieldOverride,
        schema::{self, FieldValue},
    };
    let mut f = Group::new()?;
    let rig = f.prepare_animations()?;
    let prefab = f.assets.prefab(rig.prefab)?;
    let owner = prefab
        .base
        .entities
        .iter()
        .find(|e| e.animator.is_some())
        .unwrap();
    let instance = InstanceId::new();
    let mut inherited = owner.animator.as_ref().unwrap().bindings.clone();
    let removed = *inherited.keys().next().unwrap();
    inherited.remove(&removed);
    let obsolete = EntityId::new();
    inherited.insert(obsolete, None);
    let mut authored = EntityData::new("作者动画");
    let product = f.assets.load(rig.foreign_skeleton)?;
    let comet_asset_data::product::Product::Skeleton(skeleton) = &product.payload else {
        unreachable!()
    };
    let source_reference = EntityRef::in_instance(instance, owner.id);
    let mut animation = Animator::new(rig.foreign_skeleton);
    animation.bindings = skeleton.nodes.iter().map(|node| (node.id, None)).collect();
    let retained = skeleton
        .nodes
        .iter()
        .find(|node| node.parent.is_none())
        .unwrap()
        .id;
    let authored_missing = *animation
        .bindings
        .keys()
        .find(|id| **id != retained)
        .unwrap();
    animation
        .bindings
        .insert(retained, Some(source_reference.clone()));
    animation.bindings.remove(&authored_missing);
    animation.bindings.insert(obsolete, None);
    authored.animator = Some(animation);
    f.ids = [authored.id, owner.id.in_instance(instance)];
    f.editor.document = Document::open_source(ContentSource {
        base: SceneSnapshot {
            entities: vec![authored],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            id: instance,
            prefab: rig.prefab,
            parent: Some(f.ids[0]),
            translation: [0.; 3],
            overrides: vec![FieldOverride {
                entity: owner.id,
                field: schema::ANIMATOR_BINDINGS,
                value: FieldValue::AnimatorBindings(inherited.clone()),
            }],
        }],
        ..Default::default()
    })?;
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    f.click_text("重建绑定表")?;
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let fixed = f.editor.document.shared_source();
    let bindings = &fixed.base.entities[0].animator.as_ref().unwrap().bindings;
    assert!(!bindings.contains_key(&obsolete));
    assert_eq!(bindings[&authored_missing], None);
    assert_eq!(bindings[&retained].as_ref(), Some(&source_reference));
    assert!(
        bindings
            .iter()
            .filter(|(id, _)| **id != retained)
            .all(|(_, value)| value.is_none())
    );
    let FieldValue::AnimatorBindings(values) = &fixed.instances[0].overrides[0].value else {
        unreachable!()
    };
    assert!(!values.contains_key(&obsolete));
    assert_eq!(values[&removed], None);
    for (&key, value) in &inherited {
        if key != obsolete {
            assert_eq!(values[&key], *value);
        }
    }
    assert_eq!(f.editor.document.history_len(), 1);
    f.click_text("重建绑定表")?;
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), fixed);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), fixed);
    assert!(!f.editor.document.is_dirty());
    Ok(())
}
