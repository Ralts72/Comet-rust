use anyhow::Result;
use comet_asset_data::{
    AnimationChannel, AnimationClipData, AnimationInterpolation, AnimationValues, SkeletonData,
    SkeletonNode,
};
use comet_core::{AssetId, EntityId, math::Quat};
use comet_runtime::{FIXED_DELTA, PlaySession, SceneResources, validate_animations};
use comet_scene::{
    Animator, EntityData, EntityRef, FieldUpdate, SceneSnapshot, Script, ScriptValue, Transform,
    UnboundRig,
    schema::{self, FieldValue},
};
use std::sync::Arc;

struct Rig {
    scene: SceneSnapshot,
    resources: SceneResources,
    owner: EntityId,
    a: EntityId,
    b: EntityId,
    source_a: EntityId,
    skeleton: AssetId,
    clip: AssetId,
}
fn rig() -> Rig {
    let skeleton = AssetId::new();
    let clip = AssetId::new();
    let source_a = EntityId::new();
    let source_b = EntityId::new();
    let mut owner = EntityData::new("actor");
    let mut a = EntityData::new("joint A");
    a.parent = Some(owner.id);
    a.transform.translation = [2., 0., 0.];
    a.transform.scale = [2., 3., 4.];
    let mut b = EntityData::new("joint B");
    b.parent = Some(a.id);
    b.transform.translation = [0., 1., 0.];
    owner.animator = Some(Animator {
        controller: None,
        skeleton,
        clip: Some(clip),
        bindings: [
            (source_a, Some(EntityRef::local(a.id))),
            (source_b, Some(EntityRef::local(b.id))),
        ]
        .into(),
        looping: false,
        speed: 1.,
    });
    let skeleton_data = SkeletonData {
        version: 1,
        nodes: vec![
            SkeletonNode {
                id: source_a,
                name: "A".into(),
                parent: None,
                rest: Transform::default(),
            },
            SkeletonNode {
                id: source_b,
                name: "B".into(),
                parent: Some(source_a),
                rest: b.transform.clone(),
            },
        ],
    };
    let clip_data = AnimationClipData {
        events: vec![],
        version: 1,
        skeleton,
        duration: 1.25,
        channels: vec![
            AnimationChannel {
                node: source_a,
                interpolation: AnimationInterpolation::Linear,
                times: vec![0.25, 1.25],
                values: AnimationValues::Rotation(vec![
                    [0., 0., 0., 1.],
                    Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array(),
                ]),
            },
            AnimationChannel {
                node: source_b,
                interpolation: AnimationInterpolation::Linear,
                times: vec![0., 1.],
                values: AnimationValues::Translation(vec![[0., 1., 0.], [2., 1., 0.]]),
            },
            AnimationChannel {
                node: source_b,
                interpolation: AnimationInterpolation::Step,
                times: vec![0., 1.],
                values: AnimationValues::Scale(vec![[1.; 3], [2., 1., 1.]]),
            },
        ],
    };
    let (owner_id, a_id, b_id) = (owner.id, a.id, b.id);
    Rig {
        scene: SceneSnapshot {
            entities: vec![owner, a, b],
            ..Default::default()
        },
        resources: SceneResources {
            skeletons: [(skeleton, Arc::new(skeleton_data))].into(),
            animations: [(clip, Arc::new(clip_data))].into(),
            ..Default::default()
        },
        owner: owner_id,
        a: a_id,
        b: b_id,
        source_a,
        skeleton,
        clip,
    }
}
fn entity(play: &mut PlaySession, id: EntityId) -> EntityData {
    play.shared_snapshot()
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .clone()
}
fn ticks(play: &mut PlaySession, count: usize) -> Result<()> {
    for _ in 0..count {
        play.advance(FIXED_DELTA)?;
    }
    Ok(())
}

