use anyhow::{Context, Result};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::{Document, OverrideReset};
use comet_scene::{
    AudioSource, ContentSource, EntityData, FieldOverride, PrefabInstance, PrefabResolver,
    SceneSnapshot,
    schema::{self, FieldKey, FieldValue},
};
use std::sync::Arc;

struct Library {
    id: AssetId,
    source: ContentSource,
}
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        (id == self.id)
            .then(|| self.source.clone())
            .context("unknown prefab")
    }
}
struct Fixture {
    doc: Document,
    library: Library,
    node: EntityId,
    instances: [InstanceId; 2],
}
impl Fixture {
    fn new() -> Result<Self> {
        let mut node = EntityData::new("audio source");
        node.audio_source = Some(AudioSource {
            min_distance: 1.,
            max_distance: 5.,
            ..AudioSource::new(AssetId::new())
        });
        let other = EntityData::new("unselected node");
        let id = AssetId::new();
        let node_id = node.id;
        let other_id = other.id;
        let library = Library {
            id,
            source: ContentSource {
                base: SceneSnapshot {
                    entities: vec![node, other],
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let instances = [InstanceId::new(), InstanceId::new()];
        let source = ContentSource {
            instances: instances
                .iter()
                .enumerate()
                .map(|(index, &instance)| PrefabInstance {
                    translation: [0.; 3],
                    id: instance,
                    prefab: id,
                    parent: None,
                    overrides: vec![
                        FieldOverride {
                            entity: node_id,
                            field: schema::AUDIO_MIN_DISTANCE,
                            value: FieldValue::Scalar(if index == 0 { 10. } else { 2. }),
                        },
                        FieldOverride {
                            entity: node_id,
                            field: schema::AUDIO_MAX_DISTANCE,
                            value: FieldValue::Scalar(if index == 0 { 20. } else { 4. }),
                        },
                        FieldOverride {
                            entity: other_id,
                            field: schema::TRANSLATION,
                            value: FieldValue::Vec3([3.; 3]),
                        },
                    ],
                })
                .collect(),
            ..Default::default()
        };
        Ok(Self {
            doc: Document::open_source(source)?,
            library,
            node: node_id,
            instances,
        })
    }
    fn resets(&self, fields: &[FieldKey]) -> Vec<OverrideReset> {
        self.instances
            .map(|instance| OverrideReset {
                instance,
                entity: self.node,
                fields: fields.to_vec(),
            })
            .to_vec()
    }
}

#[test]
fn group_override_reset_publishes_once_preserves_other_nodes_and_undo_redo() -> Result<()> {
    let mut f = Fixture::new()?;
    f.doc.expand(&mut f.library)?;
    let original = f.doc.shared_source();
    f.doc
        .revert_overrides_many(&f.resets(&[schema::AUDIO_MIN_DISTANCE]), &mut f.library)?;
    let edited = f.doc.shared_source();
    assert_eq!(f.doc.history_len(), 1);
    assert!(
        edited
            .instances
            .iter()
            .all(|instance| instance.overrides.len() == 2)
    );
    f.doc.undo()?;
    assert!(Arc::ptr_eq(&original, &f.doc.shared_source()));
    let state = f.doc.state_id();
    f.doc.revert_overrides_many(&[], &mut f.library)?;
    assert_eq!(f.doc.state_id(), state);
    f.doc.redo()?;
    assert_eq!(f.doc.shared_source(), edited);
    Ok(())
}

#[test]
fn invalid_group_reset_keeps_content_history_redo_and_active_preview_unchanged() -> Result<()> {
    let mut f = Fixture::new()?;
    let original = f.doc.shared_source();
    let state = f.doc.state_id();
    // The first source's inherited max=5 conflicts with its pinned min=10.
    assert!(
        f.doc
            .revert_overrides_many(&f.resets(&[schema::AUDIO_MAX_DISTANCE]), &mut f.library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&original, &f.doc.shared_source()));
    assert_eq!(f.doc.state_id(), state);
    assert_eq!(f.doc.history_len(), 0);
    let reset = f.resets(&[schema::AUDIO_MIN_DISTANCE]);
    assert!(
        f.doc
            .revert_overrides_many(&[reset[0].clone(), reset[0].clone()], &mut f.library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&original, &f.doc.shared_source()));
    f.doc.revert_overrides_many(&reset, &mut f.library)?;
    f.doc.undo()?;
    assert!(
        f.doc
            .revert_overrides_many(&f.resets(&[schema::AUDIO_MAX_DISTANCE]), &mut f.library)
            .is_err()
    );
    f.doc.redo()?;
    f.doc.begin()?;
    let preview = f.doc.shared_source();
    assert!(f.doc.revert_overrides_many(&reset, &mut f.library).is_err());
    assert!(Arc::ptr_eq(&preview, &f.doc.shared_source()));
    assert!(f.doc.is_editing());
    f.doc.cancel()?;
    Ok(())
}

#[test]
fn group_reset_can_recover_removed_source_components_without_implicit_version_downgrade()
-> Result<()> {
    let mut f = Fixture::new()?;
    f.library
        .source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.node)
        .unwrap()
        .audio_source = None;
    assert!(f.doc.expand(&mut f.library).is_err());
    f.doc.revert_overrides_many(
        &f.resets(&[schema::AUDIO_MIN_DISTANCE, schema::AUDIO_MAX_DISTANCE]),
        &mut f.library,
    )?;
    assert!(
        f.doc
            .expand(&mut f.library)?
            .entities
            .iter()
            .all(|entity| entity.audio_source.is_none())
    );
    assert!(
        f.doc
            .shared_source()
            .instances
            .iter()
            .all(|instance| instance.overrides.len() == 1)
    );
    assert_eq!(f.doc.history_len(), 1);
    assert_eq!(
        f.doc.shared_source().base.version,
        SceneSnapshot::default().version
    );
    Ok(())
}
