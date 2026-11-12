use anyhow::{Context, Result, ensure};
use comet_asset_data::{
    AnimationChannel, AnimationClipData, AnimationInterpolation, AnimationValues, AudioData,
    SkeletonData, SkeletonNode,
};
use comet_core::{
    Action, ActionEvent, ActionInput, AssetId, EntityId, InstanceId, ProgressValue, ProgressValues,
};
use comet_runtime::{AnimationState, FIXED_DELTA, PlaySession, SceneResources, ScriptLibrary};
use comet_scene::{
    Animator, ContentSource, EntityData, EntityRef, Model, PhysicsBody, PrefabResolver,
    SceneSnapshot, SkinBinding, Surface, Transform,
};
use std::{collections::BTreeMap, sync::Arc};

const OWNER: &str = "b4000000-0000-4000-8000-000000000001";
const PLAYER: &str = "b4000000-0000-4000-8000-000000000002";
const VISUAL: &str = "b4000000-0000-4000-8000-000000000003";
const MODEL: &str = "a0000000-0000-4000-8000-000000000010";
const INSTANCE: &str = "b4000000-0000-4000-8000-000000000010";
const SCENE: &str = "a0000000-0000-4000-8000-00000000000e";
const INITIAL: &str = "收集目标：0 / 1\n走近金色方块收集。";
const PLAYING: &str = "收集目标：1 / 1\n动作播放中，请稍候。";
const COMPLETE: &str = "收集目标：1 / 1\n动作完成！重新按 E 前往第二关。";
const COLLECTED: &str = "已收集目标：1 / 1。";

fn source() -> Result<SceneSnapshot> {
    ContentSource::from_json(include_bytes!("../../../demo/assets/collect.scene"))?
        .expand(&mut PreparedPrefab(prefab()))
}

fn model() -> AssetId {
    MODEL.parse().unwrap()
}

fn animator_id() -> EntityId {
    EntityId::in_asset(model(), "animation:controller")
        .in_instance(INSTANCE.parse::<InstanceId>().unwrap())
}

fn skeleton() -> SkeletonData {
    let nodes = [0, 1, 2].map(|index| EntityId::in_asset(model(), &format!("node:index:{index}")));
    SkeletonData {
        version: 1,
        nodes: nodes
            .iter()
            .enumerate()
            .map(|(index, id)| SkeletonNode {
                id: *id,
                name: format!("Node {index}"),
                parent: (index == 2).then_some(nodes[1]),
                rest: Transform {
                    translation: if index == 2 { [0., 1., 0.] } else { [0.; 3] },
                    ..Default::default()
                },
            })
            .collect(),
    }
}

// Prepared CPU products isolate Runtime behavior; real glTF import/packaging is tested by Pipeline.
struct PreparedPrefab(ContentSource);
impl PrefabResolver for PreparedPrefab {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        ensure!(id == model(), "unexpected prefab");
        Ok(self.0.clone())
    }
}
fn prefab() -> ContentSource {
    let skeleton = skeleton();
    let controller = EntityId::in_asset(model(), "animation:controller");
    let mut content = ContentSource::default();
    content.base.entities = skeleton
        .nodes
        .iter()
        .map(|node| {
            let mut entity = EntityData::new(&node.name);
            entity.id = node.id;
            entity.parent = Some(node.parent.unwrap_or(controller));
            entity.transform = node.rest.clone();
            entity
        })
        .collect();
    let mut owner = EntityData::new("prepared SimpleSkin controller");
    owner.id = controller;
    owner.animator = Some(Animator {
        controller: None,
        skeleton: model().child("skeleton:scene"),
        clip: Some(model().child("animation:index:0")),
        bindings: skeleton
            .nodes
            .iter()
            .map(|node| (node.id, Some(EntityRef::local(node.id))))
            .collect(),
        looping: true,
        speed: 1.,
    });
    let mut mesh = EntityData::new("prepared SimpleSkin mesh");
    mesh.id = EntityId::in_asset(model(), "node:index:0/primitive:0");
    mesh.parent = Some(skeleton.nodes[0].id);
    mesh.model = Some(Model {
        mesh: model().child("mesh:index:0/primitive:0"),
        surface: Surface {
            material: model().child("material:default"),
            roughness: 1.,
        },
    });
    mesh.skin = Some(SkinBinding {
        skin: model().child("skin:index:0"),
        animator: Some(EntityRef::local(controller)),
    });
    content.base.entities.extend([owner, mesh]);
    content
}

