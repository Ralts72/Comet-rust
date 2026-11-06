use super::*;
use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::{AssetId, InstanceId};
use comet_scene::{
    ContentSource, FieldOverride, Model, PrefabInstance,
    schema::{self, FieldValue},
};
use std::sync::Arc;

impl Group {
    pub(super) fn mixed_models(&mut self) -> Result<([AssetId; 2], [AssetId; 2])> {
        let products = |kind| -> Result<[AssetId; 2]> {
            self.assets
                .index()
                .products
                .iter()
                .filter_map(|(&id, entry)| (entry.kind == kind).then_some(id))
                .take(2)
                .collect::<Vec<_>>()
                .try_into()
                .map_err(|_| anyhow::anyhow!("two fixture model products"))
        };
        let meshes = products(AssetKind::Mesh)?;
        let materials = products(AssetKind::Material)?;
        let mut source = self.editor.document.source();
        for (index, id) in self.ids.iter().enumerate() {
            let model = source
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == *id)
                .unwrap()
                .model
                .as_mut()
                .unwrap();
            model.mesh = meshes[index];
            model.surface.material = materials[index];
        }
        self.editor.document = Document::open_source(source)?;
        Ok((meshes, materials))
    }
    fn model_values(&mut self) -> Result<[Model; 2]> {
        let snapshot = self.editor.snapshot(&mut self.assets)?;
        Ok(self.ids.map(|id| {
            snapshot
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .unwrap()
                .model
                .clone()
                .unwrap()
        }))
    }
    fn asset_label(&self, asset: AssetId) -> String {
        let metadata = self.assets.index().products[&asset]
            .editor
            .as_ref()
            .unwrap();
        if metadata.source == asset {
            metadata.label.clone()
        } else {
            let source = self.assets.index().products[&metadata.source]
                .editor
                .as_ref()
                .unwrap();
            format!("{} / {}", source.label, metadata.label)
        }
    }
    fn open_model_picker(&mut self, field: &str) -> Result<Pos2> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let values = texts(&output);
        let row = values
            .iter()
            .find(|(text, pos)| {
                pos.x > 970. && (text == field || text == &format!("{field}（多值）"))
            })
            .context("common model asset row")?
            .1;
        let anchor = values
            .iter()
            .find(|(_, pos)| (pos.y - row.y).abs() < 2. && pos.x > row.x + 20.)
            .context("common model asset selector")?
            .1;
        self.frame(vec![Event::PointerMoved(anchor), button(anchor, true)]);
        self.frame(vec![button(anchor, false)]);
        Ok(anchor)
    }
    fn select_model_asset(&mut self, field: &str, asset: AssetId) -> Result<()> {
        let label = self.asset_label(asset);
        let anchor = self.open_model_picker(field)?;
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .find(|(text, pos)| text == &label && pos.y > anchor.y + 3.)
            .with_context(|| format!("common model option {label}"))?
            .1;
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }
}

#[test]
fn common_model_pickers_unify_primary_assets_preserve_roughness_and_commit_each_choice_once()
-> Result<()> {
    let mut f = Group::new()?;
    let (meshes, materials) = f.mixed_models()?;
    let original = f.editor.document.shared_source();
    let transforms = f.transforms();
    let path = f.editor.project_root().join("assets/showcase.gltf");
    let bytes = std::fs::read(&path)?;
    let metadata = std::fs::read(SourceMeta::path(&path))?;
    f.open_model_picker("网格")?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.select_model_asset("网格", meshes[1])?;
    let models = f.model_values()?;
    assert_eq!(models.each_ref().map(|model| model.mesh), [meshes[1]; 2]);
    assert_eq!(
        models.each_ref().map(|model| model.surface.material),
        materials
    );
    assert_eq!(
        models.each_ref().map(|model| model.surface.roughness),
        [0.2, 0.8]
    );
    assert_eq!(f.editor.document.history_len(), 1);
    let accepted = f.editor.document.shared_source();
    f.select_model_asset("网格", meshes[1])?;
    assert!(Arc::ptr_eq(&accepted, &f.editor.document.shared_source()));
    assert_eq!(f.editor.document.history_len(), 1);
    f.select_model_asset("材质", materials[1])?;
    let models = f.model_values()?;
    assert_eq!(models.each_ref().map(|model| model.mesh), [meshes[1]; 2]);
    assert_eq!(
        models.each_ref().map(|model| model.surface.material),
        [materials[1]; 2]
    );
    assert_eq!(
        models.each_ref().map(|model| model.surface.roughness),
        [0.2, 0.8]
    );
    assert_eq!(f.transforms(), transforms);
    assert_eq!(f.editor.document.history_len(), 2);
    let edited = f.editor.document.shared_source();
    f.editor.document.undo()?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), edited);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(std::fs::read(SourceMeta::path(&path))?, metadata);
    Ok(())
}

