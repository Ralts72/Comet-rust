use anyhow::Result;
use comet_core::{
    AssetId, EntityId,
    math::{Mat4, Quat, Vec3},
};
use comet_runtime::{FIXED_DELTA, PlaySession, ScriptLibrary};
use comet_scene::{
    BodyMotion, EntityData, EntityRef, PhysicsBody, SceneSnapshot, Script, ScriptValue, Transform,
};

fn scene(source: &str) -> (SceneSnapshot, ScriptLibrary, EntityId) {
    let asset = AssetId::new();
    let mut entity = EntityData::new("script owner");
    entity.script = Some(Script::new(asset));
    let id = entity.id;
    (
        SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        [(asset, source.into())].into(),
        id,
    )
}
fn transform(play: &mut PlaySession, id: EntityId) -> Transform {
    play.snapshot()
        .entities
        .iter()
        .find(|entity| entity.id == id)
        .unwrap()
        .transform
        .clone()
}
fn healthy(play: &PlaySession) {
    assert!(
        play.script_diagnostics().iter().all(|entry| !entry.failed),
        "{:?}",
        play.script_diagnostics()
    );
}
fn same_rotation(actual: [f32; 4], expected: Quat) {
    assert!(
        Quat::from_array(actual).dot(expected).abs() > 1. - 1e-5,
        "{actual:?} != {expected:?}"
    );
}

const READ_PHYSICAL_TRANSFORM: &str = r#"
    let p = ctx.position(e);
    let q = ctx.rotation(e);
    let s = ctx.scale(e);
    if p[0] != 1.0 || p[1] != 2.0 || p[2] != 3.0 ||
       q[0] != 0.0 || q[1] != 0.0 || q[2] != 1.0 || q[3] != 0.0 ||
       s[0] != 2.0 || s[1] != 3.0 || s[2] != 4.0 {
        throw "invalid physical transform query";
    }
"#;
fn physical_transform() -> Transform {
    Transform {
        translation: [1., 2., 3.],
        rotation: [0., 0., 1., 0.],
        scale: [2., 3., 4.],
    }
}

#[test]
fn queued_local_rotations_keep_order_and_getters_read_the_entry_snapshot() -> Result<()> {
    let (snapshot, sources, id) = scene(
        r#"
        fn on_start(ctx) {
            let e = ctx.entity();
            ctx.set_rotation(e, 0, 0.0, 0, -1.0);
            ctx.rotate_local(e, 1, 0.0, 0, 1.5707963267948966);
            ctx.rotate_local(e, 0, 1.0, 0, 1.5707963267948966);
            ctx.set_scale(e, -2, 0.0, 3.5);
            let q = ctx.rotation(e);
            let s = ctx.scale(e);
            if q[3] != 1.0 || q[0] != 0.0 || s[0] != 1.0 || s[1] != 1.0 {
                throw "query observed pending commands";
            }
        }
    "#,
    );
    let author = snapshot.clone();
    let mut play = PlaySession::start_with_scripts(snapshot.clone(), sources)?;
    healthy(&play);
    let actual = transform(&mut play, id);
    let x = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let y = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    same_rotation(actual.rotation, x * y);
    assert!(Quat::from_array(actual.rotation).dot(y * x).abs() < 0.75);
    assert_eq!(actual.scale, [-2., 0., 3.5]);
    assert_eq!(snapshot, author);
    Ok(())
}

#[test]
fn local_transform_commands_do_not_decompose_nonuniform_parent_worlds() -> Result<()> {
    let (mut snapshot, sources, id) = scene(
        r#"
        fn on_start(ctx) {
            ctx.rotate_local(ctx.entity(), 1, 0, 0, 0.6);
            ctx.set_scale(ctx.entity(), -2.0, 3, 0.5);
        }
    "#,
    );
    let mut parent = EntityData::new("nonuniform rotated parent");
    parent.transform.scale = [2., 3., 4.];
    parent.transform.rotation = Quat::from_rotation_z(0.3).to_array();
    snapshot.entities[0].parent = Some(parent.id);
    snapshot.entities[0].transform.rotation = Quat::from_rotation_y(0.4).to_array();
    let expected_rotation = Quat::from_rotation_y(0.4) * Quat::from_rotation_x(0.6);
    let expected = parent.transform.matrix()
        * Mat4::from_scale_rotation_translation(
            Vec3::new(-2., 3., 0.5),
            expected_rotation,
            Vec3::ZERO,
        );
    snapshot.entities.push(parent);
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    healthy(&play);
    same_rotation(transform(&mut play, id).rotation, expected_rotation);
    assert!(comet_scene::world_transforms(&play.snapshot())?[&id].abs_diff_eq(expected, 1e-5));
    Ok(())
}

