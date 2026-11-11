use anyhow::{Context, Result};
use comet_asset_data::NavigationData;
use comet_core::EntityId;
use comet_core::math::Vec3;
use comet_runtime::{FIXED_DELTA, NavigationStatus, PlaySession, ScriptLibrary};
use comet_scene::{ContentSource, EntityRef, PoseUpdate, SceneSnapshot, ScriptValue};
use std::sync::Arc;

#[test]
fn baked_demo_geometry_routes_a_capsule_around_the_physical_wall_and_restarts() -> Result<()> {
    let mut source =
        ContentSource::from_json(include_bytes!("../../../demo/assets/navigation.scene"))?;
    assert!(source.instances.is_empty());
    let data: NavigationData =
        serde_json::from_slice(include_bytes!("../../../demo/assets/navigation.nav"))?;
    data.validate_for(&source.base)?;
    let owner = source
        .base
        .entities
        .iter()
        .find(|entity| entity.navigation_agent.is_some())
        .unwrap();
    let id = owner.id;
    let Some(EntityRef::Local { entity: target }) = owner.navigation_agent.as_ref().unwrap().target
    else {
        anyhow::bail!("demo needs a local target")
    };
    let target = Vec3::from_array(
        source
            .base
            .entities
            .iter()
            .find(|entity| entity.id == target)
            .unwrap()
            .transform
            .translation,
    );
    // Exercise the baked geometry independently of the demo's patrol state machine.
    for entity in &mut source.base.entities {
        entity.script = None;
    }
    let baseline = source.base.clone();
    let mut play = PlaySession::start_with_resources(
        source.base,
        comet_runtime::SceneResources {
            scripts: ScriptLibrary::new(),
            navigation: Some(Arc::new(data)),
            ..Default::default()
        },
    )?;
    let mut furthest_z = 0_f32;
    for _ in 0..600 {
        play.advance(FIXED_DELTA)?;
        let position =
            Vec3::from_array(play.character(id).context("navigation character")?.position);
        furthest_z = furthest_z.max(position.z.abs());
        // A capsule may pass a box's rounded clearance corner without overlapping it.
        let wall_distance = (position.x.abs() - 0.3)
            .max(0.)
            .hypot((position.z.abs() - 2.).max(0.));
        assert!(
            wall_distance >= 0.298,
            "character entered the wall: {position:?}"
        );
    }
    let position = Vec3::from_array(play.character(id).unwrap().position);
    assert!(furthest_z > 2.3, "NPC did not route around the wall");
    assert!(
        (position.x - target.x).hypot(position.z - target.z) < 0.25,
        "NPC stopped at {position:?}, target {target:?}"
    );
    play.set_paused(true);
    play.restart()?;
    assert!(play.is_paused());
    assert_eq!(play.snapshot(), baseline);
    assert_eq!(play.steps(), 0);
    play.stop();
    Ok(())
}

struct Patrol {
    play: PlaySession,
    author: SceneSnapshot,
    npc: EntityId,
    player: EntityId,
}

fn patrol(unreachable: bool, no_player: bool) -> Result<Patrol> {
    let mut author =
        ContentSource::from_json(include_bytes!("../../../demo/assets/navigation.scene"))?.base;
    let player = author
        .entities
        .iter()
        .find(|entity| {
            entity
                .character
                .as_ref()
                .is_some_and(|c| c.player_controlled)
        })
        .context("demo player")?
        .id;
    let npc = author
        .entities
        .iter_mut()
        .find(|entity| entity.navigation_agent.is_some())
        .context("demo NPC")?;
    let binding = npc.script.as_mut().context("demo patrol binding")?;
    if no_player {
        binding
            .parameters
            .insert("player".into(), ScriptValue::Entity(None));
    }
    let ScriptValue::Entity(Some(EntityRef::Local { entity: patrol_b })) =
        binding.parameters["patrol_b"]
    else {
        anyhow::bail!("demo patrol point must be a local entity")
    };
    let library = ScriptLibrary::from([(
        binding.asset,
        include_str!("../../../demo/assets/scripts/npc_patrol.rhai").to_owned(),
    )]);
    let npc = npc.id;
    if unreachable {
        author
            .entities
            .iter_mut()
            .find(|entity| entity.id == patrol_b)
            .unwrap()
            .transform
            .translation = [30., 0.1, 0.];
    }
    let data: NavigationData =
        serde_json::from_slice(include_bytes!("../../../demo/assets/navigation.nav"))?;
    let play = PlaySession::start_with_resources(
        author.clone(),
        comet_runtime::SceneResources {
            scripts: library,
            navigation: Some(Arc::new(data)),
            ..Default::default()
        },
    )?;
    Ok(Patrol {
        play,
        author,
        npc,
        player,
    })
}

fn position(play: &PlaySession, id: EntityId) -> Vec3 {
    Vec3::from_array(play.character(id).unwrap().position)
}

fn arrivals(play: &PlaySession) -> usize {
    play.script_diagnostics()
        .iter()
        .filter(|entry| entry.message == "巡逻到点，等待后折返。")
        .count()
}

fn healthy(play: &PlaySession) {
    assert!(
        play.script_diagnostics().iter().all(|entry| !entry.failed),
        "script failed: {:?}",
        play.script_diagnostics()
    );
}

