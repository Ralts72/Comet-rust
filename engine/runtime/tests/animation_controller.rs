use anyhow::Result;
use comet_asset_data::{
    AnimationChannel, AnimationClipData, AnimationControllerData, AnimationInterpolation,
    AnimationParameter as Parameter, AnimationValues, SkeletonData, SkeletonNode,
};
use comet_core::{AssetId, EntityId};
use comet_runtime::{FIXED_DELTA, PlaySession, SceneResources};
use comet_scene::{Animator, EntityData, EntityRef, SceneSnapshot, Script, Transform};
use std::sync::Arc;

struct Fixture {
    scene: SceneSnapshot,
    resources: SceneResources,
    owner: EntityId,
    bone: EntityId,
    controller: AssetId,
}
fn fixture() -> Fixture {
    let skeleton = AssetId::new();
    let controller = AssetId::new();
    let clips = [
        AssetId::new(),
        AssetId::new(),
        AssetId::new(),
        AssetId::new(),
    ];
    let node = EntityId::new();
    let mut owner = EntityData::new("controller");
    let mut bone = EntityData::new("bone");
    bone.parent = Some(owner.id);
    owner.animator = Some(Animator {
        skeleton,
        controller: Some(controller),
        clip: None,
        bindings: [(node, Some(EntityRef::local(bone.id)))].into(),
        looping: true,
        speed: 1.,
    });
    let definition: AnimationControllerData = serde_json::from_value(serde_json::json!({
        "version":1,"skeleton":skeleton,"initial":"ground",
        "parameters":{"speed":0.,"grounded":true,"rising":false},
        "states":[
            {"name":"ground","parameter":"speed","looping":true,
             "motions":[{"threshold":0.,"clip":clips[0]},{"threshold":10.,"clip":clips[1]}],
             "events":[{"phase":0.,"name":"entry"},{"phase":0.5,"name":"step"}]},
            {"name":"jump","looping":false,"motions":[{"threshold":0.,"clip":clips[2]}]},
            {"name":"fall","looping":false,"motions":[{"threshold":0.,"clip":clips[3]}]}
        ],
        "transitions":[
            {"from":null,"to":"jump","duration":0.1,"conditions":[{"parameter":"grounded","compare":"==","value":false},{"parameter":"rising","compare":"==","value":true}]},
            {"from":null,"to":"fall","duration":0.1,"conditions":[{"parameter":"grounded","compare":"==","value":false},{"parameter":"rising","compare":"==","value":false}]},
            {"from":null,"to":"ground","duration":0.1,"conditions":[{"parameter":"grounded","compare":"==","value":true}]}
        ]
    })).unwrap();
    let animations = clips
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let mut channels = vec![AnimationChannel {
                node,
                interpolation: AnimationInterpolation::Linear,
                times: vec![0., if index == 1 { 2. } else { 1. }],
                values: AnimationValues::Translation(vec![[index as f32 * 10., 0., 0.]; 2]),
            }];
            if index == 0 {
                channels.push(AnimationChannel {
                    node,
                    interpolation: AnimationInterpolation::Step,
                    times: vec![0., if index == 1 { 2. } else { 1. }],
                    values: AnimationValues::Scale(vec![[2.; 3]; 2]),
                });
            }
            if index == 1 {
                channels.push(AnimationChannel {
                    node,
                    interpolation: AnimationInterpolation::Linear,
                    times: vec![0., 2.],
                    values: AnimationValues::Rotation(vec![
                        comet_core::math::Quat::from_rotation_y(
                            std::f32::consts::FRAC_PI_2
                        )
                        .to_array();
                        2
                    ]),
                });
            }
            (
                id,
                Arc::new(AnimationClipData {
                    version: 1,
                    skeleton,
                    duration: if index == 1 { 2. } else { 1. },
                    channels,
                    events: vec![],
                }),
            )
        })
        .collect();
    Fixture {
        owner: owner.id,
        bone: bone.id,
        controller,
        scene: SceneSnapshot {
            entities: vec![owner, bone],
            ..Default::default()
        },
        resources: SceneResources {
            skeletons: [(
                skeleton,
                Arc::new(SkeletonData {
                    version: 1,
                    nodes: vec![SkeletonNode {
                        id: node,
                        name: "bone".into(),
                        parent: None,
                        rest: Transform::default(),
                    }],
                }),
            )]
            .into(),
            animations,
            controllers: [(controller, Arc::new(definition))].into(),
            ..Default::default()
        },
    }
}
fn ticks(play: &mut PlaySession, count: usize) -> Result<()> {
    for _ in 0..count {
        play.advance(FIXED_DELTA)?;
    }
    Ok(())
}
fn pose(play: &mut PlaySession, bone: EntityId) -> Transform {
    play.shared_snapshot()
        .entities
        .iter()
        .find(|e| e.id == bone)
        .unwrap()
        .transform
        .clone()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}