#[test]
fn setters_normalize_only_accepted_quaternions_and_zero_rotation_keeps_original_bytes() -> Result<()>
{
    let (mut snapshot, sources, id) =
        scene("fn on_start(ctx) { ctx.rotate_local(ctx.entity(), 0, 0.99998, 0, 0); }");
    snapshot.entities[0].transform.rotation = [0., 0., 0., 0.9999];
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    healthy(&play);
    assert_eq!(transform(&mut play, id).rotation, [0., 0., 0., 0.9999]);
    let (snapshot, sources, id) =
        scene("fn on_start(ctx) { ctx.set_rotation(ctx.entity(), 0, 0, 0, -0.9999); }");
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    healthy(&play);
    assert_eq!(transform(&mut play, id).rotation, [0., 0., 0., -1.]);
    Ok(())
}

#[test]
fn invalid_intermediate_transform_commands_discard_the_entire_callback() -> Result<()> {
    for command in [
        "ctx.set_rotation(e, 0, 0, 0, 0);",
        "ctx.set_rotation(e, 0, 0, 0, 2);",
        "ctx.set_rotation(e, 1.0e100, 0, 0, 1);",
        "ctx.rotate_local(e, 0, 0, 0, 1);",
        "ctx.rotate_local(e, 0, 2, 0, 1);",
        "ctx.rotate_local(e, 0, 1, 0, 1.0e100);",
        "ctx.set_scale(e, 1.0e100, 1, 1);",
        "ctx.set_scale(e, 0.0 / 0.0, 1, 1);",
    ] {
        let source = format!(
            "fn on_start(ctx) {{ let e=ctx.entity(); ctx.translate_local(e, 7, 0, 0); ctx.set_scale(e, 2, 2, 2); {command} ctx.set_rotation(e, 0, 0, 0, 1); ctx.set_scale(e, 1, 1, 1); }}"
        );
        let (snapshot, sources, _) = scene(&source);
        let mut play = PlaySession::start_with_scripts(snapshot.clone(), sources)?;
        assert_eq!(play.snapshot(), snapshot, "{command}");
        assert_eq!(
            play.script_diagnostics()
                .iter()
                .filter(|entry| entry.failed)
                .count(),
            1,
            "{command}"
        );
    }
    Ok(())
}

#[test]
fn physical_transforms_are_readable_but_writes_cannot_publish_a_valid_prefix() -> Result<()> {
    for (motion, sensor) in [
        (BodyMotion::Fixed, false),
        (BodyMotion::Dynamic, false),
        (BodyMotion::Kinematic, false),
        (BodyMotion::Fixed, true),
    ] {
        for command in [
            "",
            "ctx.set_rotation(e,0,0,0,1);",
            "ctx.rotate_local(e,0,1,0,0.3);",
            "ctx.set_scale(e,1,1,1);",
        ] {
            let source = format!(
                "fn on_start(ctx) {{ let e=ctx.param(\"body\"); {READ_PHYSICAL_TRANSFORM} ctx.log(\"read\"); ctx.translate_local(ctx.entity(), 7,0,0); {command} }}"
            );
            let (mut snapshot, sources, id) = scene(&source);
            let mut body = EntityData::new("body");
            body.physics = Some(PhysicsBody {
                motion,
                sensor,
                ..Default::default()
            });
            body.transform = physical_transform();
            snapshot.entities[0]
                .script
                .as_mut()
                .unwrap()
                .parameters
                .insert(
                    "body".into(),
                    ScriptValue::Entity(Some(EntityRef::Local { entity: body.id })),
                );
            snapshot.entities.push(body);
            let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
            let diagnostics = play.script_diagnostics();
            assert_eq!(diagnostics.len(), 1);
            if command.is_empty() {
                assert!(!diagnostics[0].failed);
                assert_eq!(diagnostics[0].message, "read");
                assert_eq!(transform(&mut play, id).translation, [7., 0., 0.]);
            } else {
                assert!(diagnostics[0].failed && diagnostics[0].message.contains("physics owns"));
                assert_eq!(transform(&mut play, id).translation, [0.; 3]);
            }
        }
    }
    Ok(())
}

