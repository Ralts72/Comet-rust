use anyhow::Result;
use comet_core::{Action, ActionEvent, ActionInput, AssetId, EntityId, math::Quat};
use comet_runtime::{FIXED_DELTA, PlaySession};
use comet_scene::{
    BodyMotion, Character, ColliderShape, EntityData, EntityRef, PhysicsBody, SceneSnapshot,
    Script, ScriptValue,
};

fn world(source: &str) -> Result<(PlaySession, EntityId)> {
    let script_id = AssetId::new();
    let mut host = EntityData::new("query host");
    let marker = EntityData::new("marker");
    let mut actor = EntityData::new("actor");
    actor.transform.translation = [0., 2., 3.];
    actor.character = Some(Character::default());
    actor.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    let mut floor = EntityData::new("floor");
    floor.transform.translation = [0., -0.25, 0.];
    floor.physics = Some(PhysicsBody {
        shape: ColliderShape::Box {
            half_extents: [10., 0.25, 10.],
        },
        ..Default::default()
    });
    let mut wall = EntityData::new("wall");
    wall.transform.translation = [0., 1., 0.];
    wall.physics = Some(PhysicsBody::default());
    let mut sensor = EntityData::new("sensor");
    sensor.transform.translation = [-1., 1., 0.];
    sensor.physics = Some(PhysicsBody {
        sensor: true,
        ..Default::default()
    });
    let mut parent = EntityData::new("transformed parent");
    parent.transform.translation = [1., 2., 3.];
    parent.transform.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array();
    parent.transform.scale = [2.; 3];
    let mut child = EntityData::new("child");
    child.parent = Some(parent.id);
    child.transform.translation = [1., 0., 0.];
    let mut script = Script::new(script_id);
    for (key, id) in [
        ("actor", actor.id),
        ("marker", marker.id),
        ("wall", wall.id),
        ("child", child.id),
    ] {
        script
            .parameters
            .insert(key.into(), ScriptValue::Entity(Some(EntityRef::local(id))));
    }
    host.script = Some(script);
    let marker_id = marker.id;
    let scene = SceneSnapshot {
        entities: vec![host, actor, wall, sensor, floor, parent, child, marker],
        ..Default::default()
    };
    Ok((
        PlaySession::start_with_scripts(scene, [(script_id, source.into())].into())?,
        marker_id,
    ))
}
fn tick(play: &mut PlaySession, count: usize, mut input: ActionInput) -> Result<()> {
    for _ in 0..count {
        play.advance_with_input(FIXED_DELTA, input.clone())?;
        input.events.clear();
    }
    Ok(())
}
fn movement() -> ActionInput {
    ActionInput {
        enabled: true,
        movement: [1., 0.],
        ..Default::default()
    }
}

#[test]
fn rays_and_world_points_share_callback_snapshots_then_refresh_after_teleports_and_hierarchy_writes()
-> Result<()> {
    let (mut play, _) = world(
        r#"
        fn on_start(ctx) {
            let actor=ctx.param("actor");let wall=ctx.param("wall");
            let hit=ctx.raycast([-2,1,0],[4,0,0],1,actor);
            if hit.entity.id()!=wall.id() || abs(hit.fraction-0.375)>0.00001 || hit.normal[0]!=-1.0 {throw "nearest non-sensor hit";}
            if ctx.raycast([-2,1,0],[4,0,0],0,actor)!=() {throw "layer mask";}
            let p=ctx.transform_point(ctx.param("child"),0,0,0);
            if abs(p[0]-1.0)>0.00001 || abs(p[2]-1.0)>0.00001 {throw "hierarchy point";}
            ctx.teleport(wall,0,1,5);ctx.set_position(ctx.param("child"),0,0,0);
            if ctx.raycast([-2,1,0],[4,0,0],1,actor)==() {throw "queued physics leaked";}
            if abs(ctx.transform_point(ctx.param("child"),0,0,0)[2]-1.0)>0.00001 {throw "queued transform leaked";}
        }
        fn on_fixed_update(ctx,dt) {
            let actor=ctx.param("actor");
            if ctx.raycast([-2,1,0],[4,0,0],1,actor)!=() {throw "stale physics snapshot";}
            let hit=ctx.raycast([0,1,5],[1,0,0],1,actor);
            if hit.fraction!=0.0 || hit.normal[0]!=-1.0 {throw "solid origin";}
            if abs(ctx.transform_point(ctx.param("child"),0,0,0)[2]-3.0)>0.00001 {throw "stale hierarchy";}
        }
    "#,
    )?;
    tick(&mut play, 2, ActionInput::default())?;
    assert!(
        play.script_diagnostics().is_empty(),
        "{:?}",
        play.script_diagnostics()
    );
    Ok(())
}

#[test]
fn character_control_and_query_failures_preserve_whole_command_batches_and_motion_can_resume()
-> Result<()> {
    for bad in [
        "ctx.face_character(actor,0,0);",
        "ctx.raycast([0,0,0],[0,0,0],1,actor);",
        "for n in 0..129 {ctx.raycast([0,0,0],[1,0,0],0,actor);}",
    ] {
        let source = format!(
            "fn on_start(ctx) {{let actor=ctx.param(\"actor\");ctx.set_scale(ctx.param(\"marker\"),2,2,2);ctx.set_character_motion_enabled(actor,false);{bad}}}"
        );
        let (mut play, marker) = world(&source)?;
        tick(&mut play, 6, movement())?;
        assert!(play.script_diagnostics().iter().any(|d| d.failed));
        assert_eq!(
            play.shared_snapshot()
                .entities
                .iter()
                .find(|e| e.id == marker)
                .unwrap()
                .transform
                .scale,
            [1.; 3]
        );
        assert!(
            play.player_character().unwrap().position[0] > 0.3,
            "failed control commands do not lock the character"
        );
    }
    let (mut play, _) = world(
        r#"
        fn on_start(ctx) {ctx.set_character_motion_enabled(ctx.param("actor"),false);ctx.face_character(ctx.param("actor"),1,0);}
        fn on_fixed_update(ctx,dt) {if ctx.pressed("primary") {ctx.set_character_motion_enabled(ctx.param("actor"),true);}}
    "#,
    )?;
    tick(&mut play, 10, movement())?;
    let state = play.player_character().unwrap();
    assert_eq!(state.position[0], 0.);
    assert!(
        state.position[1] < 2.,
        "gravity still runs while locomotion is disabled"
    );
    assert_eq!(state.facing, Some([1., 0.]));
    let mut input = movement();
    input.events.push(ActionEvent {
        action: Action::Primary,
        pressed: true,
    });
    tick(&mut play, 1, input)?;
    assert!(play.player_character().unwrap().position[0] > 0.05);
    assert_eq!(play.player_character().unwrap().facing, Some([1., 0.]));
    assert!(play.script_diagnostics().is_empty());
    Ok(())
}