fn resources(author: &SceneSnapshot) -> Result<SceneResources> {
    let owner = author
        .entities
        .iter()
        .find(|entity| entity.id.to_string() == OWNER)
        .unwrap();
    let script = owner.script.as_ref().unwrap().asset;
    let clip = owner.audio_source.as_ref().unwrap().clip;
    let animator = author
        .entities
        .iter()
        .find(|entity| entity.id == animator_id())
        .unwrap()
        .animator
        .as_ref()
        .unwrap();
    let source = model();
    let nodes = [0, 1, 2].map(|index| EntityId::in_asset(source, &format!("node:index:{index}")));
    let skeleton = Arc::new(skeleton());
    // The runtime consumes prepared CPU products; reuse every key from the sole official source.
    let bytes = include_bytes!("../../../demo/assets/models/simple_skin/SimpleSkin_animation.bin");
    let values: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    let rotations = values[12..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|key| {
            let length = key
                .iter()
                .map(|value| f64::from(*value).powi(2))
                .sum::<f64>()
                .sqrt();
            std::array::from_fn(|index| (f64::from(key[index]) / length) as f32)
        })
        .collect();
    let animation = Arc::new(AnimationClipData {
        events: vec![comet_asset_data::AnimationEvent {
            phase: 1.,
            name: "动作完成".into(),
        }],
        version: comet_asset_data::ANIMATION_CLIP_VERSION,
        skeleton: animator.skeleton,
        duration: values[11],
        channels: vec![AnimationChannel {
            node: nodes[2],
            interpolation: AnimationInterpolation::Linear,
            times: values[..12].to_vec(),
            values: AnimationValues::Rotation(rotations),
        }],
    });
    animation.validate_for(&skeleton)?;
    Ok(SceneResources {
        scripts: ScriptLibrary::from([(
            script,
            include_str!("../../../demo/assets/scripts/collect.rhai").into(),
        )]),
        // Decoder, real PCM and device regressions remain at the asset/output boundaries.
        audio: BTreeMap::from([(
            clip,
            Arc::new(AudioData {
                version: 1,
                sample_rate: 48_000,
                channels: 1,
                samples: Arc::from([0, 1200, -1200, 0]),
            }),
        )]),
        skeletons: [(animator.skeleton, skeleton)].into(),
        animations: [(animator.clip.unwrap(), animation)].into(),
        ..Default::default()
    })
}

fn animation(play: &PlaySession) -> AnimationState {
    play.animation_state(animator_id()).unwrap()
}

fn start(author: &SceneSnapshot) -> Result<PlaySession> {
    PlaySession::start_with_progress(
        author.clone(),
        resources(author)?,
        SCENE.parse()?,
        Arc::new(ProgressValues::default()),
    )
}

fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}

fn hud(play: &PlaySession, expected: &str) {
    let frame = play.hud_frame().expect("collect HUD");
    assert_eq!(frame.world, play.world_id());
    assert_eq!(frame.labels.len(), 1);
    assert_eq!(frame.labels[0].owner.to_string(), OWNER);
    assert_eq!(frame.labels[0].text.as_ref(), expected);
}

fn sound(play: &mut PlaySession) -> Result<(u64, bool)> {
    let frame = play.audio_frame()?.context("collect audio frame")?;
    assert_eq!(frame.world, play.world_id());
    assert_eq!(frame.voices.len(), 1);
    let voice = &frame.voices[0];
    assert_eq!(voice.entity.to_string(), OWNER);
    Ok((voice.generation, voice.playing))
}

fn visual_scale(play: &mut PlaySession) -> Option<[f32; 3]> {
    play.shared_snapshot()
        .entities
        .iter()
        .find(|entity| entity.id.to_string() == VISUAL)
        .map(|entity| entity.transform.scale)
}

fn collected(play: &PlaySession) -> usize {
    play.script_diagnostics()
        .iter()
        .filter(|entry| entry.message == COLLECTED)
        .count()
}

