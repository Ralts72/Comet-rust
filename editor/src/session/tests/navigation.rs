use super::*;
use crate::navigation::NavigationResult;
use comet_asset_data::{NavigationData, product::AssetKind};
use comet_core::InstanceId;
use comet_scene::{BodyMotion, ColliderShape, NavigationAgent, NavigationRegion, PrefabInstance};
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

struct Case {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    prefab: PathBuf,
    prefab_source: ContentSource,
    region: EntityId,
    source_floor: EntityId,
    source_npc: EntityId,
    floor: EntityId,
}
impl Case {
    fn new() -> Result<Self> {
        let (temp, mut editor, mut assets) = fixture()?;
        let mut floor = EntityData::new("导航地面");
        floor.transform.translation = [0., -0.25, 0.];
        floor.physics = Some(PhysicsBody {
            shape: ColliderShape::Box {
                half_extents: [3.5, 0.25, 3.5],
            },
            ..Default::default()
        });
        let mut goal = EntityData::new("导航目标");
        goal.transform.translation = [2., 0., 0.];
        let mut npc = EntityData::new("导航角色");
        npc.transform.translation = [-2., 1., 0.];
        npc.physics = Some(PhysicsBody {
            motion: BodyMotion::Kinematic,
            shape: ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.3,
            },
            ..Default::default()
        });
        npc.character = Some(Character {
            player_controlled: false,
            ..Default::default()
        });
        npc.navigation_agent = Some(NavigationAgent {
            target: Some(comet_scene::EntityRef::local(goal.id)),
            ..Default::default()
        });
        let (source_floor, source_npc) = (floor.id, npc.id);
        let prefab_source = ContentSource {
            base: SceneSnapshot {
                entities: vec![floor, npc, goal],
                ..Default::default()
            },
            ..Default::default()
        };
        let prefab = editor.paths.assets().join("navigation-host.prefab");
        fs::write(&prefab, prefab_source.to_json()?)?;
        comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
        assets.refresh()?;
        let asset = comet_asset_pipeline::SourceMeta::load(&prefab)?.id;
        let instance = InstanceId::new();
        let mut region = EntityData::new("导航区域");
        region.navigation_region = Some(NavigationRegion {
            half_extents: [4., 3., 4.],
            cell_size: 0.25,
            ..Default::default()
        });
        let id = region.id;
        editor.document = Document::open_source(ContentSource {
            base: SceneSnapshot {
                entities: vec![region],
                ..Default::default()
            },
            instances: vec![PrefabInstance {
                id: instance,
                prefab: asset,
                parent: None,
                overrides: vec![],
            }],
            ..Default::default()
        })?;
        Ok(Self {
            _temp: temp,
            editor,
            assets,
            prefab,
            prefab_source,
            region: id,
            source_floor,
            source_npc,
            floor: source_floor.in_instance(instance),
        })
    }
    fn wait(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.editor.navigation.is_running() {
            self.editor.poll_navigation(&mut self.assets);
            ensure!(Instant::now() < deadline, "navigation host timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
    fn bake(&mut self) -> Result<()> {
        self.editor
            .begin_navigation_bake(Path::new("navigation-host.nav"), &mut self.assets)?;
        self.wait()?;
        ensure!(
            self.editor.navigation.failure.is_none(),
            "{:?}",
            self.editor.navigation.failure
        );
        Ok(())
    }
}

#[test]
fn published_navigation_rejects_candidate_prefab_geometry_or_motor_profile_without_partial_acceptance()
-> Result<()> {
    for change_profile in [false, true] {
        let mut c = Case::new()?;
        let mut region = c.editor.document.shared_source().base.entities[0]
            .navigation_region
            .clone()
            .unwrap();
        for size in [0.3, 0.5] {
            region.cell_size = size;
            c.editor
                .edit_navigation(c.region, Some(region.clone()), None, &mut c.assets)?;
            c.editor.commit()?;
        }
        c.editor.document.undo()?;
        let current = c.editor.document.shared_source();
        let snapshot = c.editor.shared_snapshot(&mut c.assets)?;
        let state = c.editor.document.state_id();
        let revision = c.editor.document.revision();
        let index = c.assets.shared_index();
        let source_bytes = fs::read(&c.editor.source)?;
        c.editor
            .begin_navigation_bake(Path::new("navigation-host.nav"), &mut c.assets)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let baked = loop {
            if let Some(result) = c.editor.navigation.poll() {
                let NavigationResult::Baked(baked) = result.map_err(anyhow::Error::msg)? else {
                    anyhow::bail!("expected pure candidate");
                };
                break baked;
            }
            ensure!(Instant::now() < deadline, "pure bake timed out");
            std::thread::sleep(Duration::from_millis(2));
        };
        let mut changed = c.prefab_source.clone();
        if change_profile {
            changed
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == c.source_npc)
                .unwrap()
                .physics
                .as_mut()
                .unwrap()
                .shape = ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.5,
            };
        } else {
            changed
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == c.source_floor)
                .unwrap()
                .transform
                .translation[0] = 0.5;
        }
        fs::write(&c.prefab, changed.to_json()?)?;
        c.editor.navigation.prepare(
            &c.editor.paths,
            &c.editor.products,
            baked,
            c.editor.document.retained_sources().collect(),
        )?;
        c.wait()?;
        let failure = c
            .editor
            .navigation
            .failure
            .as_ref()
            .context("incompatible prepared products should be rejected")?;
        assert!(
            failure.contains("完整保留") && failure.contains("未接纳"),
            "{failure}"
        );
        let output = c.editor.paths.assets().join("navigation-host.nav");
        let data: NavigationData = serde_json::from_slice(&fs::read(&output)?)?;
        data.validate_for(&snapshot)?;
        assert!(comet_asset_pipeline::SourceMeta::load(&output).is_ok());
        assert!(Arc::ptr_eq(&current, &c.editor.document.shared_source()));
        assert!(Arc::ptr_eq(&index, &c.assets.shared_index()));
        assert!(Arc::ptr_eq(
            &snapshot,
            &c.editor.shared_snapshot(&mut c.assets)?
        ));
        assert_eq!(c.editor.document.state_id(), state);
        assert_eq!(c.editor.document.revision(), revision);
        assert_eq!(c.editor.document.history_len(), 1);
        assert_eq!(c.editor.sources.of_kind(AssetKind::Navigation).count(), 0);
        assert_eq!(fs::read(&c.editor.source)?, source_bytes);
        c.editor.document.redo()?;
        assert_eq!(
            c.editor.document.shared_source().base.entities[0]
                .navigation_region
                .as_ref()
                .unwrap()
                .cell_size,
            0.5,
            "rejected publication must preserve redo"
        );
    }
    Ok(())
}

