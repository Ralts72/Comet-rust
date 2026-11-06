use super::*;
use comet_scene::{
    BodyMotion, Character, ColliderShape, ContentSource, PhysicsBody, PrefabInstance,
    schema::{self, FieldKey, FieldValue},
};

impl Group {
    pub(super) fn physics(&mut self, character: bool) -> Result<()> {
        let mut source = self.editor.document.source();
        for (index, id) in self.ids.iter().enumerate() {
            let entity = source
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == *id)
                .unwrap();
            entity.parent = None;
            entity.physics = Some(PhysicsBody {
                mass: 1. + index as f32,
                friction: 0.2 + index as f32 * 0.3,
                ..Default::default()
            });
            if character {
                entity.transform.rotation = Quat::IDENTITY.to_array();
                let body = entity.physics.as_mut().unwrap();
                body.motion = BodyMotion::Kinematic;
                body.shape = ColliderShape::Capsule {
                    half_height: 0.5,
                    radius: 0.3,
                };
                entity.character = Some(Character {
                    player_controlled: index == 0,
                    move_speed: 3. + index as f32,
                    jump_speed: 6. + index as f32,
                    ..Default::default()
                });
            }
        }
        self.editor.document = Document::open_source(source)?;
        Ok(())
    }
    pub(super) fn scalar_values(&mut self, field: FieldKey) -> Result<[f32; 2]> {
        let snapshot = self.editor.snapshot(&mut self.assets)?;
        Ok(self.ids.map(|id| {
            let entity = snapshot
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .unwrap();
            let Some(FieldValue::Scalar(value)) = schema::descriptor(field).unwrap().read(entity)
            else {
                panic!("scalar fixture")
            };
            value
        }))
    }
    pub(super) fn type_scalar(&mut self, field: &str, value: &str) -> Result<()> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        self.type_at(scalar_control(&output, field)?, value);
        Ok(())
    }
    pub(super) fn confirm(&mut self) {
        self.frame(vec![key(egui::Key::Enter, true, Default::default())]);
        self.frame(vec![key(egui::Key::Enter, false, Default::default())]);
    }
    pub(super) fn drag_scalar(&mut self, field: &str, distance: f32) -> Result<Pos2> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let start = scalar_control(&output, field)?;
        let end = start + Vec2::new(distance, 0.);
        self.frame(vec![Event::PointerMoved(start), button(start, true)]);
        self.frame(vec![Event::PointerMoved(end)]);
        Ok(end)
    }
}
fn scalar_control(output: &egui::FullOutput, field: &str) -> Result<Pos2> {
    texts(output)
        .into_iter()
        .find(|(text, pos)| pos.x > 970. && text.starts_with(field))
        .map(|(_, pos)| pos)
        .context("common scalar control")
}

#[test]
fn confirming_the_primary_value_unifies_mixed_scalars_and_only_the_selected_transform_axis()
-> Result<()> {
    let mut f = Group::new()?;
    let before = f.transforms();
    f.type_scalar("粗糙度", "0.8")?;
    f.confirm();
    assert_eq!(f.scalar_values(schema::ROUGHNESS)?, [0.8; 2]);
    assert_eq!(f.transforms(), before);
    assert_eq!(f.editor.document.history_len(), 1);
    f.type_value("位置", "X", "1")?;
    f.confirm();
    let after = f.transforms();
    assert!(after.iter().all(|transform| transform.translation[0] == 1.));
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(old.translation[1..], new.translation[1..]);
        assert_eq!(old.rotation, new.rotation);
        assert_eq!(old.scale, new.scale);
    }
    assert_eq!(f.editor.document.history_len(), 2);
    Ok(())
}

#[test]
fn common_components_are_the_intersection_and_legal_small_values_are_not_rewritten_on_idle()
-> Result<()> {
    let mut f = Group::new()?;
    f.physics(false)?;
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .model = None;
    for entity in &mut source.base.entities {
        entity.physics.as_mut().unwrap().mass = 0.000001;
    }
    f.editor.document = Document::open_source(source)?;
    let before = f.editor.document.shared_source();
    f.frame(vec![]);
    let output = f.frame(vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text == "网格渲染" || text.starts_with("粗糙度"))
    );
    assert!(scalar_control(&output, "质量 (kg)").is_ok());
    assert_eq!(f.editor.document.shared_source(), before);
    assert!(!f.editor.document.is_editing() && !f.editor.document.is_dirty());
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .physics = None;
    f.editor.document = Document::open_source(source)?;
    let output = f.frame(vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text == "刚体" || text.starts_with("质量 (kg)"))
    );
    Ok(())
}