#[test]
fn fixed_sampling_preserves_authored_unanimated_components_and_obeys_absolute_key_times()
-> Result<()> {
    let fixture = rig();
    let author = fixture.scene.clone();
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    assert_eq!(play.animation_time(fixture.owner), Some(0.));
    assert_eq!(
        entity(&mut play, fixture.a).transform.translation,
        [2., 0., 0.]
    );
    assert_eq!(entity(&mut play, fixture.a).transform.scale, [2., 3., 4.]);
    ticks(&mut play, 30)?;
    assert!((play.animation_time(fixture.owner).unwrap() - 0.5).abs() < 1e-12);
    let a = entity(&mut play, fixture.a).transform;
    assert!(
        (Quat::from_array(a.rotation)
            .dot(Quat::from_rotation_y(std::f32::consts::FRAC_PI_8))
            .abs()
            - 1.)
            .abs()
            < 1e-5
    );
    assert_eq!(a.translation, [2., 0., 0.]);
    assert_eq!(a.scale, [2., 3., 4.]);
    let b = entity(&mut play, fixture.b).transform;
    assert!((b.translation[0] - 1.).abs() < 1e-5);
    assert_eq!(b.scale, [1.; 3]);
    ticks(&mut play, 30)?;
    assert_eq!(entity(&mut play, fixture.b).transform.scale, [2., 1., 1.]);
    ticks(&mut play, 30)?;
    let end = play.shared_snapshot();
    ticks(&mut play, 60)?;
    assert!(
        Arc::ptr_eq(&end, &play.shared_snapshot()),
        "nonlooping clips hold their last pose"
    );
    assert_eq!(author.entities[1].transform.rotation, [0., 0., 0., 1.]);
    assert_eq!(author.entities[2].transform.translation, [0., 1., 0.]);
    Ok(())
}

#[test]
fn cubic_trs_uses_segment_seconds_and_signed_quaternion_components_through_play_lifecycle()
-> Result<()> {
    let mut fixture = rig();
    let author = fixture.scene.clone();
    let clip = Arc::make_mut(fixture.resources.animations.get_mut(&fixture.clip).unwrap());
    clip.version = 2;
    clip.duration = 2.25;
    for channel in &mut clip.channels {
        channel.interpolation = AnimationInterpolation::CubicSpline;
        channel.times = vec![0.25, 2.25];
        channel.values = match channel.values {
            AnimationValues::Rotation(_) => AnimationValues::Rotation(vec![
                [0.; 4],
                [0., 0., 0., 1.],
                [0., 0., 4., 0.],
                [0.; 4],
                [0., 0., 0., -1.],
                [0.; 4],
            ]),
            AnimationValues::Translation(_) => AnimationValues::Translation(vec![
                [0.; 3],
                [0., 1., 0.],
                [4., 0., 0.],
                [0.; 3],
                [2., 1., 0.],
                [0.; 3],
            ]),
            AnimationValues::Scale(_) => AnimationValues::Scale(vec![
                [0.; 3],
                [1.; 3],
                [4., 0., 0.],
                [0.; 3],
                [3., 1., 1.],
                [0.; 3],
            ]),
        };
    }
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    assert_eq!(
        entity(&mut play, fixture.a).transform.rotation,
        [0., 0., 0., 1.]
    );
    ticks(&mut play, 15)?;
    assert_eq!(
        entity(&mut play, fixture.b).transform.translation,
        [0., 1., 0.]
    );
    ticks(&mut play, 60)?;
    let pose = entity(&mut play, fixture.b).transform;
    assert!((pose.translation[0] - 2.).abs() < 1e-5);
    assert!((pose.scale[0] - 3.).abs() < 1e-5);
    let rotation = Quat::from_array(entity(&mut play, fixture.a).transform.rotation);
    assert!(
        (rotation
            .dot(Quat::from_rotation_z(std::f32::consts::PI))
            .abs()
            - 1.)
            .abs()
            < 1e-5
    );
    assert_eq!(
        entity(&mut play, fixture.a).transform.translation,
        [2., 0., 0.]
    );
    play.set_paused(true);
    let frozen = play.shared_snapshot();
    play.advance(1.)?;
    assert!(Arc::ptr_eq(&frozen, &play.shared_snapshot()));
    play.single_step()?;
    assert!(entity(&mut play, fixture.b).transform.translation[0] > 2.);
    play.set_paused(false);
    ticks(&mut play, 90)?;
    assert_eq!(
        entity(&mut play, fixture.b).transform.translation,
        [2., 1., 0.]
    );
    assert_eq!(entity(&mut play, fixture.b).transform.scale, [3., 1., 1.]);
    assert_eq!(
        entity(&mut play, fixture.a).transform.rotation,
        [0., 0., 0., -1.]
    );
    play.restart()?;
    assert_eq!(play.animation_time(fixture.owner), Some(0.));
    assert_eq!(
        entity(&mut play, fixture.b).transform.translation,
        [0., 1., 0.]
    );
    assert_eq!(author.entities[1].transform.rotation, [0., 0., 0., 1.]);
    play.stop();
    assert_eq!(play.animation_state(fixture.owner), None);
    Ok(())
}