#[test]
fn actual_collect_demo_moves_into_the_sensor_once_then_requires_a_fresh_interaction() -> Result<()>
{
    let author = source()?;
    let bytes = author.to_json()?;
    let original_scale = author
        .entities
        .iter()
        .find(|entity| entity.id.to_string() == VISUAL)
        .unwrap()
        .transform
        .scale;
    let mut play = start(&author)?;
    hud(&play, INITIAL);
    assert_eq!(animation(&play).time, 0.);
    assert!(!animation(&play).playing);
    assert_eq!(sound(&mut play)?, (0, false));
    play.advance_with_input(FIXED_DELTA, input())?;
    let mut held = input();
    held.held.set(Action::Interact, true);
    held.events.push(ActionEvent {
        action: Action::Interact,
        pressed: true,
    });
    play.advance_with_input(FIXED_DELTA, held.clone())?;
    assert!(play.take_scene_request().is_none());
    hud(&play, INITIAL);
    held.events.clear();
    held.movement = [0., 1.];
    let mut entered = false;
    for _ in 0..45 {
        play.advance_with_input(FIXED_DELTA, held.clone())?;
        entered |= play.trigger_events().iter().any(|event| {
            event.trigger.to_string() == OWNER && event.other.to_string() == PLAYER && event.entered
        });
    }
    assert!(
        entered,
        "the capsule must physically enter the authored sensor"
    );
    assert!(play.player_character().unwrap().position[2] < 0.);
    hud(&play, PLAYING);
    assert!(animation(&play).time > 0. && animation(&play).playing);
    assert_eq!(visual_scale(&mut play), None);
    assert_eq!(sound(&mut play)?, (1, true));
    assert_eq!(collected(&play), 1);
    assert!(
        play.take_scene_request().is_none(),
        "held E cannot finish the new objective"
    );
    for direction in [-1., 1.] {
        held.movement = [0., direction];
        for _ in 0..35 {
            play.advance_with_input(FIXED_DELTA, held.clone())?;
        }
    }
    assert_eq!(
        collected(&play),
        1,
        "exit and re-entry cannot collect twice"
    );
    assert_eq!(sound(&mut play)?, (1, true));
    held.movement = [0.; 2];
    // A fresh press while the action is running cannot be carried over to its completion.
    held.held.set(Action::Interact, false);
    held.events.push(ActionEvent {
        action: Action::Interact,
        pressed: false,
    });
    play.advance_with_input(FIXED_DELTA, held.clone())?;
    held.held.set(Action::Interact, true);
    held.events = vec![ActionEvent {
        action: Action::Interact,
        pressed: true,
    }];
    play.advance_with_input(FIXED_DELTA, held.clone())?;
    assert!(play.take_scene_request().is_none());
    hud(&play, PLAYING);
    held.held.set(Action::Interact, false);
    held.events = vec![ActionEvent {
        action: Action::Interact,
        pressed: false,
    }];
    play.advance_with_input(FIXED_DELTA, held.clone())?;
    held.events.clear();
    for _ in 0..360 {
        if !animation(&play).playing {
            break;
        }
        play.advance_with_input(FIXED_DELTA, held.clone())?;
        assert!(play.take_scene_request().is_none());
    }
    let finished = animation(&play);
    assert_eq!(finished.duration, 5.5);
    assert_eq!(finished.time, finished.duration);
    assert!(!finished.playing);
    // The script observes the final pose next phase; even that phase's new E is consumed.
    held.held.set(Action::Interact, true);
    held.events = vec![ActionEvent {
        action: Action::Interact,
        pressed: true,
    }];
    play.advance_with_input(FIXED_DELTA, held.clone())?;
    hud(&play, COMPLETE);
    assert!(play.take_scene_request().is_none());
    held.events.clear();
    for _ in 0..3 {
        play.advance_with_input(FIXED_DELTA, held.clone())?;
        assert!(
            play.take_scene_request().is_none(),
            "held E cannot skip the completion prompt"
        );
        assert_eq!(animation(&play).time, finished.duration);
    }
    held.held.set(Action::Interact, false);
    held.events = vec![ActionEvent {
        action: Action::Interact,
        pressed: false,
    }];
    play.advance_with_input(FIXED_DELTA, held.clone())?;
    held.held.set(Action::Interact, true);
    held.events = vec![ActionEvent {
        action: Action::Interact,
        pressed: true,
    }];
    play.advance_with_input(FIXED_DELTA, held)?;
    let request = play
        .take_scene_request()
        .context("fresh E requests the second level")?;
    assert_eq!(
        request.target(),
        "a0000000-0000-4000-8000-000000000003".parse::<AssetId>()?
    );
    let world = play.world_id();
    let second =
        ContentSource::from_json(include_bytes!("../../../demo/assets/travel_second.scene"))?.base;
    let binding = second
        .entities
        .iter()
        .find_map(|entity| entity.script.as_ref())
        .unwrap();
    let candidate = play.prepare_scene(
        second.clone(),
        SceneResources {
            scripts: ScriptLibrary::from([(
                binding.asset,
                include_str!("../../../demo/assets/scripts/travel.rhai").into(),
            )]),
            ..Default::default()
        },
        request.target(),
    )?;
    play.accept_candidate(candidate)?;
    assert_ne!(play.world_id(), world);
    assert!(play.hud_frame().is_none());
    assert!(play.audio_frame()?.is_none());
    assert!(play.animation_state(animator_id()).is_none());
    assert_eq!(
        play.progress_snapshot()
            .get(SCENE.parse()?, OWNER.parse()?, "collected"),
        Some(&ProgressValue::Bool(true))
    );
    assert_eq!(play.snapshot(), second);
    assert_eq!(author.to_json()?, bytes);
    assert_ne!(original_scale, [0.; 3]);
    assert!(play.script_diagnostics().iter().all(|entry| !entry.failed));
    Ok(())
}

