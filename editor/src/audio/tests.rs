use super::*;
use comet_asset_data::product::AssetKind;
use comet_editor_core::Document;
use comet_scene::{EntityData, SceneSnapshot};
use std::{fs, sync::Arc};

#[test]
fn audio_candidates_validate_before_history_and_keep_accepted_previews_when_host_guards_fail()
-> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let path = editor.project_root().join("assets/tone.wav");
    fs::write(&path, b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0\x77\x01\0\x02\0\x10\0data\x04\0\0\0\0\0\x01\0")?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let clip = comet_asset_pipeline::SourceMeta::load(&path)?.id;
    let wrong = assets
        .index()
        .products
        .iter()
        .find(|(_, entry)| entry.kind == AssetKind::Mesh)
        .map(|(&id, _)| id)
        .unwrap();
    let entity = EntityData::new("sound");
    let id = entity.id;
    editor.document = Document::open(SceneSnapshot {
        version: 6,
        entities: vec![entity],
    })?;
    let original = editor.document.shared_source();
    let history = editor.document.history_len();
    assert!(
        editor
            .edit_audio_source(id, Some(AudioSource::new(wrong)), &mut assets)
            .is_err()
    );
    assert!(!editor.document.is_editing());
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    let source = AudioSource {
        gain: 0.4,
        looping: true,
        ..AudioSource::new(clip)
    };
    editor.edit_audio_source(id, Some(source.clone()), &mut assets)?;
    let preview = editor.document.shared_source();
    assert_eq!(preview.base.version, 7);
    for bad in [
        AudioSource {
            gain: f32::NAN,
            ..source.clone()
        },
        AudioSource {
            clip: wrong,
            ..source.clone()
        },
    ] {
        assert!(
            editor
                .edit_audio_source(id, Some(bad), &mut assets)
                .is_err()
        );
        assert!(Arc::ptr_eq(&preview, &editor.document.shared_source()));
        assert!(editor.document.is_editing());
    }
    editor.cancel()?;
    assert_eq!(editor.document.shared_source(), original);
    editor.edit_audio_source(id, Some(source), &mut assets)?;
    editor.commit()?;
    assert_eq!(editor.document.history_len(), history + 1);
    editor.save(&mut assets)?;
    let saved = editor.document.shared_source();
    assert_eq!(Document::load(&editor.source)?.shared_source(), saved);
    editor.edit_audio_source(id, None, &mut assets)?;
    editor.commit()?;
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), saved);
    assert!(!editor.document.is_dirty());
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    assert!(editor.edit_audio_source(id, None, &mut assets).is_err());
    editor.documents = Default::default();
    editor.start(&mut assets)?;
    assert!(editor.edit_audio_source(id, None, &mut assets).is_err());
    editor.stop();
    assert!(Arc::ptr_eq(&saved, &editor.document.shared_source()));
    editor.refresh_resources()?;
    assert!(editor.is_busy());
    assert!(editor.edit_audio_source(id, None, &mut assets).is_err());
    assert!(Arc::ptr_eq(&saved, &editor.document.shared_source()));
    Ok(())
}
