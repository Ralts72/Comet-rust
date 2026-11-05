use anyhow::Result;
use comet_core::AssetId;
use comet_editor_core::Document;
use comet_scene::{AudioSource, ContentSource, EntityData, PrefabResolver, SceneSnapshot};
struct Empty;
impl PrefabResolver for Empty {
    fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
        anyhow::bail!("unexpected prefab")
    }
}

#[test]
fn audio_component_preview_rejects_bad_candidates_and_preserves_history_savepoints_and_uuids()
-> Result<()> {
    let entity = EntityData::new("audio");
    let id = entity.id;
    let mut document = Document::open(SceneSnapshot {
        version: 6,
        entities: vec![entity],
    })?;
    let initial = document.shared_source();
    let mut source = AudioSource::new(AssetId::new());
    source.autoplay = true;
    document.begin()?;
    document.preview_audio_source(id, Some(source.clone()), &mut Empty)?;
    assert_eq!(document.shared_source().base.version, 7);
    let preview = document.shared_source();
    let mut bad = source.clone();
    bad.gain = f32::NAN;
    assert!(
        document
            .preview_audio_source(id, Some(bad), &mut Empty)
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&preview, &document.shared_source()));
    document.commit()?;
    document.undo()?;
    assert_eq!(document.shared_source(), initial);
    assert!(!document.is_dirty());
    document.redo()?;
    assert_eq!(document.shared_source(), preview);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("audio.scene");
    document.save(&path)?;
    assert_eq!(Document::load(&path)?.shared_source(), preview);
    document.begin()?;
    document.preview_audio_source(id, None, &mut Empty)?;
    document.commit()?;
    assert!(
        document.shared_source().base.entities[0]
            .audio_source
            .is_none()
    );
    document.undo()?;
    assert_eq!(document.shared_source(), preview);
    assert!(!document.is_dirty());
    assert_eq!(document.shared_source().base.entities[0].id, id);
    Ok(())
}

#[test]
fn spatial_previews_upgrade_v7_atomically_and_returning_to_the_baseline_restores_protocol_and_redo()
-> Result<()> {
    use comet_editor_core::{FieldEdit, FieldTarget};
    use comet_scene::schema::{self, FieldValue};
    let mut entity = EntityData::new("old sound");
    let id = entity.id;
    entity.audio_source = Some(AudioSource::new(AssetId::new()));
    let mut document = Document::open(SceneSnapshot {
        version: 7,
        entities: vec![entity],
    })?;
    let original = document.shared_source();
    document.begin()?;
    document.preview_field_values(
        &[FieldEdit {
            target: FieldTarget::Authored(id),
            field: schema::AUDIO_GAIN,
            value: FieldValue::Scalar(0.5),
        }],
        &mut Empty,
    )?;
    document.commit()?;
    document.undo()?;
    let history = document.history_len();
    document.begin()?;
    document.preview_field_values(
        &[FieldEdit {
            target: FieldTarget::Authored(id),
            field: schema::AUDIO_MAX_DISTANCE,
            value: FieldValue::Scalar(40.),
        }],
        &mut Empty,
    )?;
    assert_eq!(document.shared_source().base.version, 8);
    let accepted = document.shared_source();
    assert!(
        document
            .preview_field_values(
                &[FieldEdit {
                    target: FieldTarget::Authored(id),
                    field: schema::AUDIO_MIN_DISTANCE,
                    value: FieldValue::Scalar(50.)
                }],
                &mut Empty
            )
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&accepted, &document.shared_source()));
    document.preview_field_values(
        &[FieldEdit {
            target: FieldTarget::Authored(id),
            field: schema::AUDIO_MAX_DISTANCE,
            value: FieldValue::Scalar(30.),
        }],
        &mut Empty,
    )?;
    assert_eq!(document.shared_source(), original);
    document.commit()?;
    assert_eq!(document.history_len(), history);
    document.redo()?;
    assert_eq!(document.shared_source().base.version, 7);
    let mut spatial = document.shared_source().base.entities[0]
        .audio_source
        .clone()
        .unwrap();
    spatial.spatial = true;
    document.begin()?;
    document.preview_audio_source(id, Some(spatial), &mut Empty)?;
    document.commit()?;
    assert_eq!(document.shared_source().base.version, 8);
    document.undo()?;
    assert_eq!(document.shared_source().base.version, 7);
    document.redo()?;
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("spatial.scene");
    document.save(&path)?;
    let saved = Document::load(&path)?;
    assert!(
        saved.shared_source().base.entities[0]
            .audio_source
            .as_ref()
            .unwrap()
            .spatial
    );
    assert_eq!(saved.shared_source().base.version, 8);
    Ok(())
}
