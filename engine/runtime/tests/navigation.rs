use anyhow::Result;
use comet_asset_data::{NavigationData, NavigationInput};
use comet_core::{Action, ActionInput, AssetId, EntityId};
use comet_runtime::{
    FIXED_DELTA, NavigationStatus, PlaySession, ScriptLibrary, validate_navigation,
};
use comet_scene::{
    BodyMotion, Character, ColliderShape, EntityData, EntityRef, FieldUpdate, NavigationAgent,
    NavigationRegion, PhysicsBody, PoseUpdate, SceneSnapshot, Script, ScriptValue,
    schema::{self, FieldValue},
};
use std::sync::Arc;

struct Fixture {
    snapshot: SceneSnapshot,
    npc: EntityId,
    player: EntityId,
    region: EntityId,
    wall: EntityId,
    target: EntityId,
}
fn body(name: &str, position: [f32; 3], half: [f32; 3]) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.transform.translation = position;
    entity.physics = Some(PhysicsBody {
        shape: ColliderShape::Box { half_extents: half },
        ..Default::default()
    });
    entity
}
fn character(name: &str, position: [f32; 3], player: bool) -> EntityData {
    let mut entity = body(name, position, [0.5; 3]);
    entity.physics.as_mut().unwrap().motion = BodyMotion::Kinematic;
    entity.physics.as_mut().unwrap().shape = ColliderShape::Capsule {
        half_height: 0.5,
        radius: 0.3,
    };
    entity.character = Some(Character {
        player_controlled: player,
        move_speed: 2.,
        ..Default::default()
    });
    entity
}
fn fixture() -> Fixture {
    let floor = body("floor", [0., -0.5, 0.], [6., 0.5, 5.]);
    let wall = body("wall", [0., 1.5, 0.], [0.5, 1.5, 2.]);
    let mut region = EntityData::new("region");
    region.navigation_region = Some(NavigationRegion {
        data: Some(AssetId::new()),
        half_extents: [6., 4., 5.],
        ..Default::default()
    });
    let mut target = EntityData::new("target");
    target.transform.translation = [3., 0., 0.];
    let mut npc = character("npc", [-3., 0.81, 0.], false);
    npc.navigation_agent = Some(NavigationAgent {
        target: Some(EntityRef::local(target.id)),
        arrival_distance: 0.25,
        ..Default::default()
    });
    let player = character("player", [-3., 0.81, 3.5], true);
    Fixture {
        npc: npc.id,
        player: player.id,
        region: region.id,
        wall: wall.id,
        target: target.id,
        snapshot: SceneSnapshot {
            entities: vec![floor, wall, region, target, npc, player],
            ..Default::default()
        },
    }
}
fn data(snapshot: &SceneSnapshot) -> Result<Arc<NavigationData>> {
    Ok(Arc::new(NavigationData {
        version: 1,
        input_digest: NavigationInput::capture(snapshot)?.digest().to_owned(),
        vertices: vec![
            [-5.5, 0., -4.5],
            [5.5, 0., -4.5],
            [5.5, 0., 4.5],
            [-5.5, 0., 4.5],
            [-0.95, 0., -2.45],
            [0.95, 0., -2.45],
            [0.95, 0., 2.45],
            [-0.95, 0., 2.45],
        ],
        polygons: vec![
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    }))
}
fn position(play: &mut PlaySession, id: EntityId) -> [f32; 3] {
    play.snapshot()
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .transform
        .translation
}
fn start(f: &Fixture) -> Result<PlaySession> {
    PlaySession::start_with_resources(
        f.snapshot.clone(),
        comet_runtime::SceneResources {
            scripts: ScriptLibrary::new(),
            navigation: Some(data(&f.snapshot)?),
            ..Default::default()
        },
    )
}
fn ticks(play: &mut PlaySession, count: usize) -> Result<()> {
    for _ in 0..count {
        play.advance(FIXED_DELTA)?;
    }
    Ok(())
}

#[test]
fn navigation_drives_real_capsule_around_a_wall_from_its_feet_without_player_input() -> Result<()> {
    let f = fixture();
    let mut play = start(&f)?;
    assert_eq!(play.navigation_status(f.npc), Some(NavigationStatus::Idle));
    let mut max_z = 0f32;
    let mut previous = position(&mut play, f.npc);
    for _ in 0..600 {
        play.advance(FIXED_DELTA)?;
        let p = position(&mut play, f.npc);
        max_z = max_z.max(p[2].abs());
        assert!(
            (p[0] - previous[0]).hypot(p[2] - previous[2]) <= 2. * FIXED_DELTA as f32 + 0.002,
            "navigation speed was multiplied twice"
        );
        assert!(
            p[0].abs() >= 0.79 || p[2].abs() >= 2.29,
            "capsule crossed the wall: {p:?}"
        );
        previous = p;
    }
    assert!(max_z > 2.3, "NPC did not navigate around the wall");
    assert!(
        (previous[0] - 3.).hypot(previous[2]) < 0.4,
        "NPC did not arrive: {previous:?}"
    );
    assert!(play.character(f.npc).unwrap().grounded);
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Arrived)
    );
    let player = position(&mut play, f.player);
    assert!((player[0] + 3.).hypot(player[2] - 3.5) < 0.001);
    let before = player[0];
    for _ in 0..30 {
        play.advance_with_input(
            FIXED_DELTA,
            ActionInput {
                enabled: true,
                movement: [1., 0.],
                ..Default::default()
            },
        )?;
    }
    assert!(position(&mut play, f.player)[0] > before + 0.8);
    Ok(())
}