#[test]
fn blend_keeps_normalized_phase_uses_rest_for_missing_channels_and_smooths_parameter_changes()
-> Result<()> {
    let f = fixture();
    let (owner, bone) = (f.owner, f.bone);
    let mut play = PlaySession::start_with_resources(f.scene.clone(), f.resources.clone())?;
    ticks(&mut play, 6)?;
    close(play.animation_time(owner).unwrap(), 0.1);
    play.set_animation_parameter(owner, "speed", Parameter::Number(5.))?;
    ticks(&mut play, 1)?;
    let state = play.animation_state(owner).unwrap();
    close(state.duration, 1.5);
    close(state.time, 0.1 * 1.5 + FIXED_DELTA);
    close(pose(&mut play, bone).translation[0].into(), 5.);
    close(pose(&mut play, bone).scale[0].into(), 1.5);
    let rotation = comet_core::math::Quat::from_array(pose(&mut play, bone).rotation);
    close(rotation.length().into(), 1.);
    assert!(
        rotation
            .dot(comet_core::math::Quat::from_rotation_y(
                std::f32::consts::FRAC_PI_4
            ))
            .abs()
            > 0.99999
    );
    play.set_animation_parameter(owner, "speed", Parameter::Number(100.))?;
    ticks(&mut play, 1)?;
    assert_eq!(pose(&mut play, bone).scale, [1.; 3]);
    assert_eq!(pose(&mut play, bone).translation, [10., 0., 0.]);
    play.restart()?;
    assert_eq!(pose(&mut play, bone).translation, [0.; 3]);
    assert_eq!(
        play.animation_state(owner).unwrap().parameters["speed"],
        Parameter::Number(0.)
    );

    let mut resources = f.resources;
    Arc::make_mut(resources.controllers.get_mut(&f.controller).unwrap()).states[0].smoothing = 0.15;
    let mut play = PlaySession::start_with_resources(f.scene, resources)?;
    play.set_animation_parameter(owner, "speed", Parameter::Number(10.))?;
    ticks(&mut play, 1)?;
    close(
        pose(&mut play, bone).translation[0].into(),
        10. * (1. - (-FIXED_DELTA / 0.15).exp()),
    );
    Ok(())
}

#[test]
fn interrupted_fades_start_at_the_published_pose_and_finished_states_can_transition() -> Result<()>
{
    let mut f = fixture();
    let (owner, bone) = (f.owner, f.bone);
    let controller = Arc::make_mut(f.resources.controllers.get_mut(&f.controller).unwrap());
    // Give a completed jump priority over the generic airborne conditions.
    controller.transitions.insert(
        0,
        serde_json::from_value(serde_json::json!({
            "from":"jump","to":"fall","duration":0.,"after_finished":true
        }))?,
    );
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    play.set_animation_parameter(owner, "grounded", Parameter::Bool(false))?;
    play.set_animation_parameter(owner, "rising", Parameter::Bool(true))?;
    ticks(&mut play, 1)?;
    assert_eq!(
        play.animation_state(owner).unwrap().state.as_deref(),
        Some("jump")
    );
    assert_eq!(pose(&mut play, bone).translation, [0.; 3]);
    ticks(&mut play, 3)?;
    let published = pose(&mut play, bone);
    assert!(published.translation[0] > 0. && published.translation[0] < 20.);
    play.set_animation_parameter(owner, "rising", Parameter::Bool(false))?;
    ticks(&mut play, 1)?;
    assert_eq!(
        play.animation_state(owner).unwrap().state.as_deref(),
        Some("fall")
    );
    assert_eq!(pose(&mut play, bone), published);
    ticks(&mut play, 7)?;
    assert_eq!(pose(&mut play, bone).translation, [30., 0., 0.]);
    play.set_animation_parameter(owner, "rising", Parameter::Bool(true))?;
    ticks(&mut play, 62)?;
    let state = play.animation_state(owner).unwrap();
    assert_eq!(state.state.as_deref(), Some("fall"));
    close(state.time, 0.);
    Ok(())
}

#[test]
fn controller_events_are_single_tracks_and_failed_script_parameter_batches_publish_nothing()
-> Result<()> {
    let mut f = fixture();
    let (owner, bone) = (f.owner, f.bone);
    let script = AssetId::new();
    f.scene.entities[0].script = Some(Script::new(script));
    f.resources.scripts.insert(
        script,
        r#"
        fn on_fixed_update(ctx, dt) {
            ctx.set_scale(ctx.entity(), 3, 3, 3);
            ctx.set_animation_parameter(ctx.entity(), "speed", 8);
            ctx.set_animation_parameter(ctx.entity(), "grounded", 1);
        }
    "#
        .into(),
    );
    let observer_script = AssetId::new();
    let mut observer = EntityData::new("event observer");
    observer.script = Some(Script::new(observer_script));
    f.scene.entities.push(observer);
    f.resources.scripts.insert(
        observer_script,
        "fn on_animation_event(ctx, animator, name) { ctx.log(name); }".into(),
    );
    let mut play = PlaySession::start_with_resources(f.scene, f.resources)?;
    ticks(&mut play, 65)?;
    let state = play.animation_state(owner).unwrap();
    assert_eq!(state.parameters["speed"], Parameter::Number(0.));
    assert_eq!(state.parameters["grounded"], Parameter::Bool(true));
    assert_eq!(pose(&mut play, owner).scale, [1.; 3]);
    assert_eq!(pose(&mut play, bone).translation, [0.; 3]);
    let events: Vec<_> = play
        .script_diagnostics()
        .iter()
        .filter(|d| !d.failed)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(events, ["entry", "step", "entry"]);
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|d| d.failed && d.message.contains("type mismatch"))
    );
    assert!(
        play.set_animation_parameter(owner, "speed", Parameter::Number(f32::NAN))
            .is_err()
    );
    assert!(
        play.set_animation_parameter(owner, "unknown", Parameter::Bool(true))
            .is_err()
    );
    Ok(())
}
