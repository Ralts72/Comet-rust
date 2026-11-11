use super::*;
use comet_core::{AssetId, EntityId};

fn diagnostic(world: WorldId, sequence: u64) -> ScriptDiagnostic {
    ScriptDiagnostic {
        world,
        sequence,
        entity: EntityId::new(),
        asset: AssetId::new(),
        tick: sequence,
        phase: "on_update",
        message: "message".into(),
        failed: false,
    }
}

#[test]
fn logs_are_bounded_and_do_not_replay_after_collection_or_clearing() {
    let world = WorldId::new();
    let mut logs: VecDeque<_> = (0..256)
        .map(|sequence| diagnostic(world, sequence))
        .collect();
    let mut console = ScriptConsole::default();
    console.collect(&logs);
    assert_eq!(console.entries.len(), CAPACITY);
    assert_eq!(console.entries.front().unwrap().sequence, 128);
    console.collect(&logs);
    assert_eq!(console.entries.len(), CAPACITY);
    assert_eq!(console.entries.front().unwrap().sequence, 128);
    console.clear();
    console.collect(&logs);
    assert!(console.entries.is_empty());
    logs.pop_front();
    logs.push_back(diagnostic(world, 256));
    console.collect(&logs);
    assert_eq!(console.entries.len(), 1);
    assert_eq!(console.entries[0].sequence, 256);
    assert_eq!(console.seen.len(), logs.len());
}

#[test]
fn merged_restart_worlds_keep_their_identity_and_reset_clears_the_project_scope() {
    let first = WorldId::new();
    let second = WorldId::new();
    let mut logs = VecDeque::from([diagnostic(first, 1)]);
    let mut console = ScriptConsole::default();
    console.collect(&logs);
    let mut failed = diagnostic(second, 1);
    failed.failed = true;
    failed.message = "文".repeat(MESSAGE_BYTES);
    logs.push_back(failed);
    console.collect(&logs);
    console.collect(&logs);
    assert_eq!(console.entries.len(), 2);
    assert_eq!(console.entries[0].world, first);
    assert_eq!(console.entries[1].world, second);
    assert!(console.entries[1].message.len() <= MESSAGE_BYTES);
    assert!(!console.entries[1].message.is_empty());
    assert_eq!(console.report()[1]["failed"], true);
    console.reset();
    assert!(console.entries.is_empty() && console.seen.is_empty());
}
