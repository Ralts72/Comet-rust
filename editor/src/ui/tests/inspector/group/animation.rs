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
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "播放片段（多值）")
    );
    capture_common(
        f,
        "editor-common-animation-clips",
        serde_json::json!({"asset_fields": ["动画控制器", "播放片段（多值）"], "source_unchanged": true}),
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
