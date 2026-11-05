use anyhow::Result;
use comet_asset_data::product::{
    AssetKind, EditorProvenance, INDEX_VERSION, IndexEntry, MAX_EDITOR_LABEL_BYTES, ProductIndex,
};
use comet_core::{AssetId, digest};
use std::collections::BTreeMap;

#[test]
fn editor_provenance_is_optional_bounded_and_separate_from_runtime_dependencies() -> Result<()> {
    let root = AssetId::new();
    let mesh = AssetId::new();
    let entry = |source, label: &str, kind| IndexEntry {
        digest: digest(label.as_bytes()),
        kind,
        dependencies: Default::default(),
        editor: Some(EditorProvenance {
            source,
            label: label.into(),
        }),
    };
    let mut index = ProductIndex {
        version: INDEX_VERSION,
        products: BTreeMap::from([
            (root, entry(root, "models/示例.gltf", AssetKind::Prefab)),
            (mesh, entry(root, "三角网格 / primitive 0", AssetKind::Mesh)),
        ]),
    };
    index.validate()?;
    assert!(index.products[&mesh].dependencies.is_empty());
    let product = index.products[&mesh].clone();
    index
        .products
        .get_mut(&mesh)
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .label = "🛸".repeat(MAX_EDITOR_LABEL_BYTES / 4);
    index.validate()?;
    assert!(product.same_product(&index.products[&mesh]));
    for label in [
        "".into(),
        " \n".into(),
        "bad\0label".into(),
        "🛸".repeat(MAX_EDITOR_LABEL_BYTES / 4 + 1),
    ] {
        index
            .products
            .get_mut(&mesh)
            .unwrap()
            .editor
            .as_mut()
            .unwrap()
            .label = label;
        assert!(index.validate().is_err());
    }
    index.products.insert(mesh, product);
    index
        .products
        .get_mut(&mesh)
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .source = AssetId::new();
    assert!(index.validate().is_err());
    index
        .products
        .get_mut(&mesh)
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .source = root;
    index
        .products
        .get_mut(&root)
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .source = mesh;
    assert!(
        index.validate().is_err(),
        "source entries must be roots, not a provenance cycle"
    );
    for entry in index.products.values_mut() {
        entry.editor = None;
    }
    index.validate()?;
    index.version = 2;
    assert!(index.validate().is_err());
    Ok(())
}