#[test]
fn disconnected_navigation_and_an_unbound_target_stop_horizontal_motion_but_keep_gravity()
-> Result<()> {
    let f = fixture();
    let mut mesh = (*data(&f.snapshot)?).clone();
    mesh.vertices = vec![
        [-5., 0., -4.],
        [-1., 0., -4.],
        [-1., 0., 4.],
        [-5., 0., 4.],
        [1., 0., -4.],
        [5., 0., -4.],
        [5., 0., 4.],
        [1., 0., 4.],
    ];
    mesh.polygons = vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]];
    let mut play = PlaySession::start_with_resources(
        f.snapshot.clone(),
        comet_runtime::SceneResources {
            scripts: ScriptLibrary::new(),
            navigation: Some(Arc::new(mesh)),
            ..Default::default()
        },
    )?;
    ticks(&mut play, 120)?;
    let p = position(&mut play, f.npc);
    assert!((p[0] + 3.).abs() < 0.01 && p[2].abs() < 0.01);
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Unreachable)
    );
    assert!(play.character(f.npc).unwrap().grounded);
    let mut snapshot = f.snapshot;
    snapshot
        .entities
        .iter_mut()
        .find(|e| e.id == f.npc)
        .unwrap()
        .navigation_agent
        .as_mut()
        .unwrap()
        .target = None;
    let mut play = PlaySession::start_with_resources(
        snapshot.clone(),
        comet_runtime::SceneResources {
            scripts: ScriptLibrary::new(),
            navigation: Some(data(&snapshot)?),
            ..Default::default()
        },
    )?;
    ticks(&mut play, 120)?;
    assert!((position(&mut play, f.npc)[0] + 3.).abs() < 0.01);
    assert_eq!(play.navigation_status(f.npc), Some(NavigationStatus::Idle));
    Ok(())
}

