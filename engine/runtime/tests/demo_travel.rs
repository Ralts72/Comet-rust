use anyhow::{Context, Result, bail};
use comet_core::{Action, ActionEvent, ActionInput, AssetId};
use comet_runtime::{FIXED_DELTA, PlaySession, ScriptLibrary};
use comet_scene::{ContentSource, PrefabResolver, SceneSnapshot, ScriptValue};

fn scene(bytes: &[u8]) -> Result<SceneSnapshot> {
    let source = ContentSource::from_json(bytes)?;
    assert!(source.instances.is_empty());
    struct NoPrefabs;
    impl PrefabResolver for NoPrefabs {
        fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
            bail!("the travel demo must reuse mesh products without a Prefab dependency")
        }
    }
    source.expand(&mut NoPrefabs)
}

fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}

fn press(action: Action) -> ActionInput {
    let mut input = input();
    input.held.set(action, true);
    input.events.push(ActionEvent {
        action,
        pressed: true,
    });
    input
}

#[test]
fn actual_travel_scenes_require_fresh_input_and_restart_the_current_entry_with_world_scoped_logs()
-> Result<()> {
    // Runtime consumes ready scenes and script text. Product loading, render validation,
    // and the packaged scene allowlist belong to the App/Editor host regressions.
    let first = scene(include_bytes!("../../../demo/assets/travel.scene"))?;
    let second = scene(include_bytes!("../../../demo/assets/travel_second.scene"))?;
    let first_id: AssetId = "a0000000-0000-4000-8000-000000000002".parse()?;
    let second_id: AssetId = "a0000000-0000-4000-8000-000000000003".parse()?;
    let binding = |scene: &SceneSnapshot| {
        let owner = scene
            .entities
            .iter()
            .find(|entity| entity.script.is_some())
            .unwrap();
        (owner.id, owner.script.as_ref().unwrap().clone())
    };
    let (first_owner, first_script) = binding(&first);
    let (second_owner, second_script) = binding(&second);
    assert_eq!(first_script.asset, second_script.asset);
    assert_eq!(
        first_script.parameters["next_scene"],
        ScriptValue::Text(second_id.to_string())
    );
    assert_eq!(
        second_script.parameters["next_scene"],
        ScriptValue::Text(first_id.to_string())
    );
    for script in [&first_script, &second_script] {
        assert_eq!(script.parameters["auto_after"], ScriptValue::Number(0.));
    }
    let ScriptValue::Text(first_label) = &first_script.parameters["label"] else {
        bail!("first scene label must be text")
    };
    let ScriptValue::Text(second_label) = &second_script.parameters["label"] else {
        bail!("second scene label must be text")
    };
    assert_ne!(first_label, second_label);
    let sources = ScriptLibrary::from([(
        first_script.asset,
        include_str!("../../../demo/assets/scripts/travel.rhai").to_owned(),
    )]);
    let mut play = PlaySession::start_with_scripts(first.clone(), sources.clone())?;
    let first_world = play.world_id();
    for _ in 0..6 {
        play.advance_with_input(FIXED_DELTA, input())?;
        assert!(
            play.take_scene_request().is_none(),
            "the default demo must not auto-travel"
        );
    }
    let pressed = press(Action::Interact);
    play.advance_with_input(FIXED_DELTA, pressed.clone())?;
    let request = play
        .take_scene_request()
        .context("E should request the second scene")?;
    assert_eq!(request.target(), second_id);
    assert!(play.take_scene_request().is_none());
    // A second physical press before the host accepts must not queue duplicate travel.
    let mut released = input();
    released.events.push(ActionEvent {
        action: Action::Interact,
        pressed: false,
    });
    play.advance_with_input(FIXED_DELTA, released.clone())?;
    play.advance_with_input(FIXED_DELTA, pressed.clone())?;
    assert!(play.take_scene_request().is_none());
    let first_stop_tick = play.steps();

    // The host resolves request.target() to ready content, initializes the whole
    // candidate, then accepts it outside the running world's callbacks.
    let candidate = PlaySession::start_with_scripts(second.clone(), sources)?;
    let second_world = candidate.world_id();
    play.accept_candidate(candidate)?;
    assert_ne!(first_world, second_world);
    assert_eq!(play.world_id(), second_world);
    assert_eq!(play.snapshot(), second);
    assert_eq!(play.steps(), 0);
    play.advance_with_input(FIXED_DELTA, pressed.clone())?;
    assert_eq!(
        play.steps(),
        0,
        "candidate preparation time must be discarded"
    );
    let mut held = pressed.clone();
    held.events.clear();
    for _ in 0..6 {
        play.advance_with_input(FIXED_DELTA, held.clone())?;
        assert!(
            play.take_scene_request().is_none(),
            "the previous world's E edge must not replay"
        );
    }
    play.advance_with_input(FIXED_DELTA, released.clone())?;
    play.advance_with_input(FIXED_DELTA, pressed)?;
    let pending = play
        .take_scene_request()
        .context("a new E press should request the first scene")?;
    assert_eq!(pending.target(), first_id);
    let second_stop_tick = play.steps();

    // R restarts B while its request to A is still awaiting the host; it does not
    // restore the editor's original A world or carry the pending request forward.
    let mut restart = press(Action::Restart);
    restart.held.set(Action::Interact, true);
    play.advance_with_input(FIXED_DELTA, restart.clone())?;
    let restarted_world = play.world_id();
    assert_ne!(restarted_world, second_world);
    assert_eq!(play.snapshot(), second);
    assert_eq!(play.steps(), 0);
    assert!(play.take_scene_request().is_none());
    let diagnostics = play.script_diagnostics().len();
    assert!(!play.reject_scene_request(pending, "late host completion after R"));
    assert_eq!(play.script_diagnostics().len(), diagnostics);
    play.advance_with_input(FIXED_DELTA, restart.clone())?;
    play.advance_with_input(FIXED_DELTA, restart)?;
    assert_eq!(
        play.world_id(),
        restarted_world,
        "holding R must not repeatedly restart"
    );
    assert_eq!(play.steps(), 1);
    assert!(play.take_scene_request().is_none());
    released.events.push(ActionEvent {
        action: Action::Restart,
        pressed: false,
    });
    play.advance_with_input(FIXED_DELTA, released)?;
    play.advance_with_input(FIXED_DELTA, press(Action::Interact))?;
    assert_eq!(
        play.take_scene_request()
            .context("restart must reset the script's requested flag")?
            .target(),
        first_id
    );
    let final_tick = play.steps();
    play.stop();
    let count = play.script_diagnostics().len();
    play.stop();
    assert_eq!(play.script_diagnostics().len(), count);
    let first_start = format!("进入关卡：{first_label}。按 E 前往另一关。");
    let first_stop = format!("离开关卡：{first_label}。");
    let second_start = format!("进入关卡：{second_label}。按 E 前往另一关。");
    let second_stop = format!("离开关卡：{second_label}。");
    let expected = [
        (first_world, first_owner, "on_start", first_start.as_str()),
        (
            first_world,
            first_owner,
            "on_fixed_update",
            "已请求前往另一关。",
        ),
        (first_world, first_owner, "on_stop", first_stop.as_str()),
        (
            second_world,
            second_owner,
            "on_start",
            second_start.as_str(),
        ),
        (
            second_world,
            second_owner,
            "on_fixed_update",
            "已请求前往另一关。",
        ),
        (second_world, second_owner, "on_stop", second_stop.as_str()),
        (
            restarted_world,
            second_owner,
            "on_start",
            second_start.as_str(),
        ),
        (
            restarted_world,
            second_owner,
            "on_fixed_update",
            "已请求前往另一关。",
        ),
        (
            restarted_world,
            second_owner,
            "on_stop",
            second_stop.as_str(),
        ),
    ];
    assert_eq!(play.script_diagnostics().len(), expected.len());
    for (index, (entry, &(world, owner, phase, message))) in play
        .script_diagnostics()
        .iter()
        .zip(expected.iter())
        .enumerate()
    {
        assert_eq!(entry.sequence, index as u64 + 1);
        assert!(!entry.failed, "{}", entry.message);
        assert_eq!(entry.world, world);
        assert_eq!(entry.entity, owner);
        assert_eq!(entry.asset, first_script.asset);
        assert_eq!(entry.phase, phase);
        assert_eq!(entry.message, message);
    }
    assert_eq!(play.script_diagnostics()[2].tick, first_stop_tick);
    assert_eq!(play.script_diagnostics()[5].tick, second_stop_tick);
    assert_eq!(play.script_diagnostics()[8].tick, final_tick);
    assert_eq!(
        first,
        scene(include_bytes!("../../../demo/assets/travel.scene"))?
    );
    assert_eq!(
        second,
        scene(include_bytes!("../../../demo/assets/travel_second.scene"))?
    );
    Ok(())
}
