use anyhow::{Result, ensure};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::{Document, FieldEdit, FieldTarget};
use comet_scene::{
    ContentSource, EntityData, Model, PrefabInstance, PrefabResolver, SceneSnapshot, Surface,
    schema::{self, FieldValue},
};
use std::sync::Arc;

struct Library {
    asset: AssetId,
    source: ContentSource,
    fail: bool,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            asset: AssetId::new(),
            source: ContentSource::default(),
            fail: false,
        }
    }
}
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        ensure!(!self.fail && id == self.asset, "missing prefab");
        Ok(self.source.clone())
    }
}
fn model() -> Model {
    // Core stores classified identities without resolving products or GPU transforms.
    Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    }
}

#[test]
fn model_structure_preserves_v1_history_savepoints_and_non_drawable_author_transforms() -> Result<()>
{
    let mut entity = EntityData::new("tiny author");
    entity.transform.scale = [1e-40; 3];
    let id = entity.id;
    let mut doc = Document::open(SceneSnapshot {
        version: 1,
        entities: vec![entity],
    })?;
    let mut library = Library::default();
    let before = doc.shared_source();
    let first = model();
    assert!(
        doc.preview_model(id, Some(first.clone()), &mut library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &doc.shared_source()));
    doc.begin()?;
    doc.preview_model(id, Some(first.clone()), &mut library)?;
    let accepted = doc.shared_source();
    let state = doc.state_id();
    let revision = doc.revision();
    doc.preview_model(id, Some(first), &mut library)?;
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    assert_eq!(doc.state_id(), state);
    assert_eq!(doc.revision(), revision);
    let replacement = model();
    doc.preview_model(id, Some(replacement.clone()), &mut library)?;
    doc.commit()?;
    let after = doc.shared_source();
    assert_eq!(after.base.version, 1);
    assert_eq!(after.base.entities[0].model, Some(replacement.clone()));
    assert_eq!(after.base.entities[0].transform.scale, [1e-40; 3]);
    assert_eq!(doc.history_len(), 1);
    doc.undo()?;
    assert_eq!(doc.shared_source(), before);
    assert!(!doc.is_dirty());
    doc.redo()?;
    assert_eq!(doc.shared_source(), after);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("model.scene");
    doc.save(&path)?;
    assert!(!doc.is_dirty());
    assert_eq!(Document::load(&path)?.shared_source(), after);

    // Removing this model must not require its old products or a usable GPU normal matrix.
    doc.begin()?;
    doc.preview_model(id, None, &mut library)?;
    doc.commit()?;
    let removed = doc.shared_source();
    assert!(removed.base.entities[0].model.is_none());
    assert_eq!(removed.base.version, 1);
    assert_eq!(doc.history_len(), 2);
    doc.undo()?;
    assert_eq!(doc.shared_source(), after);
    assert!(!doc.is_dirty());
    doc.begin()?;
    doc.preview_model(id, Some(replacement), &mut library)?;
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    doc.redo()?;
    assert_eq!(doc.shared_source(), removed, "no-op must retain redo");
    doc.save(&path)?;
    assert_eq!(Document::load(&path)?.shared_source(), removed);
    Ok(())
}

#[test]
fn invalid_model_candidates_preserve_the_last_preview_and_prefab_component_structure() -> Result<()>
{
    let author = EntityData::new("author");
    let id = author.id;
    let prefab = EntityData::new("prefab");
    let prefab_id = prefab.id;
    let instance = InstanceId::new();
    let mut library = Library {
        source: ContentSource {
            base: SceneSnapshot {
                version: 1,
                entities: vec![prefab],
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let original_prefab = library.source.clone();
    let mut doc = Document::open_source(ContentSource {
        base: SceneSnapshot {
            version: 1,
            entities: vec![author],
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: library.asset,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    let before = doc.shared_source();
    doc.begin()?;
    doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([1., 2., 3.]))?;
    doc.preview_model(id, Some(model()), &mut library)?;
    let accepted = doc.shared_source();
    let state = doc.state_id();
    let revision = doc.revision();
    for roughness in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        let mut invalid = model();
        invalid.surface.roughness = roughness;
        assert!(doc.preview_model(id, Some(invalid), &mut library).is_err());
        assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    }
    for target in [EntityId::new(), prefab_id, prefab_id.in_instance(instance)] {
        assert!(
            doc.preview_model(target, Some(model()), &mut library)
                .is_err()
        );
        assert!(doc.preview_model(target, None, &mut library).is_err());
        assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    }
    library.fail = true;
    assert!(doc.preview_model(id, None, &mut library).is_err());
    library.fail = false;
    library.source.base.entities[0].transform.scale = [f32::NAN; 3];
    assert!(doc.preview_model(id, Some(model()), &mut library).is_err());
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    assert_eq!(doc.state_id(), state);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.history_len(), 0);
    assert_eq!(accepted.instances, before.instances);
    library.source = original_prefab;
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    assert_eq!(doc.shared_source(), accepted);
    doc.undo()?;
    assert_eq!(doc.shared_source(), before);
    assert!(!doc.is_dirty());
    Ok(())
}

#[test]
fn model_field_batches_follow_component_presence_and_never_publish_a_partial_assignment()
-> Result<()> {
    let entity = EntityData::new("author");
    let id = entity.id;
    let mut doc = Document::open(SceneSnapshot {
        version: 1,
        entities: vec![entity],
    })?;
    let mut library = Library::default();
    let target = FieldTarget::Authored(id);
    let mesh = AssetId::new();
    let material = AssetId::new();
    let fields = [
        FieldEdit {
            target,
            field: schema::MESH,
            value: FieldValue::Asset(mesh),
        },
        FieldEdit {
            target,
            field: schema::MATERIAL,
            value: FieldValue::Asset(material),
        },
        FieldEdit {
            target,
            field: schema::ROUGHNESS,
            value: FieldValue::Scalar(0.8),
        },
    ];
    let before = doc.shared_source();
    doc.begin()?;
    for edit in &fields {
        assert!(
            doc.preview_fields(std::slice::from_ref(edit), &mut library)
                .is_err()
        );
        assert!(Arc::ptr_eq(&before, &doc.shared_source()));
    }
    doc.preview_model(id, Some(model()), &mut library)?;
    let with_model = doc.shared_source();
    let mut invalid = fields.clone();
    invalid[2].value = FieldValue::Scalar(f32::NAN);
    assert!(doc.preview_fields(&invalid, &mut library).is_err());
    assert!(Arc::ptr_eq(&with_model, &doc.shared_source()));
    doc.preview_fields(&fields, &mut library)?;
    assert_eq!(
        doc.shared_source().base.entities[0].model,
        Some(Model {
            mesh,
            surface: Surface {
                material,
                roughness: 0.8,
            },
        })
    );
    doc.preview_model(id, None, &mut library)?;
    assert!(doc.preview_fields(&fields, &mut library).is_err());
    assert_eq!(doc.shared_source(), before);
    doc.commit()?;
    assert_eq!(doc.history_len(), 0);
    assert!(!doc.is_dirty());
    assert_eq!(doc.shared_source().base.version, 1);
    Ok(())
}