#[test]
fn invalid_common_model_resources_and_targets_never_open_or_partially_publish_a_transaction()
-> Result<()> {
    let mut f = Group::new()?;
    let (meshes, materials) = f.mixed_models()?;
    let original = f.editor.document.shared_source();
    let state = f.editor.document.state_id();
    let revision = f.editor.document.revision();
    for (ids, field, asset) in [
        (f.ids, schema::MESH, materials[0]),
        (f.ids, schema::MATERIAL, meshes[0]),
        (f.ids, schema::MESH, AssetId::new()),
        ([f.ids[0], EntityId::new()], schema::MESH, meshes[1]),
        ([f.ids[0]; 2], schema::MESH, meshes[1]),
        (f.ids, schema::ROUGHNESS, meshes[0]),
    ] {
        assert!(
            f.editor
                .replace_model_asset(&ids, field, asset, &mut f.assets)
                .is_err()
        );
        assert!(Arc::ptr_eq(&original, &f.editor.document.shared_source()));
        assert_eq!(f.editor.document.state_id(), state);
        assert_eq!(f.editor.document.revision(), revision);
        assert_eq!(f.editor.document.history_len(), 0);
        assert!(!f.editor.document.is_editing() && !f.editor.document.is_dirty());
    }
    Ok(())
}

#[test]
fn common_model_validation_keeps_accepted_previews_and_ignores_unrelated_broken_draws() -> Result<()>
{
    let mut f = Group::new()?;
    let (meshes, materials) = f.mixed_models()?;
    let mut source = f.editor.document.source();
    let mut broken = EntityData::new("未选中的损坏模型");
    broken.transform.scale = [1e-40; 3];
    broken.model = Some(Model {
        mesh: AssetId::new(),
        ..f.model_values()?[0].clone()
    });
    source.base.entities.push(broken);
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.editor
        .replace_model_asset(&f.ids, schema::MESH, meshes[1], &mut f.assets)?;
    let accepted = f.editor.document.shared_source();
    let revision = f.editor.document.revision();
    assert!(
        f.editor
            .replace_model_asset(&f.ids, schema::MATERIAL, meshes[0], &mut f.assets)
            .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &f.editor.document.shared_source()));
    assert_eq!(f.editor.document.revision(), revision);
    assert!(f.editor.document.is_editing());
    f.editor.cancel()?;
    assert_eq!(f.editor.document.shared_source(), original);
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .transform
        .scale = [1e-40; 3];
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_model_asset(&f.ids, schema::MATERIAL, materials[1], &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    Ok(())
}

