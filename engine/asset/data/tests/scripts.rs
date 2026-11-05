use anyhow::Result;
use comet_asset_data::{
    package::ProtocolVersions,
    product::{
        AssetKind, INDEX_VERSION, IndexEntry, MAX_SCRIPT_BYTES, PRODUCT_VERSION, Product,
        ProductFile, ProductIndex, SCRIPT_VERSION, ScriptData,
    },
};
use comet_core::{AssetId, EntityId, digest};
use comet_scene::{ContentSource, EntityData, EntityRef, SceneSnapshot, Script, ScriptValue};
use std::collections::BTreeMap;

#[test]
fn script_products_validate_utf8_byte_limits_and_plain_source_only() -> Result<()> {
    for source in [
        "let 中文 = 1;".into(),
        "a".repeat(MAX_SCRIPT_BYTES),
        "λ".repeat(MAX_SCRIPT_BYTES / 2),
    ] {
        let script = ScriptData {
            version: SCRIPT_VERSION,
            source,
        };
        script.validate()?;
        let product = Product::Script(script);
        assert_eq!(product.kind(), AssetKind::Script);
        assert!(product.dependencies().is_empty());
    }
    for source in [
        "".into(),
        " \n\t".into(),
        "let x = \0;".into(),
        "a".repeat(MAX_SCRIPT_BYTES + 1),
        "λ".repeat(MAX_SCRIPT_BYTES / 2 + 1),
    ] {
        assert!(
            ScriptData {
                version: SCRIPT_VERSION,
                source
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        ScriptData {
            version: SCRIPT_VERSION + 1,
            source: "let x = 1;".into()
        }
        .validate()
        .is_err()
    );
    Ok(())
}

#[test]
fn script_bindings_and_external_entity_parameters_require_their_actual_asset_kinds_even_when_disabled()
-> Result<()> {
    let asset = AssetId::new();
    let external = AssetId::new();
    let mut entity = EntityData::new("scripted entity");
    entity.script = Some(Script {
        asset,
        enabled: false,
        parameters: BTreeMap::from([(
            "target".into(),
            ScriptValue::Entity(Some(EntityRef::External {
                scene: external,
                entity: EntityId::new(),
            })),
        )]),
    });
    let product = Product::Scene(ContentSource {
        base: SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(
        product.dependencies(),
        [asset, external].into_iter().collect()
    );
    let mut kinds = BTreeMap::from([(asset, AssetKind::Script), (external, AssetKind::Scene)]);
    product.validate_dependency_kinds(&kinds)?;
    kinds.insert(asset, AssetKind::Material);
    assert!(product.validate_dependency_kinds(&kinds).is_err());
    kinds.insert(asset, AssetKind::Script);
    kinds.insert(external, AssetKind::Prefab);
    assert!(product.validate_dependency_kinds(&kinds).is_err());
    Ok(())
}

#[test]
fn prefab_script_asset_overrides_require_script_products() -> Result<()> {
    use comet_core::InstanceId;
    use comet_scene::{FieldOverride, PrefabInstance, schema};
    let prefab = AssetId::new();
    let script = AssetId::new();
    let product = Product::Scene(ContentSource {
        instances: vec![PrefabInstance {
            id: InstanceId::new(),
            prefab,
            parent: None,
            overrides: vec![FieldOverride {
                entity: EntityId::new(),
                field: schema::SCRIPT_ASSET,
                value: schema::FieldValue::Asset(script),
            }],
        }],
        ..Default::default()
    });
    let mut kinds = BTreeMap::from([(prefab, AssetKind::Prefab), (script, AssetKind::Script)]);
    product.validate_dependency_kinds(&kinds)?;
    kinds.insert(script, AssetKind::Material);
    assert!(product.validate_dependency_kinds(&kinds).is_err());
    Ok(())
}

#[test]
fn new_product_and_index_versions_reject_old_readers_inputs_and_match_template_protocols()
-> Result<()> {
    let id = AssetId::new();
    let mut product = ProductFile {
        version: PRODUCT_VERSION,
        id,
        source_digest: digest(b"source"),
        payload: Product::Script(ScriptData {
            version: SCRIPT_VERSION,
            source: "let x = 1;".into(),
        }),
    };
    product.validate()?;
    product.version = 1;
    assert!(product.validate().is_err());
    let mut index = ProductIndex {
        version: INDEX_VERSION,
        products: BTreeMap::from([(
            id,
            IndexEntry {
                digest: digest(b"product"),
                kind: AssetKind::Script,
                dependencies: Default::default(),
                editor: None,
            },
        )]),
    };
    index.validate()?;
    index.version = 1;
    assert!(index.validate().is_err());
    let protocols = ProtocolVersions::current();
    assert_eq!(protocols.product, PRODUCT_VERSION);
    assert_eq!(protocols.index, INDEX_VERSION);
    Ok(())
}