#[test]
fn a_non_player_body_cannot_collect_the_target() -> Result<()> {
    let mut author = source()?;
    let center = author
        .entities
        .iter()
        .find(|entity| entity.id.to_string() == OWNER)
        .unwrap()
        .transform
        .translation;
    let mut other = EntityData::new("non-player overlap");
    other.physics = Some(PhysicsBody::default());
    other.transform.translation = center;
    let other_id = other.id;
    author.entities.push(other);
    let mut play = start(&author)?;
    play.advance_with_input(FIXED_DELTA, input())?;
    assert!(play.trigger_events().iter().any(|event| {
        event.other == other_id && event.trigger.to_string() == OWNER && event.entered
    }));
    hud(&play, INITIAL);
    assert_eq!(animation(&play).time, 0.);
    assert!(!animation(&play).playing);
    assert_eq!(sound(&mut play)?, (0, false));
    assert_eq!(collected(&play), 0);
    assert!(visual_scale(&mut play).is_some());
    assert!(play.script_diagnostics().iter().all(|entry| !entry.failed));
    Ok(())
}

#[test]
fn collection_hud_survives_pause_but_restart_and_stop_clear_the_old_world() -> Result<()> {
    let mut author = source()?;
    let center = author
        .entities
        .iter()
        .find(|entity| entity.id.to_string() == OWNER)
        .unwrap()
        .transform
        .translation;
    author
        .entities
        .iter_mut()
        .find(|entity| entity.id.to_string() == PLAYER)
        .unwrap()
        .transform
        .translation = center;
    let bytes = author.to_json()?;
    let author_scale = author
        .entities
        .iter()
        .find(|entity| entity.id.to_string() == VISUAL)
        .unwrap()
        .transform
        .scale;
    let mut play = start(&author)?;
    for _ in 0..7 {
        play.advance_with_input(FIXED_DELTA, input())?;
    }
    hud(&play, PLAYING);
    let state = animation(&play);
    assert!((state.time - 5. / 60.).abs() < 1e-9);
    assert!(state.playing);
    assert_eq!(state.speed, 1.);
    let previous = play.hud_frame().unwrap();
    play.set_paused(true);
    play.advance_with_input(FIXED_DELTA * 4., input())?;
    assert_eq!(animation(&play), state);
    assert!(Arc::ptr_eq(
        &previous.labels,
        &play.hud_frame().unwrap().labels
    ));
    play.single_step()?;
    hud(&play, PLAYING);
    assert!((animation(&play).time - state.time - FIXED_DELTA).abs() < 1e-9);
    assert_eq!(sound(&mut play)?, (1, true));
    let world = play.world_id();
    play.restart()?;
    assert_ne!(play.world_id(), world);
    assert!(play.is_paused());
    hud(&play, INITIAL);
    assert_eq!(animation(&play).time, 0.);
    assert!(!animation(&play).playing);
    assert_eq!(visual_scale(&mut play), Some(author_scale));
    assert_eq!(sound(&mut play)?, (0, false));
    assert!(
        play.progress_snapshot().entries.is_empty(),
        "R restores the entry progress"
    );
    assert_eq!(play.snapshot(), author);
    assert_eq!(author.to_json()?, bytes);
    assert_eq!(previous.labels[0].text.as_ref(), PLAYING);
    play.stop();
    assert!(play.hud_frame().is_none());
    assert!(play.audio_frame()?.is_none());
    assert!(play.animation_state(animator_id()).is_none());
    assert!(play.script_diagnostics().iter().all(|entry| !entry.failed));
    Ok(())
}

