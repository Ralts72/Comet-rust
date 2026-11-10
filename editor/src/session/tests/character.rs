use super::*;
use comet_application::CheckpointAction;
use comet_core::{Action, ActionEvent, ActionInput, ProgressValue};
use comet_scene::PoseUpdate;
use std::{fs, path::Path, sync::Arc};

fn copy_project(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() == ".assetc.lock" {
            continue;
        }
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_project(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

struct Level {
    asset: AssetId,
    player: EntityId,
    gate: EntityId,
    animator: EntityId,
    visual: EntityId,
    targets: Vec<(EntityId, [f32; 3])>,
}
impl Level {
    fn load(editor: &Editor, assets: &mut AssetStore, name: &str) -> Result<Self> {
        let asset = comet_asset_pipeline::SourceMeta::load(&editor.paths.assets().join(name))?.id;
        let snapshot = assets.expand_scene(asset)?;
        let player = snapshot
            .entities
            .iter()
            .find(|e| e.character.as_ref().is_some_and(|c| c.player_controlled))
            .unwrap()
            .id;
        let gate = snapshot
            .entities
            .iter()
            .find(|e| e.name.starts_with("出口屏障"))
            .unwrap()
            .id;
        let mut targets: Vec<_> = snapshot
            .entities
            .iter()
            .filter(|e| e.name.starts_with("收集目标 "))
            .collect();
        targets.sort_by_key(|e| &e.name);
        ensure!(targets.len() == 3, "three authored targets");
        let visual = snapshot
            .entities
            .iter()
            .find(|e| e.character_animation.is_some())
            .unwrap();
        let Some(comet_scene::EntityRef::Local { entity: animator }) =
            visual.character_animation.as_ref().unwrap().animator
        else {
            panic!("expanded character animator")
        };
        Ok(Self {
            asset,
            player,
            gate,
            animator,
            visual: visual.id,
            targets: targets
                .into_iter()
                .map(|e| (e.id, e.transform.translation))
                .collect(),
        })
    }
    fn collected(&self, editor: &Editor, index: usize) -> bool {
        editor.play.as_ref().unwrap().progress_snapshot().get(
            self.asset,
            self.targets[index].0,
            "collected",
        ) == Some(&ProgressValue::Bool(true))
    }
    fn move_to(&self, editor: &mut Editor, point: [f32; 3]) -> Result<()> {
        editor.play.as_mut().unwrap().teleport(PoseUpdate {
            entity: self.player,
            translation: point,
            rotation: [0., 0., 0., 1.],
        })
    }
    fn gate_open(&self, editor: &mut Editor) -> bool {
        editor
            .play
            .as_mut()
            .unwrap()
            .shared_snapshot()
            .entities
            .iter()
            .find(|e| e.id == self.gate)
            .unwrap()
            .transform
            .translation[1]
            > 4.
    }
}
fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}
fn pressed() -> ActionInput {
    let mut input = input();
    input.held.set(Action::Interact, true);
    input.events.push(ActionEvent {
        action: Action::Interact,
        pressed: true,
    });
    input
}
fn step(
    editor: &mut Editor,
    assets: &mut AssetStore,
    count: usize,
    input: ActionInput,
) -> Result<()> {
    for index in 0..count {
        let mut input = input.clone();
        if index > 0 {
            input.events.clear();
        }
        editor
            .play
            .as_mut()
            .unwrap()
            .advance_with_input(comet_runtime::FIXED_DELTA, input)?;
        editor.process_scene_request(assets);
        ensure!(
            editor
                .play
                .as_ref()
                .unwrap()
                .script_diagnostics()
                .iter()
                .all(|d| !d.failed),
            "{:?}",
            editor.play.as_ref().unwrap().script_diagnostics()
        );
    }
    Ok(())
}

#[test]
fn real_character_levels_collect_six_targets_gate_on_animation_events_and_restore_both_scene_scopes()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    copy_project(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../demo"),
        &project,
    )?;
    let (mut editor, mut assets) = open(
        project,
        temp.path().join("products"),
        "character.scene".into(),
    )?;
    let first = Level::load(&editor, &mut assets, "character.scene")?;
    let second = Level::load(&editor, &mut assets, "character_second.scene")?;
    let author = editor.document.shared_source();
    let bytes = fs::read(&editor.source)?;
    let history = editor.document.history_len();
    editor.start(&mut assets)?;
    step(&mut editor, &mut assets, 2, input())?;
    assert!(
        first
            .targets
            .iter()
            .enumerate()
            .all(|(i, _)| !first.collected(&editor, i))
    );
    step(&mut editor, &mut assets, 30, input())?;
    assert_eq!(
        editor
            .play
            .as_ref()
            .unwrap()
            .animation_state(first.animator)
            .unwrap()
            .state
            .as_deref(),
        Some("ground")
    );
    let mut locomotion = input();
    locomotion.movement = [1., 0.];
    step(&mut editor, &mut assets, 10, locomotion.clone())?;
    let state = editor
        .play
        .as_ref()
        .unwrap()
        .animation_state(first.animator)
        .unwrap();
    assert!(
        matches!(state.parameters["speed"],comet_asset_data::AnimationParameter::Number(speed) if (speed-4.).abs()<0.01)
    );
    locomotion.held.set(Action::Run, true);
    step(&mut editor, &mut assets, 12, locomotion)?;
    let state = editor
        .play
        .as_ref()
        .unwrap()
        .animation_state(first.animator)
        .unwrap();
    assert!(
        matches!(state.parameters["speed"],comet_asset_data::AnimationParameter::Number(speed) if (speed-7.).abs()<0.01)
    );
    let mut crouch = input();
    crouch.held.set(Action::Crouch, true);
    step(&mut editor, &mut assets, 3, crouch)?;
    assert_eq!(
        editor
            .play
            .as_ref()
            .unwrap()
            .animation_state(first.animator)
            .unwrap()
            .state
            .as_deref(),
        Some("crouch")
    );
    let visual_y = editor
        .play
        .as_mut()
        .unwrap()
        .shared_snapshot()
        .entities
        .iter()
        .find(|e| e.id == first.visual)
        .unwrap()
        .transform
        .translation[1];
    assert!(
        (visual_y + 0.48).abs() < 1e-5,
        "crouching keeps the visual at the capsule's feet"
    );
    let mut jump = input();
    jump.held.set(Action::Jump, true);
    jump.events.push(ActionEvent {
        action: Action::Jump,
        pressed: true,
    });
    step(&mut editor, &mut assets, 1, jump)?;
    assert_eq!(
        editor
            .play
            .as_ref()
            .unwrap()
            .animation_state(first.animator)
            .unwrap()
            .state
            .as_deref(),
        Some("jump")
    );
    step(&mut editor, &mut assets, 40, input())?;
    assert_eq!(
        editor
            .play
            .as_ref()
            .unwrap()
            .animation_state(first.animator)
            .unwrap()
            .state
            .as_deref(),
        Some("fall")
    );
    step(&mut editor, &mut assets, 40, input())?;
    assert_eq!(
        editor
            .play
            .as_ref()
            .unwrap()
            .animation_state(first.animator)
            .unwrap()
            .state
            .as_deref(),
        Some("ground")
    );
    assert!(
        editor
            .play
            .as_mut()
            .unwrap()
            .audio_frame()?
            .unwrap()
            .voices
            .iter()
            .any(|v| v.entity == first.visual && v.generation > 0),
        "locomotion events start the spatial footstep voice"
    );
    first.move_to(&mut editor, [0., 1., -4.8])?;
    let mut walking = input();
    walking.movement = [0., 1.];
    step(&mut editor, &mut assets, 30, walking.clone())?;
    assert!(
        matches!(editor.play.as_ref().unwrap().animation_state(first.animator).unwrap().parameters["speed"],comet_asset_data::AnimationParameter::Number(speed) if speed<0.1),
        "blocked movement uses actual collision feedback"
    );
    assert!(
        editor
            .play
            .as_ref()
            .unwrap()
            .player_character()
            .unwrap()
            .position[2]
            > -5.5,
        "closed gate blocks the real character motor"
    );
    step(&mut editor, &mut assets, 1, pressed())?;
    assert_eq!(
        editor.play.as_ref().unwrap().scene_asset(),
        Some(first.asset)
    );
    first.move_to(&mut editor, first.targets[0].1)?;
    step(&mut editor, &mut assets, 2, input())?;
    assert!(first.collected(&editor, 0));
    assert!(!first.collected(&editor, 1));
    editor.preview_checkpoint_action(CheckpointAction::Save, &mut assets)?;
    let slot = editor.preview_checkpoint.as_ref().unwrap().to_bytes()?;
    first.move_to(&mut editor, first.targets[1].1)?;
    step(&mut editor, &mut assets, 2, input())?;
    assert!(first.collected(&editor, 1));
    editor.preview_checkpoint = Some(comet_core::Checkpoint::from_bytes(&slot)?);
    editor.preview_checkpoint_action(CheckpointAction::Continue, &mut assets)?;
    step(&mut editor, &mut assets, 2, input())?;
    assert!(first.collected(&editor, 0));
    assert!(!first.collected(&editor, 1));
    assert!(
        editor
            .play
            .as_mut()
            .unwrap()
            .audio_frame()?
            .unwrap()
            .voices
            .iter()
            .all(|v| v.generation == 0)
    );
    assert!(!first.gate_open(&mut editor));
    for index in [1, 2] {
        first.move_to(&mut editor, first.targets[index].1)?;
        step(&mut editor, &mut assets, 2, input())?;
        assert!(first.collected(&editor, index));
    }
    // The last target is collected, but its action has not delivered the completion marker yet.
    first.move_to(&mut editor, [0., 1., -4.9])?;
    assert!(!first.gate_open(&mut editor));
    editor.play.as_mut().unwrap().set_paused(true);
    let states = editor.play.as_mut().unwrap().shared_snapshot();
    step(&mut editor, &mut assets, 10, pressed())?;
    assert!(Arc::ptr_eq(
        &states,
        &editor.play.as_mut().unwrap().shared_snapshot()
    ));
    editor.play.as_mut().unwrap().single_step()?;
    editor.play.as_mut().unwrap().set_paused(false);
    let mut held = pressed();
    held.events.clear();
    step(&mut editor, &mut assets, 340, held.clone())?;
    assert!(first.gate_open(&mut editor));
    step(&mut editor, &mut assets, 20, walking)?;
    assert!(
        editor
            .play
            .as_ref()
            .unwrap()
            .player_character()
            .unwrap()
            .position[2]
            < -5.8,
        "opened gate releases the collider too"
    );
    assert_eq!(
        editor.play.as_ref().unwrap().scene_asset(),
        Some(first.asset),
        "holding E cannot skip the newly opened exit"
    );
    step(&mut editor, &mut assets, 1, input())?;
    step(&mut editor, &mut assets, 1, pressed())?;
    assert_eq!(
        editor.play.as_ref().unwrap().scene_asset(),
        Some(second.asset)
    );
    step(&mut editor, &mut assets, 2, input())?;
    assert!(
        first
            .targets
            .iter()
            .enumerate()
            .all(|(i, _)| first.collected(&editor, i))
    );
    for index in 0..3 {
        second.move_to(&mut editor, second.targets[index].1)?;
        step(&mut editor, &mut assets, 2, input())?;
        assert!(second.collected(&editor, index));
        if index == 0 {
            editor.preview_checkpoint_action(CheckpointAction::Save, &mut assets)?;
        }
    }
    // Continue re-enters the saved second level, preserving first-level progress and rolling back
    // only the second-level targets collected after that save.
    editor.preview_checkpoint_action(CheckpointAction::Continue, &mut assets)?;
    step(&mut editor, &mut assets, 2, input())?;
    assert_eq!(
        editor.play.as_ref().unwrap().scene_asset(),
        Some(second.asset)
    );
    assert!(
        first
            .targets
            .iter()
            .enumerate()
            .all(|(i, _)| first.collected(&editor, i))
    );
    assert!(second.collected(&editor, 0));
    assert!(!second.collected(&editor, 1));
    for index in [1, 2] {
        second.move_to(&mut editor, second.targets[index].1)?;
        step(&mut editor, &mut assets, 2, input())?;
    }
    second.move_to(&mut editor, [0., 1., -4.9])?;
    step(&mut editor, &mut assets, 340, input())?;
    assert!(second.gate_open(&mut editor));
    step(&mut editor, &mut assets, 1, pressed())?;
    assert_eq!(
        editor.play.as_ref().unwrap().scene_asset(),
        Some(first.asset)
    );
    step(&mut editor, &mut assets, 2, input())?;
    assert!(first.gate_open(&mut editor));
    assert!(
        editor
            .play
            .as_mut()
            .unwrap()
            .audio_frame()?
            .unwrap()
            .voices
            .iter()
            .all(|v| v.generation == 0),
        "returning does not replay collected feedback"
    );
    editor.restart_play()?;
    step(&mut editor, &mut assets, 2, input())?;
    assert!(
        first
            .targets
            .iter()
            .enumerate()
            .all(|(i, _)| first.collected(&editor, i)),
        "restart retains the current scene's entry progress"
    );
    editor.stop();
    assert!(editor.preview_checkpoint.is_none());
    assert!(Arc::ptr_eq(&author, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), history);
    assert_eq!(fs::read(&editor.source)?, bytes);
    editor.start(&mut assets)?;
    step(&mut editor, &mut assets, 2, input())?;
    assert!(
        first
            .targets
            .iter()
            .enumerate()
            .all(|(i, _)| !first.collected(&editor, i))
    );
    assert!(!first.gate_open(&mut editor));
    Ok(())
}