#[test]
fn world_overflow_discards_transforms_and_queued_physical_impulses() -> Result<()> {
    let (mut snapshot, sources, id) = scene(
        r#"
        fn on_start(ctx) {
            ctx.impulse(ctx.param("body"), 10, 0, 0);
            ctx.rotate_local(ctx.entity(), 1, 0, 0, 0.4);
            ctx.set_scale(ctx.entity(), 1.0e20, 1, 1);
        }
    "#,
    );
    let mut parent = EntityData::new("large parent");
    parent.transform.scale = [1e20, 1., 1.];
    snapshot.entities[0].parent = Some(parent.id);
    let original = snapshot.entities[0].transform.clone();
    let mut body = EntityData::new("body");
    body.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    let body_id = body.id;
    snapshot.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert(
            "body".into(),
            ScriptValue::Entity(Some(EntityRef::Local { entity: body_id })),
        );
    snapshot.entities.extend([parent, body]);
    let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|entry| entry.failed && entry.message.contains("world transform overflow"))
    );
    assert_eq!(transform(&mut play, id), original);
    play.advance(FIXED_DELTA)?;
    assert_eq!(
        transform(&mut play, body_id).translation[0],
        0.,
        "the earlier impulse must remain unpublished"
    );
    Ok(())
}

#[test]
fn transform_callbacks_share_fixed_time_pause_restart_and_author_isolation() -> Result<()> {
    let (snapshot, sources, id) = scene(
        r#"
        fn on_fixed_update(ctx, dt) {
            ctx.rotate_local(ctx.entity(), 0, 1, 0, dt);
            ctx.set_scale(ctx.entity(), 1.0 + ctx.time(), 1, 1);
        }
    "#,
    );
    let author = snapshot.clone();
    for fps in [30, 60, 144] {
        let mut play = PlaySession::start_with_scripts(snapshot.clone(), sources.clone())?;
        for _ in 0..fps {
            play.advance(1. / f64::from(fps))?;
        }
        healthy(&play);
        same_rotation(transform(&mut play, id).rotation, Quat::from_rotation_y(1.));
        assert_eq!(transform(&mut play, id).scale, [2., 1., 1.]);
        play.set_paused(true);
        let paused = play.snapshot();
        play.advance(1.)?;
        assert_eq!(play.snapshot(), paused);
        play.single_step()?;
        same_rotation(
            transform(&mut play, id).rotation,
            Quat::from_rotation_y(1. + FIXED_DELTA as f32),
        );
        play.restart()?;
        assert_eq!(play.snapshot(), author);
        assert!(play.is_paused());
    }
    assert_eq!(snapshot, author);
    Ok(())
}

#[test]
fn stop_can_read_physical_transforms_and_never_writes_them() -> Result<()> {
    for command in [
        "",
        "ctx.set_rotation(e,0,0,0,1);",
        "ctx.rotate_local(e,0,1,0,1);",
        "ctx.set_scale(e,2,2,2);",
    ] {
        let (mut snapshot, sources, _) = scene(&format!(
            "fn on_stop(ctx) {{ let e=ctx.entity(); {READ_PHYSICAL_TRANSFORM} ctx.log(\"stop read\"); {command} }}"
        ));
        snapshot.entities[0].physics = Some(PhysicsBody::default());
        snapshot.entities[0].transform = physical_transform();
        let mut play = PlaySession::start_with_scripts(snapshot, sources)?;
        let before = play.snapshot();
        play.stop();
        assert_eq!(play.snapshot(), before);
        let diagnostics = play.script_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        if command.is_empty() {
            assert!(!diagnostics[0].failed);
            assert_eq!(diagnostics[0].message, "stop read");
        } else {
            assert!(
                diagnostics[0].failed
                    && diagnostics[0]
                        .message
                        .contains("unavailable during on_stop")
            );
        }
        let count = play.script_diagnostics().len();
        play.stop();
        assert_eq!(play.script_diagnostics().len(), count);
    }
    Ok(())
}