#[test]
fn a_missing_model_hides_all_common_model_fields_and_rejects_replacement() -> Result<()> {
    let mut f = Group::new()?;
    let (meshes, _) = f.mixed_models()?;
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .model = None;
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    f.frame(vec![]);
    let output = f.frame(vec![]);
    assert!(!texts(&output).iter().any(|(text, pos)| pos.x > 970.
        && (text.starts_with("网格") || text.starts_with("材质") || text.starts_with("粗糙度"))));
    assert!(
        f.editor
            .replace_model_asset(&f.ids, schema::MESH, meshes[1], &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn common_prefab_assets_keep_source_ids_existing_pins_and_undo_savepoints() -> Result<()> {
    let mut f = Group::new()?;
    let (meshes, materials) = f.mixed_models()?;
    let mut source = f.editor.document.source();
    let mut node = source
        .base
        .entities
        .iter()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .clone();
    node.parent = None;
    let node_id = node.id;
    let path = f.editor.project_root().join("assets/common-model.prefab");
    std::fs::write(
        &path,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![node],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let bytes = std::fs::read(&path)?;
    let metadata = std::fs::read(SourceMeta::path(&path))?;
    let instance = InstanceId::new();
    source.base.entities.retain(|entity| entity.id == f.ids[0]);
    source.base.entities[0].model.as_mut().unwrap().mesh = meshes[1];
    let pin = FieldOverride {
        entity: node_id,
        field: schema::ROUGHNESS,
        value: FieldValue::Scalar(0.8),
    };
    source.instances.push(PrefabInstance {
        id: instance,
        prefab: SourceMeta::load(&path)?.id,
        parent: None,
        overrides: vec![pin.clone()],
    });
    f.editor.document = Document::open_source(source)?;
    f.ids[1] = node_id.in_instance(instance);
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    f.select_model_asset("网格", meshes[0])?;
    f.select_model_asset("材质", materials[0])?;
    let edited = f.editor.document.shared_source();
    let overrides = &edited.instances[0].overrides;
    assert_eq!(overrides.len(), 3);
    assert!(overrides.contains(&pin));
    assert!(overrides.iter().all(|value| value.entity == node_id));
    assert!(overrides.iter().any(|value| value.field == schema::MESH && value.value == FieldValue::Asset(meshes[0])));
    assert!(
        overrides.iter().any(|value| value.field == schema::MATERIAL
            && value.value == FieldValue::Asset(materials[0]))
    );
    let models = f.model_values()?;
    assert_eq!(
        models.each_ref().map(|model| model.surface.roughness),
        [0.2, 0.8]
    );
    assert_eq!(f.editor.document.history_len(), 2);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(std::fs::read(SourceMeta::path(&path))?, metadata);
    f.editor.document.undo()?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    let state = f.editor.document.state_id();
    f.editor
        .replace_model_asset(&f.ids, schema::MESH, meshes[0], &mut f.assets)?;
    f.editor
        .replace_model_asset(&f.ids, schema::MESH, meshes[1], &mut f.assets)?;
    f.editor.commit()?;
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.state_id(), state);
    assert_eq!(f.editor.document.history_len(), 0);
    assert_eq!(
        f.editor.document.shared_source().instances[0].overrides,
        [pin]
    );
    f.editor.document.redo()?;
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), edited);
    assert!(!f.editor.document.is_dirty());
    Ok(())
}

#[test]
fn a_nested_model_rejects_the_whole_replacement_even_when_its_asset_is_unchanged() -> Result<()> {
    let mut f = Group::new()?;
    let (meshes, _) = f.mixed_models()?;
    let source = f.editor.document.source();
    let mut node = source
        .base
        .entities
        .iter()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .clone();
    node.parent = None;
    let node_id = node.id;
    let inner = f.editor.project_root().join("assets/model-inner.prefab");
    std::fs::write(
        &inner,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![node],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    let child = InstanceId::new();
    let outer = f.editor.project_root().join("assets/model-outer.prefab");
    std::fs::write(
        &outer,
        ContentSource {
            instances: vec![PrefabInstance {
                id: child,
                prefab: SourceMeta::load(&inner)?.id,
                parent: None,
                overrides: vec![],
            }],
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let instance = InstanceId::new();
    let mut source = f.editor.document.source();
    source.base.entities.retain(|entity| entity.id == f.ids[0]);
    source.instances.push(PrefabInstance {
        id: instance,
        prefab: SourceMeta::load(&outer)?.id,
        parent: None,
        overrides: vec![],
    });
    f.editor.document = Document::open_source(source)?;
    f.ids[1] = node_id.in_instance(child.in_instance(instance));
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .snapshot(&mut f.assets)?
            .entities
            .iter()
            .any(|entity| entity.id == f.ids[1])
    );
    assert!(
        f.editor
            .replace_model_asset(&f.ids, schema::MESH, meshes[1], &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn common_model_resources_are_read_only_in_play() -> Result<()> {
    let mut f = Group::new()?;
    let (meshes, _) = f.mixed_models()?;
    let models = f.model_values()?;
    let original = f.editor.document.shared_source();
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    f.open_model_picker("网格")?;
    f.open_model_picker("材质")?;
    assert!(
        f.editor
            .replace_model_asset(&f.ids, schema::MESH, meshes[1], &mut f.assets)
            .is_err()
    );
    assert_eq!(f.model_values()?, models);
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.stop();
    Ok(())
}