#[test]
fn continuing_collected_progress_reenters_at_the_author_start_without_replaying_feedback()
-> Result<()> {
    let author = source()?;
    let bytes = author.to_json()?;
    let mut overlap = author.clone();
    let center = overlap
        .entities
        .iter()
        .find(|entity| entity.id.to_string() == OWNER)
        .unwrap()
        .transform
        .translation;
    overlap
        .entities
        .iter_mut()
        .find(|entity| entity.id.to_string() == PLAYER)
        .unwrap()
        .transform
        .translation = center;
    let mut previous = start(&overlap)?;
    previous.advance_with_input(FIXED_DELTA, input())?;
    hud(&previous, PLAYING);
    let progress = previous.progress_snapshot();
    assert_eq!(
        progress.get(SCENE.parse()?, OWNER.parse()?, "collected"),
        Some(&ProgressValue::Bool(true))
    );
    let mut resumed = PlaySession::start_with_progress(
        author.clone(),
        resources(&author)?,
        SCENE.parse()?,
        progress.clone(),
    )?;
    assert_ne!(resumed.world_id(), previous.world_id());
    assert_eq!(resumed.scene_asset(), Some(SCENE.parse()?));
    assert_eq!(
        resumed.player_character().unwrap().position,
        author
            .entities
            .iter()
            .find(|entity| entity.id.to_string() == PLAYER)
            .unwrap()
            .transform
            .translation
    );
    hud(&resumed, COMPLETE);
    assert_eq!(visual_scale(&mut resumed), None);
    assert_eq!(sound(&mut resumed)?, (0, false));
    assert_eq!(animation(&resumed).time, 0.);
    assert!(!animation(&resumed).playing);
    assert_eq!(collected(&resumed), 0);
    resumed.advance_with_input(FIXED_DELTA, input())?;
    let mut pressed = input();
    pressed.held.set(Action::Interact, true);
    pressed.events.push(ActionEvent {
        action: Action::Interact,
        pressed: true,
    });
    resumed.advance_with_input(FIXED_DELTA, pressed)?;
    assert_eq!(
        resumed
            .take_scene_request()
            .context("continued objective accepts a fresh E")?
            .target(),
        "a0000000-0000-4000-8000-000000000003".parse::<AssetId>()?
    );
    let world = resumed.world_id();
    resumed.set_paused(true);
    resumed.restart()?;
    assert_ne!(resumed.world_id(), world);
    assert!(resumed.is_paused());
    hud(&resumed, COMPLETE);
    assert_eq!(visual_scale(&mut resumed), None);
    assert_eq!(sound(&mut resumed)?, (0, false));
    assert_eq!(resumed.progress_snapshot(), progress);
    assert_eq!(animation(&resumed).time, 0.);
    assert!(!animation(&resumed).playing);
    let mut fresh = start(&author)?;
    hud(&fresh, INITIAL);
    assert!(visual_scale(&mut fresh).is_some());
    assert!(fresh.progress_snapshot().entries.is_empty());
    assert_eq!(sound(&mut fresh)?, (0, false));
    assert_eq!(author.to_json()?, bytes);
    assert!(
        resumed
            .script_diagnostics()
            .iter()
            .all(|entry| !entry.failed)
    );
    Ok(())
}