#[test]
fn a_degenerate_cubic_rotation_keeps_the_last_published_pose_and_clock() -> Result<()> {
    let mut fixture = rig();
    fixture.scene.entities[0].animator.as_mut().unwrap().speed = 30.;
    let clip = Arc::make_mut(fixture.resources.animations.get_mut(&fixture.clip).unwrap());
    clip.version = 2;
    clip.duration = 1.;
    clip.channels[0].interpolation = AnimationInterpolation::CubicSpline;
    clip.channels[0].times = vec![0., 1.];
    clip.channels[0].values = AnimationValues::Rotation(vec![
        [0.; 4],
        [0., 0., 0., 1.],
        [0.; 4],
        [0.; 4],
        [0., 0., 0., -1.],
        [0.; 4],
    ]);
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    let before = play.shared_snapshot();
    let state = play.animation_state(fixture.owner);
    let error = play.advance(FIXED_DELTA).unwrap_err();
    assert!(format!("{error:#}").contains("animation rotation is invalid"));
    assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
    assert_eq!(play.animation_state(fixture.owner), state);
    Ok(())
}

#[test]
fn ownership_rejects_whole_script_and_inspector_batches_but_leaves_other_fields_writable()
-> Result<()> {
    let mut fixture = rig();
    let script = AssetId::new();
    let mut binding = Script::new(script);
    binding.parameters.insert(
        "joint".into(),
        ScriptValue::Entity(Some(EntityRef::local(fixture.a))),
    );
    fixture.scene.entities[0].script = Some(binding);
    fixture.resources.scripts.insert(
        script,
        r#"
        fn on_start(ctx) { ctx.set_hud("原任务"); }
        fn on_fixed_update(ctx, dt) {
            ctx.translate_local(ctx.entity(), 1, 0, 0);
            ctx.set_hud("不应发布");
            ctx.log("不应发布");
            ctx.set_rotation(ctx.param("joint"), 0, 0, 0, 1);
        }
    "#
        .into(),
    );
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    assert!(play.is_animation_controlled(fixture.a, schema::ROTATION));
    assert!(!play.is_animation_controlled(fixture.a, schema::TRANSLATION));
    assert!(!play.is_animation_controlled(fixture.a, schema::MATERIAL));
    let hud = play.hud_frame().unwrap();
    let before = play.shared_snapshot();
    assert!(
        play.set_fields(&[
            FieldUpdate {
                entity: fixture.owner,
                field: schema::TRANSLATION,
                value: FieldValue::Vec3([5., 0., 0.])
            },
            FieldUpdate {
                entity: fixture.a,
                field: schema::ROTATION,
                value: FieldValue::Quat([0., 0., 0., 1.])
            },
        ])
        .is_err()
    );
    assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
    play.set_field(
        &[fixture.a],
        schema::TRANSLATION,
        FieldValue::Vec3([8., 0., 0.]),
    )?;
    play.advance(FIXED_DELTA)?;
    assert_eq!(
        entity(&mut play, fixture.owner).transform.translation,
        [0.; 3]
    );
    assert_eq!(
        entity(&mut play, fixture.a).transform.translation,
        [8., 0., 0.]
    );
    assert!(Arc::ptr_eq(&hud.labels, &play.hud_frame().unwrap().labels));
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(
        play.script_diagnostics()[0].failed
            && play.script_diagnostics()[0]
                .message
                .contains("animation owns")
    );
    assert!(
        play.set_field(
            &[fixture.owner],
            schema::ANIMATOR_SPEED,
            FieldValue::Scalar(0.)
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn missing_bindings_are_typed_only_after_other_resources_and_initial_poses_have_been_validated()
-> Result<()> {
    let mut fixture = rig();
    fixture.scene.entities[0]
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .insert(fixture.source_a, None);
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<UnboundRig>().is_some());
    let skeleton = fixture
        .resources
        .skeletons
        .remove(&fixture.skeleton)
        .unwrap();
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<UnboundRig>().is_none());
    fixture
        .resources
        .skeletons
        .insert(fixture.skeleton, skeleton);
    let clip = Arc::make_mut(fixture.resources.animations.get_mut(&fixture.clip).unwrap());
    clip.channels[1].times = vec![1., 0.];
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<UnboundRig>().is_none());

    let mut partial = rig();
    partial.scene.entities[0]
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .insert(partial.source_a, None);
    let mut bad = rig();
    bad.scene.entities[0].transform.scale = [1e20; 3];
    let data = Arc::make_mut(bad.resources.animations.get_mut(&bad.clip).unwrap());
    if let AnimationValues::Translation(values) = &mut data.channels[1].values {
        values[0] = [1e20, 0., 0.];
    }
    partial.scene.entities.extend(bad.scene.entities);
    partial.resources.skeletons.extend(bad.resources.skeletons);
    partial
        .resources
        .animations
        .extend(bad.resources.animations);
    let error = validate_animations(
        &partial.scene,
        &partial.resources.skeletons,
        &partial.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.downcast_ref::<UnboundRig>().is_none(),
        "an unbound rig must not mask another rig's overflowing sampled pose"
    );
    Ok(())
}

