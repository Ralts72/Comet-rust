use anyhow::Result;
use comet_asset_data::AudioData;
use comet_core::{AssetId, EntityId};
use comet_runtime::{FIXED_DELTA, JointKind, JointStatus, PlaySession, SceneResources};
use comet_scene::{
    AudioSource, BodyMotion, EntityData, EntityRef, JointConnection, PhysicsBody, PointJoint,
    SceneSnapshot, Script, ScriptValue,
};
use std::sync::Arc;

fn script(entity: &mut EntityData, text: &str, resources: &mut SceneResources) {
    let asset = AssetId::new();
    entity.script = Some(Script::new(asset));
    resources.scripts.insert(asset, text.into());
}
fn target(entity: &mut EntityData, id: EntityId) {
    entity.script.as_mut().unwrap().parameters.insert(
        "target".into(),
        ScriptValue::Entity(Some(EntityRef::local(id))),
    );
}

#[test]
fn subtree_destruction_finishes_callbacks_then_releases_physics_joints_sound_hud_and_references()
-> Result<()> {
    let mut resources = SceneResources::default();
    let sound = AssetId::new();
    resources.audio.insert(
        sound,
        Arc::new(AudioData {
            version: 1,
            sample_rate: 48_000,
            channels: 1,
            samples: vec![1; 16].into(),
        }),
    );
    let mut actor = EntityData::new("removed actor");
    actor.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        ..Default::default()
    });
    let mut audio = AudioSource::new(sound);
    audio.autoplay = true;
    audio.spatial = true;
    actor.audio_source = Some(audio);
    script(
        &mut actor,
        r#"
        fn on_start(ctx) { ctx.set_hud("temporary"); }
        fn on_fixed_update(ctx, dt) {
            ctx.destroy_entity(ctx.entity()); ctx.destroy_entity(ctx.entity());
            ctx.log("request");
            if !ctx.exists(ctx.entity()) { throw "destruction was not deferred"; }
        }
        fn on_stop(ctx) {
            if !ctx.exists(ctx.entity()) { throw "stop lost its snapshot"; }
            ctx.log("actor stopped");
        }
    "#,
        &mut resources,
    );
    let mut child = EntityData::new("child");
    child.parent = Some(actor.id);
    script(
        &mut child,
        r#"
        fn on_fixed_update(ctx, dt) { ctx.log("child tick"); }
        fn on_stop(ctx) { ctx.log("child stopped"); }
    "#,
        &mut resources,
    );
    let mut survivor = EntityData::new("surviving joint owner");
    survivor.transform.translation = [2., 0., 0.];
    survivor.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    let mut joint = PointJoint::default();
    joint.settings.connection = JointConnection::Body(EntityRef::local(actor.id));
    survivor.point_joint = Some(joint);
    survivor.links.target = Some(EntityRef::local(child.id));
    script(
        &mut survivor,
        r#"
        fn on_start(ctx) { this.target = ctx.param("target"); this.n = 0; }
        fn on_fixed_update(ctx, dt) {
            this.n += 1;
            if this.n == 1 && !ctx.exists(this.target) { throw "partial tick deletion"; }
            if this.n > 1 {
                if ctx.exists(this.target) || ctx.param("target") != () { throw "stale references"; }
                if ctx.raycast([-2,0,0], [3,0,0], 1, ctx.entity()) != () { throw "ghost collider"; }
                ctx.log("gone");
            }
        }
    "#,
        &mut resources,
    );
    target(&mut survivor, actor.id);
    let (actor_id, survivor_id) = (actor.id, survivor.id);
    let author = SceneSnapshot {
        entities: vec![actor, child, survivor],
        ..Default::default()
    };
    let mut play = PlaySession::start_with_resources(author.clone(), resources.clone())?;
    let old_scene = play.shared_snapshot();
    let old_audio = play.audio_frame()?.unwrap();
    assert_eq!(
        play.joint_status(survivor_id, JointKind::Point),
        Some(JointStatus::Active)
    );
    play.set_paused(true);
    play.advance(FIXED_DELTA * 4.)?;
    assert!(Arc::ptr_eq(&old_scene, &play.shared_snapshot()));
    play.single_step()?;
    assert_eq!(play.shared_snapshot().entities.len(), 1);
    assert!(play.hud_frame().is_none());
    assert!(play.audio_frame()?.is_none());
    assert!(play.set_linear_velocity(actor_id, [0.; 3]).is_err());
    assert_eq!(
        play.joint_status(survivor_id, JointKind::Point),
        Some(JointStatus::Unbound)
    );
    assert_eq!(play.shared_snapshot().entities[0].links.target, None);
    assert_eq!(old_scene.entities.len(), 3);
    assert_eq!(old_audio.voices.len(), 1);
    assert!(old_audio.voices[0].playing);
    play.single_step()?;
    play.stop();
    let diagnostics = play.script_diagnostics();
    assert!(diagnostics.iter().all(|d| !d.failed), "{diagnostics:?}");
    for text in ["actor stopped", "child stopped", "child tick", "gone"] {
        assert_eq!(
            diagnostics.iter().filter(|d| d.message == text).count(),
            1,
            "{text}"
        );
    }
    let count = author.entities.len();
    let mut fresh = PlaySession::start_with_resources(author, resources)?;
    assert_eq!(fresh.shared_snapshot().entities.len(), count);
    assert!(fresh.hud_frame().is_some() && fresh.audio_frame()?.is_some());
    Ok(())
}