#[test]
fn body_scalar_drags_commit_once_and_escape_restores_the_entire_gesture() -> Result<()> {
    let mut f = Group::new()?;
    f.physics(false)?;
    let before = f.editor.document.shared_source();
    let end = f.drag_scalar("质量 (kg)", 20.)?;
    let masses = f.scalar_values(schema::MASS)?;
    assert_eq!(masses[0], masses[1]);
    assert!(masses[1] > 2.);
    f.frame(vec![button(end, false)]);
    assert_eq!(f.editor.document.history_len(), 1);
    let accepted = f.editor.document.shared_source();
    let end = f.drag_scalar("摩擦系数", 15.)?;
    assert_ne!(f.scalar_values(schema::FRICTION)?, [0.2, 0.5]);
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![Event::PointerMoved(end + Vec2::new(30., 0.))]);
    f.frame(vec![
        button(end, false),
        key(egui::Key::Escape, false, Default::default()),
    ]);
    assert_eq!(f.editor.document.shared_source(), accepted);
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn character_common_fields_preserve_unedited_configuration_and_upgrade_legacy_crouch_values()
-> Result<()> {
    let mut f = Group::new()?;
    f.physics(true)?;
    let mut source = f.editor.document.source();
    source.base.version = 2;
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    for (label, field, value) in [
        ("移动速度 (m/s)", schema::MOVE_SPEED, 5.),
        ("跳跃速度 (m/s)", schema::JUMP_SPEED, 8.),
        ("蹲伏速度 (m/s)", schema::CROUCH_SPEED, 3.),
        ("蹲伏高度比例", schema::CROUCH_HEIGHT_RATIO, 0.75),
    ] {
        f.type_scalar(label, &value.to_string())?;
        f.confirm();
        assert_eq!(f.scalar_values(field)?, [value; 2]);
    }
    let edited = f.editor.document.shared_source();
    assert_eq!(edited.base.version, 3);
    for (before, after) in original.base.entities.iter().zip(&edited.base.entities) {
        assert_eq!(before.transform, after.transform);
        assert_eq!(before.physics, after.physics);
        assert_eq!(before.model, after.model);
        let before = before.character.as_ref().unwrap();
        let after = after.character.as_ref().unwrap();
        assert_eq!(before.run_multiplier, after.run_multiplier);
        assert_eq!(before.gravity, after.gravity);
        assert_eq!(before.step_height, after.step_height);
    }
    assert_eq!(f.editor.document.history_len(), 4);
    for _ in 0..4 {
        f.editor.document.undo()?;
    }
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn authored_and_direct_prefab_roughness_use_one_transaction_and_keep_source_identity() -> Result<()>
{
    use comet_asset_pipeline::{SourceMeta, prepare};
    use comet_core::InstanceId;
    let mut f = Group::new()?;
    let original = f.editor.document.shared_source();
    let mut entity = EntityData::new("共同属性 Prefab");
    entity.model = original
        .base
        .entities
        .iter()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .model
        .clone();
    let prefab_node = entity.id;
    let path = f.editor.project_root().join("assets/common.prefab");
    std::fs::write(
        &path,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![entity],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let metadata = std::fs::read(SourceMeta::path(&path))?;
    let prefab_bytes = std::fs::read(&path)?;
    let instance = InstanceId::new();
    let mut source = f.editor.document.source();
    source.base.entities.retain(|entity| entity.id == f.ids[0]);
    source.instances.push(PrefabInstance {
        id: instance,
        prefab: SourceMeta::load(&path)?.id,
        parent: None,
        overrides: vec![],
    });
    f.editor.document = Document::open_source(source)?;
    f.ids[1] = prefab_node.in_instance(instance);
    f.editor.selection.replace(f.ids);
    let before = f.editor.document.shared_source();
    f.type_scalar("粗糙度", "0.45")?;
    f.confirm();
    assert_eq!(f.scalar_values(schema::ROUGHNESS)?, [0.45; 2]);
    let after = f.editor.document.shared_source();
    let overrides = &after.instances[0].overrides;
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].entity, prefab_node);
    assert_eq!(overrides[0].field, schema::ROUGHNESS);
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), after);
    assert_eq!(std::fs::read(&path)?, prefab_bytes);
    assert_eq!(std::fs::read(SourceMeta::path(&path))?, metadata);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn play_common_scalar_controls_stay_read_only() -> Result<()> {
    let mut f = Group::new()?;
    let before = f.editor.document.shared_source();
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    let end = f.drag_scalar("粗糙度", -30.)?;
    f.frame(vec![button(end, false)]);
    assert_eq!(f.scalar_values(schema::ROUGHNESS)?, [0.2, 0.8]);
    assert_eq!(f.editor.document.shared_source(), before);
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.stop();
    Ok(())
}
