use anyhow::{Context, Result};
use comet_core::{Action, ActionEvent, ActionInput, EntityId};
use comet_runtime::{FIXED_DELTA, PlaySession, ScriptLibrary};
use comet_scene::{ContentSource, EntityRef, SceneSnapshot, ScriptValue};

fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}

fn without_scripts(mut snapshot: SceneSnapshot) -> SceneSnapshot {
    for entity in &mut snapshot.entities {
        entity.script = None;
    }
    snapshot
}

fn y(play: &mut PlaySession, entity: EntityId) -> f32 {
    play.snapshot()
        .entities
        .iter()
        .find(|item| item.id == entity)
        .unwrap()
        .transform
        .translation[1]
}

#[test]
fn actual_demo_script_is_idle_without_input_applies_one_interact_impulse_and_logs_its_lifecycle()
-> Result<()> {
    // The author base contains every physical target; package verification covers visual Prefabs.
    let author = ContentSource::from_json(include_bytes!("../../../demo/assets/main.scene"))?.base;
    let (owner, binding) = author
        .entities
        .iter()
        .find_map(|entity| entity.script.as_ref().map(|binding| (entity.id, binding)))
        .context("demo script binding")?;
    let ScriptValue::Entity(Some(EntityRef::Local { entity: target })) = binding
        .parameters
        .get("target")
        .context("demo target parameter")?
    else {
        anyhow::bail!("demo target must be a local entity")
    };
    let target = *target;
    let asset = binding.asset;
    let library = ScriptLibrary::from([(
        asset,
        include_str!("../../../demo/assets/scripts/falling_cube.rhai").to_owned(),
    )]);
    let mut scripted = PlaySession::start_with_scripts(author.clone(), library)?;
    let mut control = PlaySession::start(without_scripts(author.clone()))?;
    assert_eq!(scripted.script_diagnostics().len(), 1);
    for _ in 0..90 {
        scripted.advance_with_input(FIXED_DELTA, input())?;
        control.advance_with_input(FIXED_DELTA, input())?;
    }
    assert_eq!(without_scripts(scripted.snapshot()), control.snapshot());
    let mut pressed = input();
    pressed.held.set(Action::Interact, true);
    pressed.events.push(ActionEvent {
        action: Action::Interact,
        pressed: true,
    });
    scripted.advance_with_input(FIXED_DELTA, pressed.clone())?;
    control.advance_with_input(FIXED_DELTA, pressed.clone())?;
    let first = y(&mut scripted, target);
    assert!(
        first > y(&mut control, target) + 0.05,
        "interact must actually move the dynamic target upward"
    );
    pressed.events.clear();
    scripted.advance_with_input(FIXED_DELTA, pressed.clone())?;
    let second = y(&mut scripted, target);
    assert!(
        (0.05..0.2).contains(&(second - first)),
        "held input must not apply another impulse every tick"
    );
    scripted.advance_with_input(FIXED_DELTA * 3., pressed)?;
    assert_eq!(
        scripted
            .script_diagnostics()
            .iter()
            .filter(|entry| entry.message == "已向落体方块施加向上冲量。")
            .count(),
        1
    );
    let before_stop = scripted.snapshot();
    scripted.stop();
    scripted.stop();
    assert_eq!(scripted.snapshot(), before_stop);
    let diagnostics = scripted.script_diagnostics();
    assert_eq!(diagnostics.len(), 3);
    assert_eq!(
        diagnostics[0].message,
        "落体方块示例已启动：按 E 弹起方块。"
    );
    assert_eq!(diagnostics[0].phase, "on_start");
    assert_eq!(diagnostics[2].message, "落体方块示例已停止。");
    assert_eq!(diagnostics[2].phase, "on_stop");
    assert!(diagnostics.iter().all(|entry| !entry.failed
        && entry.asset == asset
        && entry.entity == owner
        && entry.world == scripted.world_id()));
    assert_eq!(
        author
            .entities
            .iter()
            .find(|entity| entity.id == target)
            .unwrap()
            .transform
            .translation,
        [-2.5, 3., 1.]
    );
    Ok(())
}
