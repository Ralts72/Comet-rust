use super::*;

#[test]
fn range_anchor_toggles_and_pruning_keep_one_active_member() {
    let ids = [
        EntityId::new(),
        EntityId::new(),
        EntityId::new(),
        EntityId::new(),
    ];
    let mut selection = Selection::default();
    selection.choose(Some(ids[1]), SelectMode::Replace);
    selection.range(&ids, ids[3], false);
    assert_eq!(selection.len(), 3);
    selection.range(&ids, ids[0], false);
    assert_eq!(selection.len(), 2);
    assert!(selection.contains(ids[0]) && selection.contains(ids[1]));
    selection.choose(Some(ids[2]), SelectMode::Toggle);
    assert_eq!(selection.active(), Some(ids[2]));
    selection.choose(Some(ids[2]), SelectMode::Toggle);
    assert!(selection.active().is_some_and(|id| selection.contains(id)));
    selection.retain(|id| id == ids[0]);
    assert_eq!(selection.active(), Some(ids[0]));
    selection.retain(|_| false);
    assert!(selection.is_empty());
    assert_eq!(selection.active(), None);
}

#[test]
fn direct_intent_invalidates_old_picks_but_ordered_pick_results_can_accumulate() {
    let a = EntityId::new();
    let b = EntityId::new();
    let mut selection = Selection::default();
    selection.choose(Some(a), SelectMode::Replace);
    let generation = selection.generation();
    selection.choose(Some(a), SelectMode::Replace);
    assert_ne!(selection.generation(), generation);
    let generation = selection.generation();
    selection.apply_pick(Some(b), SelectMode::Toggle);
    selection.apply_pick(Some(a), SelectMode::Toggle);
    assert_eq!(selection.ids(), vec![b]);
    assert_eq!(selection.generation(), generation);
    selection.apply_pick(None, SelectMode::Toggle);
    assert_eq!(selection.ids(), vec![b]);
    selection.apply_pick(None, SelectMode::Replace);
    assert!(selection.is_empty());
}

#[test]
fn toggle_blank_does_not_reset_the_range_anchor() {
    let ids = [EntityId::new(), EntityId::new(), EntityId::new()];
    let mut selection = Selection::default();
    selection.choose(Some(ids[0]), SelectMode::Replace);
    selection.range(&ids, ids[2], false);
    let before = selection.clone();
    selection.apply_pick(None, SelectMode::Toggle);
    assert_eq!(selection, before);
    selection.range(&ids, ids[1], false);
    assert!(selection.contains(ids[0]) && selection.contains(ids[1]));
    assert_eq!(selection.len(), 2);
}