#[test]
fn actual_patrol_script_alternates_around_the_wall_waits_and_restarts_without_changing_authoring()
-> Result<()> {
    let mut demo = patrol(false, false)?;
    let original = demo.author.clone();
    let mut previous_arrivals = 0;
    let mut hold = None;
    let mut furthest_z = 0_f32;
    for tick in 0..1000 {
        demo.play.advance(FIXED_DELTA)?;
        let p = position(&demo.play, demo.npc);
        furthest_z = furthest_z.max(p.z.abs());
        let wall_distance = (p.x.abs() - 0.3).max(0.).hypot((p.z.abs() - 2.).max(0.));
        assert!(
            wall_distance >= 0.298,
            "patrol crossed the physical wall: {p:?}"
        );
        let count = arrivals(&demo.play);
        if count != previous_arrivals {
            assert_eq!(count, previous_arrivals + 1);
            let expected = if count % 2 == 1 { 3. } else { -3. };
            assert!(
                (p.x - expected).hypot(p.z) < 0.25,
                "wrong patrol endpoint: {p:?}"
            );
            hold = Some((tick, p));
            previous_arrivals = count;
        } else if let Some((began, point)) = hold
            && tick - began < 55
        {
            assert!(
                (p.x - point.x).hypot(p.z - point.z) < 0.01,
                "NPC skipped its wait"
            );
        }
    }
    healthy(&demo.play);
    assert!(previous_arrivals >= 4 && furthest_z > 2.3);
    assert!(
        !demo
            .play
            .script_diagnostics()
            .iter()
            .any(|entry| entry.message == "进入追逐。")
    );
    demo.play.set_paused(true);
    let frozen = demo.play.snapshot();
    let steps = demo.play.steps();
    demo.play.advance(1.)?;
    assert_eq!(demo.play.snapshot(), frozen);
    assert_eq!(demo.play.steps(), steps);
    demo.play.single_step()?;
    demo.play.advance(0.)?;
    assert_eq!(demo.play.steps(), steps + 1);
    let world = demo.play.world_id();
    demo.play.restart()?;
    assert_ne!(demo.play.world_id(), world);
    assert!(demo.play.is_paused());
    assert_eq!(demo.play.steps(), 0);
    assert_eq!(demo.play.snapshot(), original);
    assert_eq!(
        demo.play.navigation_status(demo.npc),
        Some(NavigationStatus::Idle)
    );
    demo.play.stop();
    assert_eq!(demo.play.navigation_status(demo.npc), None);
    assert_eq!(demo.author, original);
    healthy(&demo.play);
    Ok(())
}

#[test]
fn actual_patrol_script_chases_a_live_character_then_returns_before_resuming_patrol() -> Result<()>
{
    let mut demo = patrol(false, false)?;
    for _ in 0..90 {
        demo.play.advance(FIXED_DELTA)?;
    }
    let npc = position(&demo.play, demo.npc);
    let near = Vec3::new(npc.x - 0.5, 0.82, npc.z - 1.);
    demo.play.teleport(PoseUpdate {
        entity: demo.player,
        translation: near.to_array(),
        rotation: [0., 0., 0., 1.],
    })?;
    let before = (npc.x - near.x).hypot(npc.z - near.z);
    for _ in 0..30 {
        demo.play.advance(FIXED_DELTA)?;
    }
    let npc = position(&demo.play, demo.npc);
    assert!((npc.x - near.x).hypot(npc.z - near.z) < before - 0.15);
    assert!(
        demo.play
            .script_diagnostics()
            .iter()
            .any(|entry| entry.message == "进入追逐。")
    );
    demo.play.teleport(PoseUpdate {
        entity: demo.player,
        translation: [9., 0.82, 9.],
        rotation: [0., 0., 0., 1.],
    })?;
    let previous = arrivals(&demo.play);
    for _ in 0..500 {
        demo.play.advance(FIXED_DELTA)?;
        if arrivals(&demo.play) != previous {
            break;
        }
    }
    assert!(
        demo.play
            .script_diagnostics()
            .iter()
            .any(|entry| entry.message == "脱离追逐，返回巡逻。")
    );
    assert_eq!(arrivals(&demo.play), previous + 1);
    let npc = position(&demo.play, demo.npc);
    assert!(
        (npc.x + 3.).hypot(npc.z) < 0.25,
        "did not return to A: {npc:?}"
    );
    for _ in 0..320 {
        demo.play.advance(FIXED_DELTA)?;
    }
    assert!(
        arrivals(&demo.play) >= previous + 2,
        "patrol did not resume"
    );
    healthy(&demo.play);
    Ok(())
}

#[test]
fn actual_patrol_script_allows_an_unbound_player_and_waits_for_an_unreachable_goal() -> Result<()> {
    for unreachable in [false, true] {
        let mut demo = patrol(unreachable, true)?;
        for _ in 0..400 {
            demo.play.advance(FIXED_DELTA)?;
        }
        healthy(&demo.play);
        if unreachable {
            let npc = position(&demo.play, demo.npc);
            assert!((npc.x + 3.).hypot(npc.z) < 0.01);
            assert_eq!(arrivals(&demo.play), 0);
            assert_eq!(
                demo.play.navigation_status(demo.npc),
                Some(NavigationStatus::Unreachable)
            );
        } else {
            assert!(arrivals(&demo.play) >= 1);
        }
    }
    Ok(())
}
