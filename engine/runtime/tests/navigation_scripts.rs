use anyhow::Result;
use comet_asset_data::{NavigationData, NavigationInput};
use comet_core::{AssetId, EntityId};
use comet_runtime::{FIXED_DELTA, NavigationStatus, PlaySession, ScriptLibrary};
use comet_scene::{
    BodyMotion, Character, ColliderShape, EntityData, EntityRef, NavigationAgent, NavigationRegion,
    PhysicsBody, PoseUpdate, SceneSnapshot, Script, ScriptValue,
};
use std::sync::Arc;

struct Fixture {
    audio: comet_audio::AudioLibrary,
    snapshot: SceneSnapshot,
    sources: ScriptLibrary,
    npc: EntityId,
    marker: EntityId,
    body: EntityId,
    goal: EntityId,
    child: EntityId,
    player: EntityId,
    sensor: EntityId,
}
fn character(name: &str, position: [f32; 3], player: bool) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.transform.translation = position;
    entity.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    entity.character = Some(Character {
        player_controlled: player,
        move_speed: 2.,
        ..Default::default()
    });
    entity
}
impl Fixture {
    fn new() -> Self {
        let mut floor = EntityData::new("floor");
        floor.transform.translation = [0., -0.5, 0.];
        floor.physics = Some(PhysicsBody {
            shape: ColliderShape::Box {
                half_extents: [5., 0.5, 5.],
            },
            ..Default::default()
        });
        let mut region = EntityData::new("region");
        region.navigation_region = Some(NavigationRegion {
            data: Some(AssetId::new()),
            half_extents: [5., 4., 5.],
            cell_size: 0.25,
            ..Default::default()
        });
        let mut goal = EntityData::new("authored goal");
        goal.transform.translation = [2., 0., 0.];
        let mut npc = character("npc", [-2., 0.81, 0.], false);
        npc.navigation_agent = Some(NavigationAgent {
            target: Some(EntityRef::local(goal.id)),
            arrival_distance: 0.25,
            ..Default::default()
        });
        let player = character("player", [2., 0.81, 2.], true);
        let marker = EntityData::new("script controller");
        let mut body = EntityData::new("dynamic");
        body.transform.translation = [0., 5., -4.];
        body.physics = Some(PhysicsBody {
            motion: BodyMotion::Dynamic,
            shape: ColliderShape::Box {
                half_extents: [0.2; 3],
            },
            ..Default::default()
        });
        let mut parent = EntityData::new("goal parent");
        parent.transform.translation = [1., 0., 0.];
        let mut child = EntityData::new("nested goal");
        child.parent = Some(parent.id);
        child.transform.translation = [1., 0., 1.];
        let mut sensor = EntityData::new("sensor");
        sensor.transform.translation = [0., 1., 4.];
        sensor.physics = Some(PhysicsBody {
            motion: BodyMotion::Kinematic,
            sensor: true,
            shape: ColliderShape::Box {
                half_extents: [0.5, 1., 0.5],
            },
            ..Default::default()
        });
        Self {
            audio: Default::default(),
            npc: npc.id,
            marker: marker.id,
            body: body.id,
            goal: goal.id,
            child: child.id,
            player: player.id,
            sensor: sensor.id,
            snapshot: SceneSnapshot {
                entities: vec![
                    floor, region, npc, player, goal, marker, body, parent, child, sensor,
                ],
                ..Default::default()
            },
            sources: ScriptLibrary::new(),
        }
    }
    fn bind(&mut self, owner: EntityId, source: &str) {
        let asset = AssetId::new();
        let mut script = Script::new(asset);
        for (name, entity) in [
            ("npc", self.npc),
            ("marker", self.marker),
            ("body", self.body),
            ("goal", self.goal),
            ("child", self.child),
            ("player", self.player),
        ] {
            script.parameters.insert(
                name.into(),
                ScriptValue::Entity(Some(EntityRef::local(entity))),
            );
        }
        script.parameters.insert(
            "external".into(),
            ScriptValue::Entity(Some(EntityRef::External {
                scene: AssetId::new(),
                entity: self.goal,
            })),
        );
        script.parameters.insert(
            "next_scene".into(),
            ScriptValue::Text(AssetId::new().to_string()),
        );
        self.snapshot
            .entities
            .iter_mut()
            .find(|entity| entity.id == owner)
            .unwrap()
            .script = Some(script);
        self.sources.insert(asset, source.into());
    }
    fn start(&self) -> Result<PlaySession> {
        let input = NavigationInput::capture(&self.snapshot)?;
        let data = NavigationData {
            version: 1,
            input_digest: input.digest().into(),
            vertices: vec![
                [-4.5, 0., -4.5],
                [4.5, 0., -4.5],
                [4.5, 0., 4.5],
                [-4.5, 0., 4.5],
            ],
            polygons: vec![vec![0, 1, 2, 3]],
        };
        PlaySession::start_with_resources(
            self.snapshot.clone(),
            comet_runtime::RuntimeResources {
                scripts: self.sources.clone(),
                navigation: Some(Arc::new(data)),
                audio: self.audio.clone(),
                ..Default::default()
            },
        )
    }
}
fn position(play: &mut PlaySession, id: EntityId) -> [f32; 3] {
    play.shared_snapshot()
        .entities
        .iter()
        .find(|entity| entity.id == id)
        .unwrap()
        .transform
        .translation
}
fn ticks(play: &mut PlaySession, count: usize) -> Result<()> {
    for _ in 0..count {
        play.advance(FIXED_DELTA)?;
    }
    Ok(())
}

