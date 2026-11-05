use super::*;
use comet_asset_data::product::AssetKind;
use comet_editor_core::Document;
use comet_scene::EntityData;
use std::{fs, sync::Arc};

fn fixture_model(assets: &mut AssetStore) -> Result<Model> {
    let mesh = assets
        .index()
        .products
        .iter()
        .find_map(|(&id, entry)| (entry.kind == AssetKind::Mesh).then_some(id))
        .context("fixture mesh")?;
    let material = assets
        .index()
        .products
        .iter()
        .find_map(|(&id, entry)| (entry.kind == AssetKind::Material).then_some(id))
        .context("fixture material")?;
    let product = assets.load(material)?;
    let Product::Material(surface) = &product.payload else {
        anyhow::bail!("fixture material kind changed")
    };
    Ok(Model {
        mesh,
        surface: Surface {
            material,
            roughness: surface.roughness,
        },
    })
}

#[test]
fn non_drawable_target_rejects_addition_without_changing_saved_content_or_an_existing_preview()
-> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let model = fixture_model(&mut assets)?;
    let mut tiny = EntityData::new("tiny empty author");
    tiny.transform.scale = [1e-40; 3];
    let id = tiny.id;
    let mut source = editor.document.source();
    source.base.entities.push(tiny);
    editor.document = Document::open_source(source)?;
    editor.save(&mut assets)?;
    let path = editor.project_root().join("assets/main.scene");
    let saved = fs::read(&path)?;
    let before = editor.document.shared_source();
    let state = editor.document.state_id();
    let revision = editor.document.revision();
    let history = editor.document.history_len();
    assert!(
        editor
            .add_model(id, model.mesh, model.surface.material, &mut assets)
            .is_err()
    );
    assert!(!editor.document.is_editing());
    assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
    assert_eq!(editor.document.state_id(), state);
    assert_eq!(editor.document.revision(), revision);
    assert_eq!(editor.document.history_len(), history);
    assert!(!editor.document.is_dirty());
    assert_eq!(fs::read(&path)?, saved);

    editor.document.begin()?;
    editor
        .document
        .preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([1., 2., 3.]))?;
    let preview = editor.document.shared_source();
    let state = editor.document.state_id();
    let revision = editor.document.revision();
    assert!(editor.edit_model(id, Some(model), &mut assets).is_err());
    assert!(editor.document.is_editing());
    assert!(Arc::ptr_eq(&preview, &editor.document.shared_source()));
    assert_eq!(editor.document.state_id(), state);
    assert_eq!(editor.document.revision(), revision);
    assert_eq!(editor.document.history_len(), history);
    assert!(editor.document.is_dirty());
    assert_eq!(fs::read(path)?, saved);
    editor.cancel()?;
    assert_eq!(editor.document.shared_source(), before);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn unrelated_bad_models_do_not_block_addition_and_removing_a_broken_model_restores_saving()
-> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let model = fixture_model(&mut assets)?;
    let mut broken = EntityData::new("broken model");
    broken.transform.scale = [1e-40; 3];
    broken.model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    let broken_id = broken.id;
    assert!(
        !assets
            .index()
            .products
            .contains_key(&broken.model.as_ref().unwrap().mesh)
    );
    assert!(
        !assets
            .index()
            .products
            .contains_key(&broken.model.as_ref().unwrap().surface.material)
    );
    let mut parent = EntityData::new("scaled parent");
    parent.transform.scale = [2., 1., 3.];
    let mut target = EntityData::new("rotated child");
    target.parent = Some(parent.id);
    target.transform.rotation = comet_core::math::Quat::from_rotation_y(0.4).to_array();
    let target_id = target.id;
    let mut source = editor.document.source();
    source
        .base
        .entities
        .extend([broken.clone(), parent, target]);
    editor.document = Document::open_source(source)?;
    let before = editor.document.shared_source();
    let snapshot = before.expand(&mut assets)?;
    assert!(comet_application::extract(&snapshot, &mut assets, &[]).is_err());
    editor.add_model(target_id, model.mesh, model.surface.material, &mut assets)?;
    let added = editor.document.shared_source();
    assert_eq!(
        added
            .base
            .entities
            .iter()
            .find(|entity| entity.id == target_id)
            .unwrap()
            .model,
        Some(model)
    );
    assert_eq!(
        added
            .base
            .entities
            .iter()
            .find(|entity| entity.id == broken_id),
        Some(&broken)
    );
    let snapshot = added.expand(&mut assets)?;
    assert!(comet_application::extract(&snapshot, &mut assets, &[]).is_err());

    // Removal bypasses the broken target's old product identities and GPU transform.
    editor.edit_model(broken_id, None, &mut assets)?;
    editor.commit()?;
    let repaired = editor.document.shared_source();
    assert!(
        repaired
            .base
            .entities
            .iter()
            .find(|entity| entity.id == broken_id)
            .unwrap()
            .model
            .is_none()
    );
    let snapshot = repaired.expand(&mut assets)?;
    assert!(
        comet_application::extract(&snapshot, &mut assets, &[])?
            .iter()
            .any(|draw| draw.entity == target_id)
    );
    assert_eq!(editor.document.history_len(), 1);
    editor.save(&mut assets)?;
    let path = editor.project_root().join("assets/main.scene");
    assert_eq!(Document::load(&path)?.shared_source(), repaired);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), repaired);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn wrong_kind_and_missing_candidate_products_leave_the_accepted_model_preview_intact() -> Result<()>
{
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let model = fixture_model(&mut assets)?;
    let target = EntityData::new("new model owner");
    let id = target.id;
    let mut source = editor.document.source();
    source.base.entities.push(target);
    editor.document = Document::open_source(source)?;
    let before = editor.document.shared_source();
    editor.add_model(id, model.mesh, model.surface.material, &mut assets)?;
    let accepted = editor.document.shared_source();
    let state = editor.document.state_id();
    let revision = editor.document.revision();
    let mut wrong_mesh = model.clone();
    wrong_mesh.mesh = model.surface.material;
    let mut wrong_material = model.clone();
    wrong_material.surface.material = model.mesh;
    let mut missing_mesh = model.clone();
    missing_mesh.mesh = AssetId::new();
    let mut missing_material = model;
    missing_material.surface.material = AssetId::new();
    for candidate in [wrong_mesh, wrong_material, missing_mesh, missing_material] {
        assert!(editor.edit_model(id, Some(candidate), &mut assets).is_err());
        assert!(editor.document.is_editing());
        assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
        assert_eq!(editor.document.state_id(), state);
        assert_eq!(editor.document.revision(), revision);
        assert_eq!(editor.document.history_len(), 0);
    }
    editor.cancel()?;
    assert_eq!(editor.document.shared_source(), before);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn collapsed_geometry_allows_a_model_but_still_requires_valid_candidate_products() -> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let model = fixture_model(&mut assets)?;
    let mut target = EntityData::new("collapsed model owner");
    target.transform.scale = [0.; 3];
    let id = target.id;
    let mut source = editor.document.source();
    source.base.entities.push(target);
    editor.document = Document::open_source(source)?;
    let before = editor.document.shared_source();
    let state = editor.document.state_id();
    let revision = editor.document.revision();
    for (mesh, material) in [
        (model.surface.material, model.surface.material),
        (AssetId::new(), model.surface.material),
        (model.mesh, model.mesh),
        (model.mesh, AssetId::new()),
    ] {
        assert!(editor.add_model(id, mesh, material, &mut assets).is_err());
        assert!(!editor.document.is_editing());
        assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
        assert_eq!(editor.document.state_id(), state);
        assert_eq!(editor.document.revision(), revision);
        assert_eq!(editor.document.history_len(), 0);
        assert!(!editor.document.is_dirty());
    }
    editor.add_model(id, model.mesh, model.surface.material, &mut assets)?;
    editor.commit()?;
    let after = editor.document.shared_source();
    assert_eq!(
        after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .model,
        Some(model)
    );
    let snapshot = after.expand(&mut assets)?;
    assert!(
        comet_application::extract(&snapshot, &mut assets, &[])?
            .iter()
            .all(|draw| draw.entity != id)
    );
    assert_eq!(editor.document.history_len(), 1);
    editor.save(&mut assets)?;
    Ok(())
}