#[test]
fn navigation_rejects_missing_stale_invalid_and_incompatible_products_before_start() -> Result<()> {
    let f = fixture();
    assert!(validate_navigation(&f.snapshot, None).is_err());
    assert!(PlaySession::start(f.snapshot.clone()).is_err());
    let original = data(&f.snapshot)?;
    validate_navigation(&f.snapshot, Some(&original))?;
    let mut changed = f.snapshot.clone();
    changed
        .entities
        .iter_mut()
        .find(|e| e.id == f.wall)
        .unwrap()
        .transform
        .translation[2] += 1.;
    assert!(
        PlaySession::start_with_resources(
            changed,
            comet_runtime::SceneResources {
                scripts: ScriptLibrary::new(),
                navigation: Some(original.clone()),
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut invalid = (*original).clone();
    invalid.polygons[0][0] = 999;
    assert!(
        PlaySession::start_with_resources(
            f.snapshot.clone(),
            comet_runtime::SceneResources {
                scripts: ScriptLibrary::new(),
                navigation: Some(Arc::new(invalid)),
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut incompatible = f.snapshot.clone();
    incompatible
        .entities
        .iter_mut()
        .find(|e| e.id == f.region)
        .unwrap()
        .navigation_region
        .as_mut()
        .unwrap()
        .agent_radius = 0.3;
    assert!(
        PlaySession::start_with_resources(
            incompatible.clone(),
            comet_runtime::SceneResources {
                scripts: ScriptLibrary::new(),
                navigation: Some(data(&incompatible)?),
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut unbaked = f.snapshot.clone();
    unbaked
        .entities
        .iter_mut()
        .find(|e| e.id == f.region)
        .unwrap()
        .navigation_region
        .as_mut()
        .unwrap()
        .data = None;
    assert!(
        PlaySession::start_with_resources(
            unbaked,
            comet_runtime::SceneResources {
                scripts: ScriptLibrary::new(),
                navigation: Some(original),
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut external = f.snapshot.clone();
    external
        .entities
        .iter_mut()
        .find(|e| e.id == f.npc)
        .unwrap()
        .navigation_agent
        .as_mut()
        .unwrap()
        .target = Some(EntityRef::External {
        scene: AssetId::new(),
        entity: f.target,
    });
    assert!(
        PlaySession::start_with_resources(
            external.clone(),
            comet_runtime::SceneResources {
                scripts: ScriptLibrary::new(),
                navigation: Some(data(&external)?),
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut no_region = f.snapshot.clone();
    no_region
        .entities
        .iter_mut()
        .for_each(|e| e.navigation_region = None);
    assert!(PlaySession::start(no_region.clone()).is_err());
    no_region
        .entities
        .iter_mut()
        .find(|e| e.id == f.npc)
        .unwrap()
        .navigation_agent
        .as_mut()
        .unwrap()
        .enabled = false;
    PlaySession::start(no_region)?;
    Ok(())
}

#[test]
fn navigation_lifecycle_rebuilds_from_accepted_resources_and_keeps_authoring_isolated() -> Result<()>
{
    let f = fixture();
    let author = f.snapshot.clone();
    let mut play = start(&f)?;
    ticks(&mut play, 30)?;
    play.set_paused(true);
    let paused = position(&mut play, f.npc);
    ticks(&mut play, 10)?;
    assert_eq!(position(&mut play, f.npc), paused);
    play.single_step()?;
    assert_ne!(position(&mut play, f.npc), paused);
    play.set_suspended(true);
    assert!(play.single_step().is_err());
    let world = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), world);
    assert!(play.is_paused() && play.is_suspended());
    assert_eq!(position(&mut play, f.npc), [-3., 0.81, 0.]);
    let other = fixture();
    play.accept_candidate(start(&other)?)?;
    assert!(play.is_paused() && play.is_suspended());
    play.set_suspended(false);
    play.set_paused(false);
    ticks(&mut play, 31)?;
    assert_ne!(position(&mut play, other.npc), [-3., 0.81, 0.]);
    assert_eq!(f.snapshot, author);
    play.stop();
    assert!(play.advance(FIXED_DELTA).is_err());
    Ok(())
}

#[test]
fn baked_geometry_is_frozen_through_public_and_script_transactions_without_affecting_old_worlds()
-> Result<()> {
    let mut f = fixture();
    let outside = body("outside static", [20., 1., 0.], [0.5; 3]);
    let outside_id = outside.id;
    let mut dynamic = body("dynamic", [4., 4., 4.], [0.2; 3]);
    dynamic.physics.as_mut().unwrap().motion = BodyMotion::Dynamic;
    let dynamic_id = dynamic.id;
    f.snapshot.entities.extend([outside, dynamic]);
    let script = AssetId::new();
    let target = f
        .snapshot
        .entities
        .iter_mut()
        .find(|e| e.id == f.target)
        .unwrap();
    target.script = Some(Script::new(script));
    target.script.as_mut().unwrap().parameters.insert(
        "wall".into(),
        ScriptValue::Entity(Some(EntityRef::local(f.wall))),
    );
    let scripts = ScriptLibrary::from([(
        script,
        r#"fn on_fixed_update(ctx, dt) {
        ctx.translate_local(ctx.entity(), 1, 0, 0);
        ctx.teleport(ctx.param("wall"), 20, 1.5, 0);
    }"#
        .into(),
    )]);
    let mut play = PlaySession::start_with_resources(
        f.snapshot.clone(),
        comet_runtime::SceneResources {
            scripts,
            navigation: Some(data(&f.snapshot)?),
            ..Default::default()
        },
    )?;
    ticks(&mut play, 1)?;
    assert_eq!(position(&mut play, f.target), [3., 0., 0.]);
    assert!(
        play.teleport(PoseUpdate {
            entity: outside_id,
            translation: [0., 1., 0.],
            rotation: [0., 0., 0., 1.]
        })
        .is_err()
    );
    play.teleport(PoseUpdate {
        entity: dynamic_id,
        translation: [4., 5., 4.],
        rotation: [0., 0., 0., 1.],
    })?;
    assert_eq!(position(&mut play, dynamic_id), [4., 5., 4.]);
    assert_eq!(position(&mut play, f.wall), [0., 1.5, 0.]);
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|d| d.message.contains("navigation freezes"))
    );
    assert!(
        play.teleport(PoseUpdate {
            entity: f.wall,
            translation: [20., 1.5, 0.],
            rotation: [0., 0., 0., 1.]
        })
        .is_err()
    );
    assert!(
        play.set_fields(&[
            FieldUpdate {
                entity: f.target,
                field: schema::TRANSLATION,
                value: FieldValue::Vec3([5., 0., 0.])
            },
            FieldUpdate {
                entity: f.region,
                field: schema::TRANSLATION,
                value: FieldValue::Vec3([1., 0., 0.])
            },
        ])
        .is_err()
    );
    assert_eq!(position(&mut play, f.target), [3., 0., 0.]);
    play.teleport(PoseUpdate {
        entity: f.npc,
        translation: [-3., 0.81, 1.],
        rotation: [0., 0., 0., 1.],
    })?;
    assert_eq!(position(&mut play, f.npc), [-3., 0.81, 1.]);
    f.snapshot.entities.iter_mut().for_each(|e| {
        e.navigation_region = None;
        e.navigation_agent = None;
        e.script = None;
    });
    let mut old = PlaySession::start(f.snapshot)?;
    old.teleport(PoseUpdate {
        entity: f.wall,
        translation: [20., 1.5, 0.],
        rotation: [0., 0., 0., 1.],
    })?;
    assert_eq!(position(&mut old, f.wall), [20., 1.5, 0.]);
    Ok(())
}

#[test]
fn region_motion_is_rejected_before_script_start_and_fixed_callback_prefixes_publish() -> Result<()>
{
    for callback in ["on_start(ctx)", "on_fixed_update(ctx, dt)"] {
        let mut f = fixture();
        let asset = AssetId::new();
        let target = f
            .snapshot
            .entities
            .iter_mut()
            .find(|e| e.id == f.target)
            .unwrap();
        target.script = Some(Script::new(asset));
        target.script.as_mut().unwrap().parameters.insert(
            "region".into(),
            ScriptValue::Entity(Some(EntityRef::local(f.region))),
        );
        let script = format!(
            "fn {callback} {{ ctx.translate_local(ctx.entity(), 1, 0, 0); ctx.translate_local(ctx.param(\"region\"), 1, 0, 0); }}"
        );
        let mut play = PlaySession::start_with_resources(
            f.snapshot.clone(),
            comet_runtime::SceneResources {
                scripts: ScriptLibrary::from([(asset, script)]),
                navigation: Some(data(&f.snapshot)?),
                ..Default::default()
            },
        )?;
        ticks(&mut play, 1)?;
        assert_eq!(position(&mut play, f.target), [3., 0., 0.]);
        assert_eq!(position(&mut play, f.region), [0.; 3]);
        assert!(
            play.script_diagnostics()
                .iter()
                .any(|d| d.message.contains("navigation region transforms"))
        );
    }
    Ok(())
}

#[test]
fn runtime_goals_and_persistent_stop_leave_author_targets_unchanged_and_reset_with_the_world()
-> Result<()> {
    let f = fixture();
    let mut play = start(&f)?;
    ticks(&mut play, 1)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Moving)
    );
    play.stop_navigation(f.npc)?;
    // Commands do not invent a new steering result before the next fixed step.
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Moving)
    );
    let stopped = position(&mut play, f.npc);
    ticks(&mut play, 60)?;
    assert_eq!(play.navigation_status(f.npc), Some(NavigationStatus::Idle));
    let p = position(&mut play, f.npc);
    assert!((p[0] - stopped[0]).hypot(p[2] - stopped[2]) < 0.001);
    assert!(play.stop_navigation(f.player).is_err());
    assert!(play.follow_navigation(f.npc, EntityId::new()).is_err());
    ticks(&mut play, 1)?;
    assert_eq!(play.navigation_status(f.npc), Some(NavigationStatus::Idle));

    play.navigate_to(f.npc, [-3., 0., -3.])?;
    ticks(&mut play, 180)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Arrived)
    );
    let p = position(&mut play, f.npc);
    assert!((p[0] + 3.).hypot(p[2] + 3.) < 0.3);
    let snapshot = play.snapshot();
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|e| e.id == f.npc)
            .unwrap()
            .navigation_agent,
        f.snapshot
            .entities
            .iter()
            .find(|e| e.id == f.npc)
            .unwrap()
            .navigation_agent,
    );

    play.stop_navigation(f.npc)?;
    let world = play.world_id();
    play.restart()?;
    assert_ne!(world, play.world_id());
    assert_eq!(play.navigation_status(f.npc), Some(NavigationStatus::Idle));
    ticks(&mut play, 2)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Moving)
    );

    let next = fixture();
    play.accept_candidate(start(&next)?)?;
    assert_eq!(play.navigation_status(f.npc), None);
    assert_eq!(
        play.navigation_status(next.npc),
        Some(NavigationStatus::Idle)
    );
    ticks(&mut play, 2)?;
    assert_eq!(
        play.navigation_status(next.npc),
        Some(NavigationStatus::Moving)
    );
    play.stop();
    assert_eq!(play.navigation_status(next.npc), None);
    assert!(play.navigate_to(next.npc, [0.; 3]).is_err());
    assert!(play.follow_navigation(next.npc, next.player).is_err());
    assert!(play.stop_navigation(next.npc).is_err());
    Ok(())
}