#[test]
fn mixed_failed_callbacks_discard_navigation_scene_physics_logs_and_scene_requests() -> Result<()> {
    for failure in [
        "ctx.stop_navigation(ctx.param(\"marker\"));",
        "ctx.set_velocity(ctx.param(\"marker\"), 1, 0, 0);",
        "ctx.set_roughness(ctx.param(\"marker\"), 0.5);",
        "ctx.navigate_to(ctx.param(\"npc\"), ctx.param(\"external\"));",
        "ctx.navigate_to(ctx.param(\"npc\"), 1e300, 0, 0);",
        "ctx.navigation_status(ctx.param(\"marker\"));",
        "this.saved = ctx;",
    ] {
        let mut f = Fixture::new();
        f.bind(
            f.marker,
            &format!(
                r#"fn on_start(ctx) {{
            this.changed = 17;
            ctx.translate_local(ctx.param("marker"), 1, 0, 0);
            ctx.set_velocity(ctx.param("body"), 12, 0, 0);
            ctx.stop_navigation(ctx.param("npc"));
            ctx.log("uncommitted");
            ctx.request_scene(ctx.param("next_scene"));
            {failure}
        }}"#
            ),
        );
        let author = f.snapshot.clone();
        let mut play = f.start()?;
        assert_eq!(position(&mut play, f.marker), [0.; 3], "{failure}");
        assert!(play.take_scene_request().is_none(), "{failure}");
        assert_eq!(play.script_diagnostics().len(), 1, "{failure}");
        assert!(play.script_diagnostics()[0].failed, "{failure}");
        ticks(&mut play, 30)?;
        assert!(
            position(&mut play, f.npc)[0] > -1.5,
            "failed stop must not replace the authored goal: {failure}"
        );
        assert!(
            position(&mut play, f.body)[0].abs() < 0.001,
            "failed velocity prefix: {failure}"
        );
        assert_eq!(
            play.script_diagnostics().len(),
            1,
            "failed binding runs once: {failure}"
        );
        assert_eq!(f.snapshot, author);
    }
    Ok(())
}

