use super::character::copy_project;
use super::*;
use comet_core::{Action, ActionEvent, ActionInput, math::Quat};
use comet_scene::PoseUpdate;
use std::{fs, path::Path};

struct Arena {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    actors: [EntityId; 3],
}
fn input() -> ActionInput {
    ActionInput {
        enabled: true,
        ..Default::default()
    }
}
impl Arena {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        copy_project(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../demo"),
            &project,
        )?;
        let (mut editor, mut assets) =
            open(project, temp.path().join("products"), "combat.scene".into())?;
        let source = editor.document.shared_source();
        let actors = ["练习者", "守卫 1", "守卫 2"].map(|name| {
            source
                .base
                .entities
                .iter()
                .find(|e| e.name == name)
                .unwrap()
                .id
        });
        editor.start(&mut assets)?;
        let mut value = Self {
            _temp: temp,
            editor,
            assets,
            actors,
        };
        value.step(25, input())?;
        Ok(value)
    }
    fn step(&mut self, count: usize, mut input: ActionInput) -> Result<()> {
        for _ in 0..count {
            let play = self.editor.play.as_mut().unwrap();
            play.advance_with_input(comet_runtime::FIXED_DELTA, input.clone())?;
            input.events.clear();
            ensure!(
                play.script_diagnostics().iter().all(|d| !d.failed),
                "{:?}",
                play.script_diagnostics()
            );
        }
        Ok(())
    }
    fn position(&self, index: usize) -> [f32; 3] {
        self.editor
            .play
            .as_ref()
            .unwrap()
            .character(self.actors[index])
            .unwrap()
            .position
    }
    fn place(&mut self, index: usize, position: [f32; 3], yaw: f32) -> Result<()> {
        self.editor.play.as_mut().unwrap().teleport(PoseUpdate {
            entity: self.actors[index],
            translation: position,
            rotation: Quat::from_rotation_y(yaw).to_array(),
        })
    }
    fn hud(&self) -> String {
        self.editor
            .play
            .as_ref()
            .unwrap()
            .hud_frame()
            .unwrap()
            .labels[0]
            .text
            .to_string()
    }
    fn attacks(&self) -> usize {
        self.editor
            .play
            .as_ref()
            .unwrap()
            .script_diagnostics()
            .iter()
            .filter(|d| d.message == "战斗攻击：练习者")
            .count()
    }
    fn press(&mut self, count: usize) -> Result<()> {
        self.step(1, input())?;
        let mut press = input();
        press.held.set(Action::Primary, true);
        press.events.push(ActionEvent {
            action: Action::Primary,
            pressed: true,
        });
        self.step(count, press)
    }
}

#[test]
fn real_combat_events_lock_input_filter_facing_and_walls_then_win_restart_and_restore_authoring()
-> Result<()> {
    let mut a = Arena::new()?;
    let source = a.editor.document.shared_source();
    let bytes = fs::read(&a.editor.source)?;
    let history = a.editor.document.history_len();
    a.place(0, [4., 0.82, 6.], 0.)?;
    a.place(1, [4., 0.82, 4.7], 0.)?;
    a.place(2, [8., 0.82, -8.], 0.)?;
    a.press(12)?;
    assert!(
        a.hud().contains("守卫 1：90"),
        "damage waits for the actual strike marker: {}",
        a.hud()
    );
    a.editor.play.as_mut().unwrap().set_paused(true);
    let paused = a.editor.play.as_mut().unwrap().shared_snapshot();
    let hud = a.hud();
    a.step(30, input())?;
    assert!(std::sync::Arc::ptr_eq(
        &paused,
        &a.editor.play.as_mut().unwrap().shared_snapshot()
    ));
    assert_eq!(a.hud(), hud);
    a.editor.play.as_mut().unwrap().single_step()?;
    assert!(
        a.hud().contains("守卫 1：90"),
        "one step does not skip the strike phase"
    );
    a.editor.play.as_mut().unwrap().set_paused(false);
    let before = a.position(0);
    let mut held = input();
    held.held.set(Action::Primary, true);
    held.movement = [1., 0.];
    a.step(1, held.clone())?;
    assert!(
        (a.position(0)[0] - before[0]).abs() < 0.001,
        "attack suppresses movement"
    );
    held.movement = [0.; 2];
    a.step(105, held)?;
    assert_eq!(a.attacks(), 1, "holding never repeats an attack");
    assert!(
        a.hud().contains("守卫 1：55"),
        "one strike deals exactly 35: {}",
        a.hud()
    );

    a.place(0, [4., 0.82, 6.], 0.)?;
    a.place(1, [4., 0.82, 7.3], 0.)?;
    a.press(55)?;
    assert!(
        a.hud().contains("守卫 1：55"),
        "targets behind the visual are missed: {}",
        a.hud()
    );
    a.place(0, [-0.67, 0.82, 0.], -std::f32::consts::FRAC_PI_2)?;
    a.place(1, [0.67, 0.82, 0.], 0.)?;
    a.press(55)?;
    assert!(
        a.hud().contains("守卫 1：55"),
        "a wall blocks an in-range facing attack: {}",
        a.hud()
    );

    a.place(0, [4., 0.82, 6.], 0.)?;
    a.place(1, [4., 0.82, 4.7], 0.)?;
    a.place(2, [4.6, 0.82, 4.7], 0.)?;
    for _ in 0..3 {
        a.press(55)?;
    }
    assert!(a.hud().contains("胜利！"), "{}", a.hud());
    let p = a.position(0);
    let mut movement = input();
    movement.movement = [1., 0.];
    a.step(20, movement)?;
    assert!(
        (a.position(0)[0] - p[0]).abs() < 0.001,
        "victory stops control"
    );
    let world = a.editor.play.as_ref().unwrap().world_id();
    let mut restart = input();
    restart.events.push(ActionEvent {
        action: Action::Restart,
        pressed: true,
    });
    a.step(1, restart)?;
    assert_ne!(world, a.editor.play.as_ref().unwrap().world_id());
    assert!(a.hud().contains("练习者：120") && a.hud().contains("击败：0 / 2"));
    a.editor.stop();
    assert_eq!(a.editor.document.shared_source(), source);
    assert_eq!(a.editor.document.history_len(), history);
    assert_eq!(fs::read(&a.editor.source)?, bytes);
    // Reopening the authored source must not retain runtime health, intent or pose.
    a.editor.start(&mut a.assets)?;
    assert!(a.hud().contains("守卫 1：90"));
    Ok(())
}

#[test]
fn real_guards_route_around_the_wall_deal_event_damage_and_freeze_after_player_defeat() -> Result<()>
{
    let mut a = Arena::new()?;
    a.place(0, [0., 0.82, 4.], 0.)?;
    a.place(1, [0., 0.82, -4.], 0.)?;
    a.place(2, [7., 0.82, -7.], 0.)?;
    let mut detour = 0_f32;
    for _ in 0..300 {
        a.step(5, input())?;
        detour = detour.max(a.position(1)[0].abs());
        if a.hud().contains("失败！") {
            break;
        }
    }
    assert!(
        detour > 0.8,
        "the real navigation route bypasses the blocking wall"
    );
    assert!(
        a.hud().contains("失败！"),
        "guards eventually defeat the idle player: {}",
        a.hud()
    );
    let positions = [a.position(0), a.position(1), a.position(2)];
    a.step(30, input())?;
    for (i, p) in positions.iter().enumerate() {
        assert!((a.position(i)[0] - p[0]).abs() < 0.01 && (a.position(i)[2] - p[2]).abs() < 0.01);
    }
    Ok(())
}
