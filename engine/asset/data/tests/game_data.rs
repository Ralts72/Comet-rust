use anyhow::Result;
use comet_asset_data::{
    game_data::{DataValue, GameData},
    product::{AssetKind, Product},
};
use comet_core::AssetId;
use serde_json::{Value, json};

fn record() -> Value {
    json!({"version":1,"fields":{"hp":{"type":"integer","min":1,"max":100},"rules":{"type":"json"}},
        "rows":{"hero":{"id":"hero","hp":50,"rules":{"nested":[true,"中文",1.25]}}}})
}
#[test]
fn records_roundtrip_canonically_preserve_nested_values_and_only_declared_assets_are_dependencies()
-> Result<()> {
    let empty: GameData = serde_json::from_value(json!({"version":1,"fields":{},"rows":{}}))?;
    assert!(empty.rows.is_empty());
    let id = AssetId::new();
    let mut source = record();
    source["fields"]["next"] = json!({"type":"asset","asset_type":"data"});
    source["rows"]["hero"]["next"] = json!(id);
    source["rows"]["hero"]["rules"]["uuid_is_only_text"] = json!(AssetId::new());
    let data: GameData = serde_json::from_value(source)?;
    assert_eq!(data.dependencies(), [id].into());
    let product = Product::Data(data.clone());
    product.validate_dependency_kinds(|_| Some(AssetKind::Data))?;
    assert!(
        product
            .validate_dependency_kinds(|_| Some(AssetKind::Prefab))
            .is_err()
    );
    let bytes = serde_json::to_vec(&product)?;
    let roundtrip: Product = serde_json::from_slice(&bytes)?;
    assert_eq!(roundtrip, product);
    assert_eq!(serde_json::to_vec(&roundtrip)?, bytes);
    assert_eq!(data.rows["hero"]["hp"], DataValue::Integer(50));
    Ok(())
}
#[test]
fn malformed_records_duplicate_keys_and_exhausted_limits_never_decode_as_defaults() -> Result<()> {
    for (pointer, value) in [
        ("/version", json!(2)),
        ("/rows/hero/hp", json!(0)),
        ("/rows/hero/id", json!("different")),
        ("/rows/hero/rules", Value::Null),
        ("/fields/hp/min", json!(101)),
        ("/fields/hp/unknown", json!(true)),
        ("/rows/hero/extra", json!(1)),
        ("/rows/hero/hp", json!(9_007_199_254_740_992_i64)),
        ("/rows/hero/rules", json!("x".repeat(4097))),
    ] {
        let mut source = record();
        if let Some(slot) = source.pointer_mut(pointer) {
            *slot = value;
        } else {
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            source.pointer_mut(parent).unwrap()[key] = value;
        }
        assert!(
            serde_json::from_value::<GameData>(source).is_err(),
            "{pointer}"
        );
    }
    let bytes = serde_json::to_string(&record())?;
    for duplicate in [
        bytes.replace("\"hp\":50", "\"hp\":50,\"hp\":40"),
        bytes.replace("\"version\":1", "\"version\":1,\"version\":1"),
    ] {
        assert!(serde_json::from_str::<GameData>(&duplicate).is_err());
    }
    let mut source = record();
    let mut deep = json!(true);
    for _ in 0..18 {
        deep = json!([deep]);
    }
    source["rows"]["hero"]["rules"] = deep;
    assert!(serde_json::from_value::<GameData>(source).is_err());
    let mut data: GameData = serde_json::from_value(record())?;
    data.rows.get_mut("hero").unwrap().insert(
        "rules".into(),
        DataValue::Array(vec![DataValue::Array(vec![DataValue::Bool(true); 1024]); 4]),
    );
    assert!(data.validate().is_err());
    data.rows.get_mut("hero").unwrap().insert(
        "rules".into(),
        DataValue::Array(vec![DataValue::Text("x".repeat(2048)); 129]),
    );
    assert!(data.validate().is_err());
    Ok(())
}