#[test]
fn stale_source_keys_remain_authorable_but_never_hide_bad_resources_or_known_initial_poses()
-> Result<()> {
    let mut fixture = rig();
    let stale = EntityId::new();
    fixture.scene.entities[0]
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .insert(stale, Some(EntityRef::local(fixture.a)));
    let original = fixture.scene.to_json()?;
    let loaded = SceneSnapshot::from_json(&original)?;
    assert_eq!(
        loaded.to_json()?,
        original,
        "only an explicit author action may rebuild the binding table"
    );
    let error = validate_animations(
        &loaded,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<UnboundRig>().unwrap().entities,
        [fixture.owner]
    );
    assert!(
        loaded
            .entities
            .iter()
            .find(|e| e.id == fixture.owner)
            .unwrap()
            .animator
            .as_ref()
            .unwrap()
            .bindings
            .contains_key(&stale)
    );
    let error = PlaySession::start_with_resources(loaded, fixture.resources.clone())
        .err()
        .unwrap();
    assert!(error.downcast_ref::<UnboundRig>().is_some());

    let mut bad_resources = fixture.resources.clone();
    bad_resources.animations.clear();
    let error = validate_animations(
        &fixture.scene,
        &bad_resources.skeletons,
        &bad_resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<UnboundRig>().is_none());
    fixture.scene.entities[0].transform.scale = [1e20; 3];
    let clip = Arc::make_mut(fixture.resources.animations.get_mut(&fixture.clip).unwrap());
    if let AnimationValues::Translation(values) = &mut clip.channels[1].values {
        values[0] = [1e20, 0., 0.];
    }
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.downcast_ref::<UnboundRig>().is_none(),
        "known channels must still be sampled before the stale-map diagnostic"
    );
    Ok(())
}