#[test]
fn audio_commands_join_the_same_navigation_scene_physics_and_vm_commit_boundary() -> Result<()> {
    for failure in [
        "ctx.set_sound_volume(ctx.entity(), 1.1);",
        "ctx.play_sound(ctx.param(\"goal\"));",
        "ctx.stop_navigation(ctx.param(\"marker\"));",
        "ctx.set_velocity(ctx.param(\"marker\"), 1, 0, 0);",
        "ctx.set_roughness(ctx.param(\"marker\"), 0.5);",
        "this.saved = ctx;",
    ] {
        let mut f = Fixture::new();
        let clip = AssetId::new();
        f.snapshot
            .entities
            .iter_mut()
            .find(|entity| entity.id == f.marker)
            .unwrap()
            .audio_source = Some(comet_scene::AudioSource::new(clip));
        f.audio.insert(
            clip,
            Arc::new(comet_asset_data::AudioData {
                version: 1,
                sample_rate: 48_000,
                channels: 1,
                samples: vec![0; 16].into(),
            }),
        );
        f.bind(
            f.marker,
            &format!(
                r#"fn on_start(ctx) {{
            ctx.translate_local(ctx.entity(), 1, 0, 0);
            ctx.set_velocity(ctx.param("body"), 12, 0, 0);
            ctx.stop_navigation(ctx.param("npc"));
            ctx.play_sound(ctx.entity());
            ctx.set_sound_volume(ctx.entity(), 0.2);
            ctx.log("uncommitted sound");
            ctx.request_scene(ctx.param("next_scene"));
            {failure}
        }}"#
            ),
        );
        let mut play = f.start()?;
        let frame = play.audio_frame()?.unwrap();
        assert_eq!(frame.voices[0].generation, 0, "{failure}");
        assert_eq!(frame.voices[0].gain, 1., "{failure}");
        assert!(!frame.voices[0].playing, "{failure}");
        assert_eq!(position(&mut play, f.marker), [0.; 3], "{failure}");
        assert!(play.take_scene_request().is_none(), "{failure}");
        assert_eq!(play.script_diagnostics().len(), 1, "{failure}");
        assert!(play.script_diagnostics()[0].failed, "{failure}");
        ticks(&mut play, 30)?;
        assert!(position(&mut play, f.npc)[0] > -1.5, "{failure}");
        assert!(position(&mut play, f.body)[0].abs() < 0.001, "{failure}");
        assert!(
            Arc::ptr_eq(&frame.voices, &play.audio_frame()?.unwrap().voices),
            "failed binding cannot keep mutating sound intents: {failure}"
        );
    }
    Ok(())
}

#[test]
fn follow_overload_observes_committed_parented_positions_and_actual_character_feet() -> Result<()> {
    for follow_player in [false, true] {
        let mut f = Fixture::new();
        let source = if follow_player {
            r#"fn on_start(ctx) { ctx.teleport(ctx.param("player"), 1, 0.81, -2); ctx.navigate_to(ctx.param("npc"), ctx.param("player")); }"#
        } else {
            r#"fn on_start(ctx) { ctx.set_position(ctx.param("child"), -1, 0, 2); ctx.navigate_to(ctx.param("npc"), ctx.param("child")); }"#
        };
        f.bind(f.marker, source);
        let mut play = f.start()?;
        ticks(&mut play, 180)?;
        let npc = position(&mut play, f.npc);
        let (goal, tolerance) = if follow_player {
            ([1., -2.], 0.9)
        } else {
            ([0., 2.], 0.4)
        };
        assert!(
            (npc[0] - goal[0]).hypot(npc[2] - goal[1]) < tolerance,
            "follow_player={follow_player}, NPC={npc:?}"
        );
        assert!(
            play.script_diagnostics().iter().all(|entry| !entry.failed),
            "{:?}",
            play.script_diagnostics()
        );
        assert_eq!(
            play.shared_snapshot()
                .entities
                .iter()
                .find(|entity| entity.id == f.npc)
                .unwrap()
                .navigation_agent
                .as_ref()
                .unwrap()
                .target,
            Some(EntityRef::local(f.goal))
        );
    }
    Ok(())
}

