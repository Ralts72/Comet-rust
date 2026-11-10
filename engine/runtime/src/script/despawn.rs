//! Structural requests commit at the end of initialization or a fixed step, after callbacks finish.
use super::Scripts;
use crate::{animation::Animation, audio::Audio, navigation::Navigation, physics::Physics};
use anyhow::{Result, ensure};
use comet_core::{ActionInput, EntityId};
use comet_scene::{EntityRef, Scene, SceneSnapshot};
use std::{collections::BTreeSet, sync::Arc};

pub(super) fn validate(
    snapshot: &SceneSnapshot,
    roots: &BTreeSet<EntityId>,
    physics: &Physics,
) -> Result<()> {
    if roots.is_empty() {
        return Ok(());
    }
    let removed = snapshot.subtree_ids(&roots.iter().copied().collect::<Vec<_>>())?;
    physics.validate_removal(snapshot, &removed)?;
    // A live rig cannot lose bones, its animator or its character driver target mid-playback.
    // Remove their enclosing actor subtree together; ordinary links use Scene's shared cleanup.
    for entity in snapshot
        .entities
        .iter()
        .filter(|e| !removed.contains(&e.id))
    {
        for reference in entity
            .animator
            .iter()
            .flat_map(|a| a.bindings.values().flatten())
            .chain(entity.skin.iter().filter_map(|s| s.animator.as_ref()))
            .chain(
                entity
                    .character_animation
                    .iter()
                    .flat_map(|d| d.character.iter().chain(d.animator.iter())),
            )
        {
            ensure!(
                !matches!(reference, EntityRef::Local { entity } if removed.contains(entity)),
                "cannot destroy part of a live animation rig; destroy the enclosing actor subtree"
            );
        }
    }
    Ok(())
}

impl Scripts {
    /// Destruction is deferred so the complete event batch observes live entity identities.
    #[allow(clippy::too_many_arguments)]
    pub fn flush_destruction(
        &mut self,
        tick: u64,
        scene: &mut Scene,
        physics: &mut Physics,
        mut navigation: Option<&mut Navigation>,
        audio: &mut Audio,
        animation: &mut Animation,
    ) -> Result<()> {
        if self.pending_destruction.is_empty() {
            return Ok(());
        }
        let roots = self.pending_destruction.iter().copied().collect::<Vec<_>>();
        let removed = scene.shared_snapshot().subtree_ids(&roots)?;
        self.phase();
        for index in 0..self.bindings.len() {
            if removed.contains(&self.bindings[index].entity) {
                self.call(
                    index,
                    "on_stop",
                    tick,
                    &ActionInput::default(),
                    scene,
                    physics,
                    navigation.as_deref_mut(),
                    audio,
                    animation,
                    None,
                    false,
                );
            }
        }
        scene.remove_subtrees(&roots)?;
        let snapshot = scene.shared_snapshot();
        physics.remove_entities(&removed, &snapshot);
        if let Some(nav) = navigation {
            nav.remove_entities(&removed);
            self.navigation_statuses = Some(nav.statuses());
        }
        animation.remove_entities(&removed);
        audio.remove_entities(&removed);
        self.hud.remove_entities(&removed);
        self.bindings
            .retain(|binding| !removed.contains(&binding.entity));
        for binding in &mut self.bindings {
            let index = snapshot
                .entities
                .binary_search_by_key(&binding.entity, |e| e.id)
                .expect("surviving script owner");
            let parameters = &snapshot.entities[index]
                .script
                .as_ref()
                .expect("surviving script binding")
                .parameters;
            if *binding.parameters != *parameters {
                binding.parameters = Arc::new(parameters.clone());
            }
        }
        if self
            .scene_request
            .as_ref()
            .is_some_and(|r| removed.contains(&r.entity))
        {
            self.scene_request = None;
        }
        self.pending_destruction.clear();
        Ok(())
    }
}
