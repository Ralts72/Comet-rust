use super::*;
use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::InstanceId;
use comet_scene::schema::{FieldKey, FieldValue};
use comet_scene::{
    Animator, ContentSource, EntityRef, NavigationAgent, PrefabInstance, Script, schema,
};

impl Group {
    fn flag_values(&mut self, field: FieldKey) -> Result<[bool; 2]> {
        let snapshot = self.editor.snapshot(&mut self.assets)?;
        Ok(self.ids.map(|id| {
            let entity = snapshot
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .unwrap();
            let Some(FieldValue::Bool(value)) = schema::descriptor(field).unwrap().read(entity)
            else {
                panic!("flag fixture")
            };
            value
        }))
    }
    fn click_flag(&mut self, label: &str) -> Result<()> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .find(|(text, pos)| pos.x > 970. && text.starts_with(label))
            .context("common flag control")?
            .1;
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }
    fn focus_control(&mut self, label: &str) -> Result<()> {
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .find(|(text, pos)| pos.x > 970. && text.starts_with(label))
            .context("common flag control")?
            .1;
        let mut visited = std::collections::HashSet::new();
        loop {
            self.frame(vec![
                key(egui::Key::Tab, true, Default::default()),
                key(egui::Key::Tab, false, Default::default()),
            ]);
            let focused = self.ctx.memory(|memory| memory.focused());
            if focused
                .and_then(|id| self.ctx.read_response(id))
                .is_some_and(|response| {
                    response.rect.left() > 970. && response.rect.y_range().contains(position.y)
                })
            {
                return Ok(());
            }
            if focused.is_none_or(|id| !visited.insert(id)) {
                anyhow::bail!("common control did not receive keyboard focus");
            }
        }
    }
}

#[test]
fn mixed_audio_flags_toggle_from_the_primary_commit_individually_and_preserve_other_fields()
-> Result<()> {
    let mut f = Group::new()?;
    let path = f.audio()?;
    let metadata = std::fs::read(SourceMeta::path(&path))?;
    let original = f.editor.document.shared_source();
    for (label, field, expected) in [
        ("循环", schema::AUDIO_LOOPING, true),
        ("启动时播放", schema::AUDIO_AUTOPLAY, false),
        ("空间音效", schema::AUDIO_SPATIAL, true),
    ] {
        let output = f.frame(vec![]);
        assert!(
            texts(&output)
                .iter()
                .any(|(text, _)| text == &format!("{label}（多值）"))
        );
        f.click_flag(label)?;
        assert_eq!(f.flag_values(field)?, [expected; 2]);
        assert!(!f.editor.document.is_editing());
    }
    assert_eq!(f.editor.document.history_len(), 3);
    let edited = f.editor.document.shared_source();
    for (before, after) in original.base.entities.iter().zip(&edited.base.entities) {
        assert_eq!(before.transform, after.transform);
        assert_eq!(before.model, after.model);
        let before = before.audio_source.as_ref().unwrap();
        let after = after.audio_source.as_ref().unwrap();
        assert_eq!(
            (
                before.clip,
                before.gain,
                before.min_distance,
                before.max_distance
            ),
            (
                after.clip,
                after.gain,
                after.min_distance,
                after.max_distance
            )
        );
    }
    // Keyboard activation keeps the same control identity after the mixed label disappears.
    f.focus_control("空间音效")?;
    f.frame(vec![key(egui::Key::Space, true, Default::default())]);
    f.frame(vec![key(egui::Key::Space, false, Default::default())]);
    assert_eq!(f.flag_values(schema::AUDIO_SPATIAL)?, [false; 2]);
    assert_eq!(f.editor.document.history_len(), 4);
    for _ in 0..4 {
        f.editor.document.undo()?;
    }
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(std::fs::read(SourceMeta::path(&path))?, metadata);
    Ok(())
}

#[test]
fn common_audio_distances_validate_every_member_keep_accepted_previews_and_cancel_atomically()
-> Result<()> {
    let mut f = Group::new()?;
    f.audio()?;
    let mut source = f.editor.document.source();
    for (index, id) in f.ids.iter().enumerate() {
        let audio = source
            .base
            .entities
            .iter_mut()
            .find(|entity| entity.id == *id)
            .unwrap()
            .audio_source
            .as_mut()
            .unwrap();
        audio.min_distance = 1. + index as f32;
        audio.max_distance = if index == 0 { 10. } else { 30. };
    }
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.type_scalar("开始衰减距离（米）", "无效数值")?;
    f.confirm();
    assert_eq!(f.editor.document.shared_source(), original);
    f.type_scalar("开始衰减距离（米）", "15")?;
    f.confirm();
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 0);
    f.type_scalar("开始衰减距离（米）", "2")?;
    f.confirm();
    assert_eq!(f.scalar_values(schema::AUDIO_MIN_DISTANCE)?, [2.; 2]);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    let end = f.drag_scalar("开始衰减距离（米）", 20.)?;
    let accepted = f.editor.document.shared_source();
    let values = f.scalar_values(schema::AUDIO_MIN_DISTANCE)?;
    assert_eq!(values[0], values[1]);
    assert!(values[0] > 2. && values[0] < 10.);
    f.frame(vec![Event::PointerMoved(end + Vec2::new(200., 0.))]);
    assert_eq!(f.editor.document.shared_source(), accepted);
    assert!(f.editor.document.is_editing());
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![
        button(end, false),
        key(egui::Key::Escape, false, Default::default()),
    ]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.type_scalar("音量", "0.75")?;
    f.confirm();
    assert_eq!(f.scalar_values(schema::AUDIO_GAIN)?, [0.75; 2]);
    f.type_scalar("最大距离（米）", "40")?;
    f.confirm();
    assert_eq!(f.scalar_values(schema::AUDIO_MAX_DISTANCE)?, [40.; 2]);
    assert_eq!(f.scalar_values(schema::AUDIO_MIN_DISTANCE)?, [1., 2.]);
    assert_eq!(f.editor.document.history_len(), 2);
    Ok(())
}