#[test]
fn removing_an_intermediate_source_node_keeps_an_old_valid_author_graph_repairable() -> Result<()> {
    let mut fixture = rig();
    let skeleton = Arc::make_mut(
        fixture
            .resources
            .skeletons
            .get_mut(&fixture.skeleton)
            .unwrap(),
    );
    skeleton.nodes.retain(|node| node.id != fixture.source_a);
    skeleton.nodes[0].parent = None;
    let clip = Arc::make_mut(fixture.resources.animations.get_mut(&fixture.clip).unwrap());
    clip.channels
        .retain(|channel| channel.node != fixture.source_a);
    clip.duration = 1.;
    let original = fixture.scene.to_json()?;
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<UnboundRig>().unwrap().entities,
        [fixture.owner]
    );
    assert_eq!(fixture.scene.to_json()?, original);

    // Once keys match, a still-incompatible binding topology is a real configuration error.
    fixture.scene.entities[0]
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .remove(&fixture.source_a);
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<UnboundRig>().is_none());
    assert!(error.to_string().contains("hierarchy differs"));

    // An explicit unbind permits author repair; it does not make a dangling local UUID valid.
    let target = fixture.scene.entities[0]
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .values_mut()
        .next()
        .unwrap();
    *target = None;
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<UnboundRig>().is_some());
    *fixture.scene.entities[0]
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .values_mut()
        .next()
        .unwrap() = Some(EntityRef::local(EntityId::new()));
    let error = validate_animations(
        &fixture.scene,
        &fixture.resources.skeletons,
        &fixture.resources.animations,
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.downcast_ref::<UnboundRig>().is_none());
    assert!(error.to_string().contains("missing local entity"));
    Ok(())
}

#[test]
fn pause_single_step_restart_and_candidate_acceptance_preserve_instance_lifetimes() -> Result<()> {
    let fixture = rig();
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    ticks(&mut play, 30)?;
    play.set_paused(true);
    let frozen = play.shared_snapshot();
    play.advance(1.)?;
    assert!(Arc::ptr_eq(&frozen, &play.shared_snapshot()));
    play.single_step()?;
    assert!(entity(&mut play, fixture.b).transform.translation[0] > 1.);
    play.set_suspended(true);
    let frozen = play.shared_snapshot();
    play.advance(1.)?;
    assert!(Arc::ptr_eq(&frozen, &play.shared_snapshot()));
    let mut bad = rig();
    bad.resources.animations.clear();
    assert!(PlaySession::start_with_resources(bad.scene, bad.resources).is_err());
    assert!(Arc::ptr_eq(&frozen, &play.shared_snapshot()));
    let next = rig();
    let candidate = PlaySession::start_with_resources(next.scene, next.resources)?;
    let old_world = play.world_id();
    play.accept_candidate(candidate)?;
    assert_ne!(play.world_id(), old_world);
    assert!(play.is_paused() && play.is_suspended());
    play.set_suspended(false);
    play.single_step()?;
    assert!(entity(&mut play, next.b).transform.translation[0] > 0.);
    let world = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), world);
    assert_eq!(entity(&mut play, next.b).transform.translation[0], 0.);
    play.stop();
    assert!(play.advance(FIXED_DELTA).is_err());
    assert_eq!(play.animation_time(next.owner), None);
    Ok(())
}