#[test]
fn a_trigger_can_destroy_its_visitor_without_stale_events_or_a_synthetic_exit() -> Result<()> {
    let mut visitor = EntityData::new("visitor");
    visitor.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        ..Default::default()
    });
    let visitor_id = visitor.id;
    let mut sensor = EntityData::new("sensor");
    sensor.physics = Some(PhysicsBody {
        sensor: true,
        ..Default::default()
    });
    let mut resources = SceneResources::default();
    script(
        &mut sensor,
        r#"
        fn on_trigger(ctx, other, entered) {
            if !entered { throw "synthetic exit"; }
            ctx.destroy_entity(other);
            ctx.log("entered");
        }
    "#,
        &mut resources,
    );
    let mut play = PlaySession::start_with_resources(
        SceneSnapshot {
            entities: vec![sensor, visitor],
            ..Default::default()
        },
        resources,
    )?;
    for _ in 0..3 {
        play.advance(FIXED_DELTA)?;
    }
    assert!(
        play.shared_snapshot()
            .entities
            .iter()
            .all(|e| e.id != visitor_id)
    );
    assert!(play.trigger_events().is_empty());
    let diagnostics = play.script_diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].message, "entered");
    assert!(!diagnostics[0].failed);
    Ok(())
}

#[test]
fn failed_callback_does_not_queue_destruction_or_publish_its_other_commands() -> Result<()> {
    for failure in ["throw \"bad callback\";", "ctx.play_sound(ctx.entity());"] {
        let mut entity = EntityData::new("survivor");
        let mut resources = SceneResources::default();
        script(
            &mut entity,
            &format!(
                r#"
            fn on_fixed_update(ctx, dt) {{
                ctx.destroy_entity(ctx.entity());
                ctx.translate_local(ctx.entity(), 1, 0, 0);
                ctx.set_hud("must not publish");
                ctx.log("must not publish");
                {failure}
            }}
        "#
            ),
            &mut resources,
        );
        let author = SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        };
        let mut play = PlaySession::start_with_resources(author, resources)?;
        let before = play.shared_snapshot();
        play.advance(FIXED_DELTA)?;
        assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
        assert!(play.hud_frame().is_none());
        assert_eq!(play.script_diagnostics().len(), 1);
        assert!(play.script_diagnostics()[0].failed);
        play.advance(FIXED_DELTA)?;
        assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
    }
    Ok(())
}