#[test]
fn undoing_geometry_keeps_refresh_available_but_requires_rebake_for_play_and_plain_worlds_still_start()
-> Result<()> {
    let mut c = Case::new()?;
    c.bake()?;
    c.editor.edit(
        c.floor,
        schema::TRANSLATION,
        FieldValue::Vec3([0.5, -0.25, 0.]),
        &mut c.assets,
    )?;
    c.editor.commit()?;
    c.bake()?;
    c.editor.document.undo()?;
    let old_geometry = c.editor.document.shared_source();
    let history = c.editor.document.history_len();
    c.editor.refresh_resources()?;
    super::resources::wait(&mut c.editor, &mut c.assets)?;
    assert!(
        c.editor.resource_build.failure.is_none(),
        "{:?}",
        c.editor.resource_build.failure
    );
    assert!(Arc::ptr_eq(
        &old_geometry,
        &c.editor.document.shared_source()
    ));
    assert_eq!(c.editor.document.history_len(), history);
    let error = c
        .editor
        .start(&mut c.assets)
        .err()
        .context("stale geometry must not start Play")?;
    assert!(format!("{error:#}").contains("bake"), "{error:#}");
    assert!(c.editor.play.is_none());
    assert!(Arc::ptr_eq(
        &old_geometry,
        &c.editor.document.shared_source()
    ));
    c.editor.document.redo()?;
    c.editor.start(&mut c.assets)?;
    c.editor.stop();

    let mut plain = c.editor.document.expand(&mut c.assets)?;
    for entity in &mut plain.entities {
        entity.navigation_region = None;
        entity.navigation_agent = None;
    }
    c.editor.document = Document::open(plain)?;
    let source = c.editor.document.shared_source();
    c.editor.start(&mut c.assets)?;
    c.editor.play.as_mut().unwrap().advance(1. / 30.)?;
    c.editor.stop();
    assert!(Arc::ptr_eq(&source, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), 0);
    Ok(())
}