#[test]
fn finite_extreme_targets_are_unreachable_without_panics_or_nonfinite_motion_and_bad_goals_abort()
-> Result<()> {
    let f = fixture();
    let mut play = start(&f)?;
    for point in [
        [f32::MAX, 0., 0.],
        [-f32::MAX, f32::MAX, f32::MAX],
        [1e30, -1e30, 1e30],
        [100., 0., 100.],
    ] {
        play.navigate_to(f.npc, point)?;
        let before = position(&mut play, f.npc);
        ticks(&mut play, 2)?;
        assert_eq!(
            play.navigation_status(f.npc),
            Some(NavigationStatus::Unreachable)
        );
        let after = position(&mut play, f.npc);
        assert!(after.into_iter().all(f32::is_finite));
        assert!((after[0] - before[0]).hypot(after[2] - before[2]) < 0.001);
    }
    for point in [[f32::NAN, 0., 0.], [0., f32::INFINITY, 0.]] {
        assert!(play.navigate_to(f.npc, point).is_err());
    }
    assert!(play.navigate_to(f.player, [0.; 3]).is_err());
    ticks(&mut play, 1)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Unreachable)
    );
    play.navigate_to(f.npc, [-3., 0., -3.])?;
    ticks(&mut play, 1)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Moving)
    );
    Ok(())
}