#[test]
fn script_status_is_initial_then_previous_tick_and_trigger_reads_the_same_frozen_snapshot()
-> Result<()> {
    let mut f = Fixture::new();
    f.bind(f.sensor, r#"
        fn on_start(ctx) { this.t = 0; ctx.log("start:" + ctx.navigation_status(ctx.param("npc"))); ctx.navigate_to(ctx.param("npc"), 3, 0, 0); }
        fn on_fixed_update(ctx, dt) {
            this.t += 1;
            if this.t <= 3 { ctx.log("fixed" + this.t + ":" + ctx.navigation_status(ctx.param("npc"))); }
            if this.t == 2 { ctx.stop_navigation(ctx.param("npc")); }
        }
        fn on_trigger(ctx, other, entered) { if entered && other.id() == ctx.param("npc").id() { ctx.log("trigger:" + ctx.navigation_status(ctx.param("npc"))); } }
        fn on_stop(ctx) { ctx.log("stop:" + ctx.navigation_status(ctx.param("npc"))); }
    "#);
    let mut play = f.start()?;
    ticks(&mut play, 1)?;
    let npc = position(&mut play, f.npc);
    play.teleport(PoseUpdate {
        entity: f.sensor,
        translation: npc,
        rotation: [0., 0., 0., 1.],
    })?;
    ticks(&mut play, 2)?;
    assert_eq!(play.navigation_status(f.npc), Some(NavigationStatus::Idle));
    play.stop();
    let logs: Vec<_> = play
        .script_diagnostics()
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(
        logs,
        [
            "start:idle",
            "fixed1:idle",
            "fixed2:moving",
            "trigger:moving",
            "fixed3:idle",
            "stop:idle"
        ]
    );
    assert!(play.script_diagnostics().iter().all(|entry| !entry.failed));
    Ok(())
}

#[test]
fn stop_intent_persists_over_authored_targets_and_restart_rebuilds_script_and_navigation_state()
-> Result<()> {
    let mut f = Fixture::new();
    f.bind(f.marker, r#"
        fn on_start(ctx) { this.t = 0; ctx.log(ctx.navigation_status(ctx.param("npc"))); }
        fn on_fixed_update(ctx, dt) { this.t += 1; if this.t == 3 { ctx.stop_navigation(ctx.param("npc")); } if this.t == 20 { ctx.navigate_to(ctx.param("npc"), -3, 0, 0); } }
        fn on_stop(ctx) { ctx.log("stopped:" + ctx.navigation_status(ctx.param("npc"))); }
    "#);
    f.bind(f.sensor, r#"fn on_stop(ctx) { ctx.log("discard readonly"); ctx.stop_navigation(ctx.param("npc")); }"#);
    let author = f.snapshot.clone();
    let mut play = f.start()?;
    ticks(&mut play, 3)?;
    let stopped = position(&mut play, f.npc);
    ticks(&mut play, 12)?;
    assert!((position(&mut play, f.npc)[0] - stopped[0]).abs() < 0.001);
    ticks(&mut play, 20)?;
    assert!(position(&mut play, f.npc)[0] < stopped[0] - 0.2);
    let world = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), world);
    assert_eq!(position(&mut play, f.npc), [-2., 0.81, 0.]);
    ticks(&mut play, 2)?;
    assert!(position(&mut play, f.npc)[0] > -2.);
    play.stop();
    assert!(
        play.script_diagnostics()
            .iter()
            .all(|entry| entry.message != "discard readonly")
    );
    assert_eq!(
        play.script_diagnostics()
            .iter()
            .filter(|entry| entry.failed && entry.phase == "on_stop")
            .count(),
        2
    );
    assert_eq!(f.snapshot, author);
    Ok(())
}

#[test]
fn navigation_commands_share_the_exact_existing_budget_and_missing_navigation_is_an_error()
-> Result<()> {
    for scene_commands in [255, 256] {
        let mut f = Fixture::new();
        f.bind(f.marker, &format!(r#"fn on_start(ctx) {{ ctx.log("accepted"); for n in 0..{scene_commands} {{ ctx.translate_local(ctx.param("marker"), 0, 0, 0); }} ctx.stop_navigation(ctx.param("npc")); }}"#));
        let mut play = f.start()?;
        ticks(&mut play, 30)?;
        if scene_commands == 255 {
            assert!((position(&mut play, f.npc)[0] + 2.).abs() < 0.001);
            assert_eq!(play.script_diagnostics()[0].message, "accepted");
            assert!(!play.script_diagnostics()[0].failed);
        } else {
            assert!(position(&mut play, f.npc)[0] > -1.5);
            assert_eq!(play.script_diagnostics().len(), 1);
            assert!(
                play.script_diagnostics()[0].failed
                    && play.script_diagnostics()[0]
                        .message
                        .contains("command budget")
            );
        }
    }
    let mut f = Fixture::new();
    for entity in &mut f.snapshot.entities {
        entity.navigation_region = None;
        entity.navigation_agent = None;
    }
    f.bind(f.marker, r#"fn on_start(ctx) { ctx.translate_local(ctx.entity(), 3, 0, 0); ctx.log("discard"); ctx.stop_navigation(ctx.param("npc")); }"#);
    let mut play = PlaySession::start_with_scripts(f.snapshot, f.sources)?;
    assert_eq!(position(&mut play, f.marker), [0.; 3]);
    ticks(&mut play, 1)?;
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(
        play.script_diagnostics()[0].failed
            && play.script_diagnostics()[0]
                .message
                .contains("navigation is not available")
    );
    Ok(())
}
