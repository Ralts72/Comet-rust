use super::*;
use comet_asset_data::{
    ANIMATION_VERSION, AnimationChannel, AnimationClipData, AnimationInterpolation,
    AnimationValues, SkeletonData, SkeletonNode,
};
use comet_scene::{
    Animator, ContentSource, EntityData, EntityRef, SceneSnapshot, scene::SCENE_VERSION,
};

#[test]
fn scene_handoff_and_shutdown_report_keep_the_current_published_animation_state() -> Result<()> {
    let first = AssetId::new();
    let second = AssetId::new();
    let request = AssetId::new();
    let control = AssetId::new();
    let skeleton = AssetId::new();
    let clip = AssetId::new();
    let node = EntityId::new();
    let mut controller = EntityData::new("current controller");
    let owner = controller.id;
    let mut joint = EntityData::new("current joint");
    joint.parent = Some(owner);
    let target = joint.id;
    controller.animator = Some(Animator {
        controller: None,
        skeleton,
        clip: Some(clip),
        bindings: BTreeMap::from([(node, Some(EntityRef::Local { entity: target }))]),
        looping: false,
        speed: 1.,
    });
    controller.script = Some(comet_scene::Script::new(control));
    let (_products, mut assets) = Products::new(vec![
        (first, scene(vec![entity("requester", Some(request))])?),
        (request, request_script(second)),
        (
            second,
            Product::Scene(ContentSource {
                base: SceneSnapshot {
                    version: SCENE_VERSION,
                    entities: vec![controller, joint],
                },
                ..Default::default()
            }),
        ),
        (
            control,
            script("fn on_start(ctx) { ctx.set_animation_speed(ctx.entity(), 2.0); }"),
        ),
        (
            skeleton,
            Product::Skeleton(SkeletonData {
                version: ANIMATION_VERSION,
                nodes: vec![SkeletonNode {
                    id: node,
                    name: "source joint".into(),
                    parent: None,
                    rest: Default::default(),
                }],
            }),
        ),
        (
            clip,
            Product::AnimationClip(AnimationClipData {
                events: vec![],
                version: ANIMATION_VERSION,
                skeleton,
                duration: 1.,
                channels: vec![AnimationChannel {
                    node,
                    interpolation: AnimationInterpolation::Linear,
                    times: vec![0., 1.],
                    values: AnimationValues::Translation(vec![[0.; 3], [1., 0., 0.]]),
                }],
            }),
        ),
    ])?;
    let mut app = app(&mut assets, first, Some(BTreeSet::from([first, second])))?;
    let previous_world = app.play.world_id();
    let accepted = frame(&mut app, &mut assets, 0)?;
    assert_ne!(accepted.world, previous_world);
    assert_eq!(app.scene, second);
    assert_eq!(app.play.steps(), 0);
    let initial = app.play.animation_state(owner).unwrap();
    assert_eq!(initial.time, 0.);
    assert_eq!(initial.speed, 2.);
    assert!(initial.playing);
    let settled = frame(&mut app, &mut assets, 1)?;
    assert_eq!(app.play.steps(), 0);
    assert!(std::sync::Arc::ptr_eq(
        &accepted.snapshot,
        &settled.snapshot
    ));
    let view = frame(&mut app, &mut assets, 2)?;
    let expected = 2. * comet_runtime::FIXED_DELTA;
    let state = app.play.animation_state(owner).unwrap();
    assert!((state.time - expected).abs() < 1e-12);
    let pose = view
        .snapshot
        .entities
        .iter()
        .find(|e| e.id == target)
        .unwrap();
    assert!((f64::from(pose.transform.translation[0]) - expected).abs() < 1e-7);
    let report = app.report();
    assert_eq!(report["world"], json!(view.world));
    assert_eq!(report["animations"].as_array().unwrap().len(), 1);
    let playback = &report["animations"][0];
    assert_eq!(playback["entity"], json!(owner));
    assert_eq!(playback["time"], json!(state.time));
    assert_eq!(playback["duration"], 1.);
    assert_eq!(playback["speed"], 2.);
    assert_eq!(playback["playing"], true);
    assert_eq!(report["animation_poses"][0]["entity"], json!(target));
    assert_eq!(report["animation_poses"][0]["source"], json!(node));
    assert!(app.play.animation_state(owner).is_none());
    assert!(
        app.play
            .script_diagnostics()
            .iter()
            .all(|entry| !entry.failed)
    );
    Ok(())
}