#[test]
fn following_a_character_uses_live_feet_through_crouching_instead_of_its_center() -> Result<()> {
    let f = fixture();
    let mut play = start(&f)?;
    play.follow_navigation(f.npc, f.player)?;
    ticks(&mut play, 60)?;
    assert!(position(&mut play, f.npc)[2] > 1.5);
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Moving)
    );
    let mut crouch = ActionInput {
        enabled: true,
        ..Default::default()
    };
    crouch.held.set(Action::Crouch, true);
    for _ in 0..30 {
        play.advance_with_input(FIXED_DELTA, crouch.clone())?;
        assert!(matches!(
            play.navigation_status(f.npc),
            Some(NavigationStatus::Moving | NavigationStatus::Arrived)
        ));
    }
    assert!(play.player_character().unwrap().crouched);
    play.teleport(PoseUpdate {
        entity: f.player,
        translation: [-3., 0.81, -3.5],
        rotation: [0., 0., 0., 1.],
    })?;
    ticks(&mut play, 180)?;
    assert!(position(&mut play, f.npc)[2] < -2.);
    Ok(())
}

#[test]
fn following_an_authored_child_recomputes_its_world_target_after_parent_motion() -> Result<()> {
    let mut f = fixture();
    let mut parent = EntityData::new("target parent");
    parent.transform.translation = [2., 0., -3.];
    let target = f
        .snapshot
        .entities
        .iter_mut()
        .find(|e| e.id == f.target)
        .unwrap();
    target.parent = Some(parent.id);
    target.transform.translation = [1., 0., 0.];
    let parent_id = parent.id;
    f.snapshot.entities.push(parent);
    let mut play = start(&f)?;
    play.follow_navigation(f.npc, f.target)?;
    ticks(&mut play, 360)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Arrived)
    );
    let p = position(&mut play, f.npc);
    assert!((p[0] - 3.).hypot(p[2] + 3.) < 0.3);
    play.set_field(
        &[parent_id],
        schema::TRANSLATION,
        FieldValue::Vec3([2., 0., 3.]),
    )?;
    ticks(&mut play, 240)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::Arrived)
    );
    let p = position(&mut play, f.npc);
    assert!((p[0] - 3.).hypot(p[2] - 3.) < 0.3);
    Ok(())
}

#[test]
fn off_mesh_agents_report_real_sampling_failure_and_stop_suppresses_previous_velocity() -> Result<()>
{
    let f = fixture();
    let mut play = start(&f)?;
    ticks(&mut play, 1)?;
    play.teleport(PoseUpdate {
        entity: f.npc,
        translation: [20., 0.81, 0.],
        rotation: [0., 0., 0., 1.],
    })?;
    ticks(&mut play, 1)?;
    assert_eq!(
        play.navigation_status(f.npc),
        Some(NavigationStatus::OffMesh)
    );
    assert_eq!(position(&mut play, f.npc)[0], 20.);
    play.stop_navigation(f.npc)?;
    ticks(&mut play, 1)?;
    assert_eq!(play.navigation_status(f.npc), Some(NavigationStatus::Idle));
    assert_eq!(position(&mut play, f.npc)[0], 20.);
    Ok(())
}