#[test]
fn looping_constant_clips_and_no_clip_have_explicit_sampling_rules() -> Result<()> {
    let mut fixture = rig();
    fixture.scene.entities[0].animator.as_mut().unwrap().looping = true;
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    ticks(&mut play, 75)?;
    assert!(entity(&mut play, fixture.b).transform.translation[0].abs() < 1e-5);
    let mut fixture = rig();
    let clip = Arc::make_mut(fixture.resources.animations.get_mut(&fixture.clip).unwrap());
    clip.duration = 0.;
    clip.channels.truncate(1);
    clip.channels[0].times = vec![0.];
    clip.channels[0].values =
        AnimationValues::Rotation(vec![Quat::from_rotation_y(0.3).to_array()]);
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    let constant = play.shared_snapshot();
    ticks(&mut play, 90)?;
    assert!(Arc::ptr_eq(&constant, &play.shared_snapshot()));
    let mut fixture = rig();
    fixture.scene.entities[0].animator.as_mut().unwrap().clip = None;
    fixture.resources.animations.clear();
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    play.set_field(
        &[fixture.a],
        schema::ROTATION,
        FieldValue::Quat(Quat::from_rotation_y(0.5).to_array()),
    )?;
    let pose = play.shared_snapshot();
    ticks(&mut play, 2)?;
    assert!(Arc::ptr_eq(&pose, &play.shared_snapshot()));
    Ok(())
}

#[test]
fn an_overflowing_sample_never_publishes_a_valid_subset_of_bone_fields() -> Result<()> {
    let mut fixture = rig();
    fixture.scene.entities[0].transform.scale = [1e20; 3];
    let clip = Arc::make_mut(fixture.resources.animations.get_mut(&fixture.clip).unwrap());
    clip.channels[0].times[0] = 0.;
    if let AnimationValues::Translation(values) = &mut clip.channels[1].values {
        values[1] = [3e20, 1., 0.];
    }
    let mut play = PlaySession::start_with_resources(fixture.scene, fixture.resources)?;
    let initial = play.shared_snapshot();
    assert!(play.advance(FIXED_DELTA).is_err());
    assert_eq!(play.animation_time(fixture.owner), Some(0.));
    assert!(Arc::ptr_eq(&initial, &play.shared_snapshot()));
    assert_eq!(
        entity(&mut play, fixture.a).transform.rotation,
        [0., 0., 0., 1.]
    );
    assert_eq!(
        entity(&mut play, fixture.b).transform.translation,
        [0., 1., 0.]
    );
    Ok(())
}

#[test]
fn duplicated_rigs_cannot_exceed_the_world_budget_with_individually_valid_products() -> Result<()> {
    let skeleton = AssetId::new();
    let clip = AssetId::new();
    let nodes: Vec<_> = (0..comet_scene::MAX_ANIMATOR_BINDINGS)
        .map(|i| SkeletonNode {
            id: EntityId::new(),
            name: format!("joint {i}"),
            parent: None,
            rest: Transform::default(),
        })
        .collect();
    let channels = nodes
        .iter()
        .map(|node| AnimationChannel {
            node: node.id,
            interpolation: AnimationInterpolation::Step,
            times: vec![0.],
            values: AnimationValues::Translation(vec![[0.; 3]]),
        })
        .collect();
    let skeleton_data = Arc::new(SkeletonData { version: 1, nodes });
    let clip_data = Arc::new(AnimationClipData {
        events: vec![],
        version: 1,
        skeleton,
        duration: 0.,
        channels,
    });
    clip_data.validate_for(&skeleton_data)?;
    let mut scene = SceneSnapshot::default();
    for _ in 0..5 {
        let mut owner = EntityData::new("rig instance");
        let mut bindings = std::collections::BTreeMap::new();
        for node in &skeleton_data.nodes {
            let mut target = EntityData::new("joint");
            target.parent = Some(owner.id);
            bindings.insert(node.id, Some(EntityRef::local(target.id)));
            scene.entities.push(target);
        }
        owner.animator = Some(Animator {
            controller: None,
            skeleton,
            clip: Some(clip),
            bindings,
            looping: true,
            speed: 1.,
        });
        scene.entities.push(owner);
    }
    let error = validate_animations(
        &scene,
        &[(skeleton, skeleton_data)].into(),
        &[(clip, clip_data)].into(),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("animated node budget"));
    assert!(error.downcast_ref::<UnboundRig>().is_none());
    Ok(())
}
