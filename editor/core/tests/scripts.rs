use anyhow::Result;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{
    ContentSource, EntityData, PrefabInstance, PrefabResolver, SceneSnapshot, Script, ScriptValue,
};
use std::{collections::BTreeMap, sync::Arc};

struct Library(AssetId, ContentSource);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        anyhow::ensure!(id == self.0, "missing prefab");
        Ok(self.1.clone())
    }
}

#[test]
fn script_component_transactions_keep_history_and_savepoints() -> Result<()> {
    let entity = EntityData::new("author");
    let id = entity.id;
    let mut doc = Document::open(SceneSnapshot {
        version: comet_scene::scene::SCENE_VERSION,
        entities: vec![entity],
    })?;
    let mut library = Library(AssetId::new(), Default::default());
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("author.scene");
    doc.save(&path)?;
    let before = doc.shared_source();
    let mut script = Script::new(AssetId::new());
    script
        .parameters
        .insert("speed".into(), ScriptValue::Number(2.5));
    assert!(
        doc.preview_script(id, Some(script.clone()), &mut library)
            .is_err()
    );
    doc.begin()?;
    doc.preview_script(id, Some(script.clone()), &mut library)?;
    script.enabled = false;
    doc.preview_script(id, Some(script), &mut library)?;
    assert_eq!(doc.history_len(), 0);
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    let after = doc.shared_source();
    assert!(doc.save(temp.path()).is_err());
    assert!(doc.is_dirty());
    assert_eq!(Document::load(&path)?.shared_source(), before);
    doc.undo()?;
    assert_eq!(doc.shared_source(), before);
    assert!(!doc.is_dirty());
    doc.redo()?;
    assert_eq!(doc.shared_source(), after);
    doc.save(&path)?;
    assert!(!doc.is_dirty());
    assert_eq!(Document::load(&path)?.shared_source(), after);
    doc.begin()?;
    doc.preview_script(id, None, &mut library)?;
    doc.commit()?;
    assert!(doc.source().base.entities[0].script.is_none());
    doc.undo()?;
    assert_eq!(doc.shared_source(), after);
    Ok(())
}

#[test]
fn invalid_script_component_candidates_keep_the_last_preview_and_reject_prefab_structure()
-> Result<()> {
    let entity = EntityData::new("author");
    let id = entity.id;
    let prefab = EntityData::new("prefab");
    let prefab_id = prefab.id;
    let mut library = Library(
        AssetId::new(),
        ContentSource {
            base: SceneSnapshot {
                entities: vec![prefab],
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let instance = InstanceId::new();
    let mut doc = Document::open_source(ContentSource {
        base: SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: library.0,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    doc.begin()?;
    doc.preview_script(id, Some(Script::new(AssetId::new())), &mut library)?;
    let accepted = doc.shared_source();
    let revision = doc.revision();
    let mut invalid = Script::new(AssetId::new());
    invalid.parameters = BTreeMap::from([("speed".into(), ScriptValue::Number(f64::NAN))]);
    assert!(doc.preview_script(id, Some(invalid), &mut library).is_err());
    for target in [EntityId::new(), prefab_id.in_instance(instance)] {
        assert!(
            doc.preview_script(target, Some(Script::new(AssetId::new())), &mut library)
                .is_err()
        );
    }
    library.0 = AssetId::new();
    assert!(doc.preview_script(id, None, &mut library).is_err());
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.history_len(), 0);
    doc.cancel()?;
    assert!(!doc.is_dirty());
    assert!(doc.source().base.entities[0].script.is_none());
    Ok(())
}
