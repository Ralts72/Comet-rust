use super::*;
use crate::selection::Selection;
use comet_render::{PickFeedback, ViewToken};

fn feedback(frame: u64, world: WorldId, generation: u64, hit: Option<EntityId>) -> FrameFeedback {
    let token = ViewToken {
        world,
        view: 1,
        target_generation: generation,
    };
    FrameFeedback {
        number: frame,
        token,
        presented: true,
        backend: "test".into(),
        worker_thread: "test".into(),
        draw_count: 0,
        gpu_ms: 0.,
        pick: Some(PickFeedback {
            token,
            entities: hit.into_iter().collect(),
        }),
    }
}

#[test]
fn repeated_direct_selection_rejects_older_click_and_clear_consumes_pending_intent() {
    let world = WorldId::new();
    let a = EntityId::new();
    let b = EntityId::new();
    let mut selection = Selection::default();
    let mut picking = Picking::default();
    selection.choose(Some(a), SelectMode::Replace);
    picking.request(1, world, 3, selection.generation(), SelectMode::Replace);
    selection.choose(Some(a), SelectMode::Replace);
    assert!(
        picking
            .resolve(
                &feedback(1, world, 3, Some(b)),
                world,
                3,
                selection.generation()
            )
            .is_none()
    );
    assert_eq!(selection.active(), Some(a));
    picking.request(2, world, 3, selection.generation(), SelectMode::Toggle);
    picking.clear();
    assert!(
        picking
            .resolve(
                &feedback(2, world, 3, Some(b)),
                world,
                3,
                selection.generation()
            )
            .is_none()
    );
}

#[test]
fn captured_modes_accumulate_ordered_toggles_and_each_feedback_is_consumed_once() {
    let world = WorldId::new();
    let a = EntityId::new();
    let b = EntityId::new();
    let mut selection = Selection::default();
    let mut picking = Picking::default();
    for (frame, mode) in [
        (1, SelectMode::Replace),
        (2, SelectMode::Toggle),
        (3, SelectMode::Toggle),
    ] {
        picking.request(frame, world, 4, selection.generation(), mode);
    }
    for (frame, hit, expected) in [(1, a, vec![a]), (2, b, vec![a, b]), (3, a, vec![b])] {
        let result = feedback(frame, world, 4, Some(hit));
        let (mode, hit) = picking
            .resolve(&result, world, 4, selection.generation())
            .unwrap();
        selection.apply_pick(hit.iter().copied(), mode);
        assert_eq!(
            selection
                .ids()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            expected.into_iter().collect()
        );
        assert!(
            picking
                .resolve(&result, world, 4, selection.generation())
                .is_none()
        );
    }
    assert!(
        picking
            .resolve(
                &feedback(9, world, 4, None),
                world,
                4,
                selection.generation()
            )
            .is_none()
    );
}

#[test]
fn batches_replace_or_toggle_once_without_invalidating_later_feedback() {
    let world = WorldId::new();
    let [a, b, c] = [EntityId::new(), EntityId::new(), EntityId::new()];
    let mut selection = Selection::default();
    let mut picking = Picking::default();
    let generation = selection.generation();
    for (frame, mode) in [
        (1, SelectMode::Replace),
        (2, SelectMode::Toggle),
        (3, SelectMode::Toggle),
    ] {
        picking.request(frame, world, 4, generation, mode);
    }
    for (frame, hits, expected) in [
        (1, vec![a, b, a], vec![a, b]),
        (2, vec![b, c, c], vec![a, c]),
        (3, vec![a], vec![c]),
    ] {
        let mut result = feedback(frame, world, 4, None);
        result.pick.as_mut().unwrap().entities = hits;
        let (mode, hits) = picking.resolve(&result, world, 4, generation).unwrap();
        selection.apply_pick(hits.iter().copied(), mode);
        assert_eq!(
            selection
                .ids()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            expected.into_iter().collect()
        );
        assert_eq!(selection.generation(), generation);
        assert!(picking.resolve(&result, world, 4, generation).is_none());
    }
    selection.apply_pick([], SelectMode::Toggle);
    assert_eq!(selection.ids(), vec![c]);
    selection.apply_pick([], SelectMode::Replace);
    assert!(selection.is_empty());
    assert_eq!(selection.generation(), generation);
}

#[test]
fn wrong_world_target_or_feedback_token_never_resolves_a_click() {
    let world = WorldId::new();
    for case in 0..5 {
        let mut picking = Picking::default();
        picking.request(1, world, 7, 8, SelectMode::Replace);
        let mut result = feedback(1, world, 7, Some(EntityId::new()));
        let (current_world, target, selection) = match case {
            0 => (WorldId::new(), 7, 8),
            1 => (world, 9, 8),
            2 => (world, 7, 9),
            3 => {
                result.pick.as_mut().unwrap().token.world = WorldId::new();
                (world, 7, 8)
            }
            _ => {
                result.token.target_generation = 9;
                (world, 7, 8)
            }
        };
        assert!(
            picking
                .resolve(&result, current_world, target, selection)
                .is_none()
        );
        assert!(picking.pending.is_empty());
    }
}

#[test]
fn filtered_or_missing_feedback_retires_old_requests_and_storage_is_bounded() {
    let world = WorldId::new();
    let mut picking = Picking::default();
    for frame in 0..100 {
        picking.request(frame, world, 1, 0, SelectMode::Replace);
    }
    assert_eq!(picking.pending.len(), MAX_PENDING_PICKS);
    assert!(
        picking
            .resolve(&feedback(0, world, 1, None), world, 1, 0)
            .is_none()
    );
    let mut absent = feedback(99, world, 1, None);
    absent.pick = None;
    assert!(picking.resolve(&absent, world, 1, 0).is_none());
    assert!(picking.pending.is_empty());

    picking.request(100, world, 1, 0, SelectMode::Replace);
    picking.request(101, world, 2, 0, SelectMode::Toggle);
    assert_eq!(
        picking.pending.len(),
        1,
        "new target retires the old request chain"
    );
    assert!(
        picking
            .resolve(&feedback(100, world, 1, None), world, 2, 0)
            .is_none()
    );
    assert_eq!(picking.pending.len(), 1);
    let hit = EntityId::new();
    assert_eq!(
        picking.resolve(&feedback(101, world, 2, Some(hit)), world, 2, 0),
        Some((SelectMode::Toggle, [hit].as_slice()))
    );
    picking.request(102, world, 2, 0, SelectMode::Toggle);
    assert!(
        picking
            .resolve(&feedback(105, world, 2, None), world, 2, 0)
            .is_none()
    );
    assert!(picking.pending.is_empty());
}