#[test]
fn keyboard_focus_preserves_full_precision_and_small_imported_values() -> Result<()> {
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    for (index, id) in f.ids.iter().enumerate() {
        source
            .base
            .entities
            .iter_mut()
            .find(|entity| entity.id == *id)
            .unwrap()
            .physics
            .as_mut()
            .unwrap()
            .mass = 0.000001 * (index + 1) as f32;
    }
    f.editor.document = Document::open_source(source)?;
    let before = f.editor.document.shared_source();
    // Tab passes all TRS axes, roughness and mass without entering a value.
    f.focus_control("摩擦系数")?;
    f.frame(vec![
        key(egui::Key::Tab, true, Default::default()),
        key(egui::Key::Tab, false, Default::default()),
    ]);
    assert_eq!(f.editor.document.shared_source(), before);
    assert_eq!(f.editor.document.history_len(), 0);
    assert!(!f.editor.document.is_editing() && !f.editor.document.is_dirty());
    f.type_scalar("质量 (kg)", "0.000002")?;
    f.confirm();
    assert_eq!(f.scalar_values(schema::MASS)?, [0.000002; 2]);
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn two_dimensional_audio_distances_stay_disabled_and_missing_components_hide_common_controls()
-> Result<()> {
    let mut f = Group::new()?;
    f.audio()?;
    let mut source = f.editor.document.source();
    for entity in &mut source.base.entities {
        entity.audio_source.as_mut().unwrap().spatial = false;
    }
    f.editor.document = Document::open_source(source)?;
    let before = f.editor.document.shared_source();
    let end = f.drag_scalar("开始衰减距离（米）", 30.)?;
    f.frame(vec![button(end, false)]);
    assert_eq!(f.editor.document.shared_source(), before);
    assert!(!f.editor.document.is_editing());
    f.click_flag("空间音效")?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), before);
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.ids[0])
        .unwrap()
        .audio_source = None;
    f.editor.document = Document::open_source(source)?;
    f.frame(vec![]);
    let output = f.frame(vec![]);
    assert!(!texts(&output).iter().any(|(text, pos)| pos.x > 970.
        && (text == "音频源" || text.starts_with("空间音效") || text.starts_with("音量"))));
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn common_script_navigation_and_animation_fields_preserve_bindings_and_commit_separately()
-> Result<()> {
    let mut f = Group::new()?;
    let script = f.editor.project_root().join("assets/common.rhai");
    std::fs::write(&script, "fn on_start(ctx) {}")?;
    let mut source = f.editor.document.source();
    // Metadata is created only in this temporary project's assets.
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    for (index, id) in f.ids.iter().enumerate() {
        let mut script = Script::new(SourceMeta::load(&script)?.id);
        script.enabled = index == 0;
        source
            .base
            .entities
            .iter_mut()
            .find(|entity| entity.id == *id)
            .unwrap()
            .script = Some(script);
    }
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.click_flag("启用脚本")?;
    assert_eq!(f.flag_values(schema::SCRIPT_ENABLED)?, [true; 2]);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);

    f.physics(true)?;
    let mut source = f.editor.document.source();
    for (index, id) in f.ids.iter().enumerate() {
        let entity = source
            .base
            .entities
            .iter_mut()
            .find(|entity| entity.id == *id)
            .unwrap();
        entity.model = None;
        entity.script = None;
        entity.character.as_mut().unwrap().player_controlled = false;
        entity.navigation_agent = Some(NavigationAgent {
            enabled: index == 0,
            target: Some(EntityRef::local(f.ids[1 - index])),
            arrival_distance: 0.25 + index as f32 * 0.5,
        });
    }
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.click_flag("启用导航代理")?;
    f.type_scalar("到达距离", "1.25")?;
    f.confirm();
    assert_eq!(f.flag_values(schema::NAVIGATION_ENABLED)?, [true; 2]);
    assert_eq!(
        f.scalar_values(schema::NAVIGATION_ARRIVAL_DISTANCE)?,
        [1.25; 2]
    );
    let edited = f.editor.document.shared_source();
    for (before, after) in original.base.entities.iter().zip(&edited.base.entities) {
        assert_eq!(before.character, after.character);
        assert_eq!(before.physics, after.physics);
        assert_eq!(
            before.navigation_agent.as_ref().unwrap().target,
            after.navigation_agent.as_ref().unwrap().target
        );
    }
    assert_eq!(f.editor.document.history_len(), 2);

    crate::test_support::prepare_skin_asset(&f.editor)?;
    f.assets.refresh()?;
    let skeleton = *f
        .assets
        .index()
        .products
        .iter()
        .find(|(_, product)| product.kind == AssetKind::Skeleton)
        .context("fixture skeleton")?
        .0;
    let clip = *f
        .assets
        .index()
        .products
        .iter()
        .find(|(_, product)| product.kind == AssetKind::AnimationClip)
        .context("fixture clip")?
        .0;
    let mut source = f.editor.document.source();
    for (index, id) in f.ids.iter().enumerate() {
        let entity = source
            .base
            .entities
            .iter_mut()
            .find(|entity| entity.id == *id)
            .unwrap();
        entity.physics = None;
        entity.character = None;
        entity.navigation_agent = None;
        entity.animator = Some(Animator {
            clip: Some(clip),
            looping: index == 0,
            speed: 1. + index as f32,
            ..Animator::new(skeleton)
        });
    }
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.click_flag("循环")?;
    f.type_scalar("播放速度", "0")?;
    f.confirm();
    assert_eq!(f.flag_values(schema::ANIMATOR_LOOPING)?, [true; 2]);
    assert_eq!(f.scalar_values(schema::ANIMATOR_SPEED)?, [0.; 2]);
    let edited = f.editor.document.shared_source();
    for (before, after) in original.base.entities.iter().zip(&edited.base.entities) {
        let before = before.animator.as_ref().unwrap();
        let after = after.animator.as_ref().unwrap();
        assert_eq!(
            (before.skeleton, before.clip, &before.bindings),
            (after.skeleton, after.clip, &after.bindings)
        );
    }
    assert_eq!(f.editor.document.history_len(), 2);
    f.editor.document.undo()?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn common_prefab_flags_keep_lexical_identity_and_undo_spatial_changes() -> Result<()> {
    let mut f = Group::new()?;
    f.audio()?;
    let mut source = f.editor.document.source();
    for entity in &mut source.base.entities {
        entity.audio_source.as_mut().unwrap().spatial = false;
    }
    let mut node = source
        .base
        .entities
        .iter()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .clone();
    node.parent = None;
    let source_node = node.id;
    let prefab = f.editor.project_root().join("assets/common-flags.prefab");
    std::fs::write(
        &prefab,
        ContentSource {
            base: SceneSnapshot {
                version: comet_scene::scene::SCENE_VERSION,
                entities: vec![node],
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let bytes = std::fs::read(&prefab)?;
    let metadata = std::fs::read(SourceMeta::path(&prefab))?;
    let instance = InstanceId::new();
    source.base.entities.retain(|entity| entity.id == f.ids[0]);
    source.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: instance,
        prefab: SourceMeta::load(&prefab)?.id,
        parent: None,
        overrides: vec![],
    });
    f.editor.document = Document::open_source(source)?;
    f.ids[1] = source_node.in_instance(instance);
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    f.click_flag("空间音效")?;
    assert_eq!(f.flag_values(schema::AUDIO_SPATIAL)?, [true; 2]);
    let edited = f.editor.document.shared_source();
    assert_eq!(edited.instances[0].overrides.len(), 1);
    assert_eq!(edited.instances[0].overrides[0].entity, source_node);
    assert_eq!(
        edited.instances[0].overrides[0].field,
        schema::AUDIO_SPATIAL
    );
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    assert_eq!(std::fs::read(&prefab)?, bytes);
    assert_eq!(std::fs::read(SourceMeta::path(&prefab))?, metadata);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn common_configuration_flags_and_scalars_are_read_only_during_play() -> Result<()> {
    let mut f = Group::new()?;
    f.audio()?;
    let original = f.editor.document.shared_source();
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    for label in ["循环", "启动时播放", "空间音效"] {
        f.click_flag(label)?;
    }
    let end = f.drag_scalar("音量", -20.)?;
    f.frame(vec![button(end, false)]);
    assert_eq!(f.flag_values(schema::AUDIO_LOOPING)?, [true, false]);
    assert_eq!(f.flag_values(schema::AUDIO_AUTOPLAY)?, [false, true]);
    assert_eq!(f.flag_values(schema::AUDIO_SPATIAL)?, [true, false]);
    assert_eq!(f.scalar_values(schema::AUDIO_GAIN)?, [0.25, 0.75]);
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.stop();
    Ok(())
}
