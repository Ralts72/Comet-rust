//! The authoring fields/rows array is normalized into the CPU record protocol.
use anyhow::{Context, Result, ensure};
use comet_asset_data::game_data::{DataField, DataValue, GameData, MAX_DATA_BYTES};
use std::collections::BTreeMap;

pub(super) fn import(bytes: &[u8]) -> Result<GameData> {
    ensure!(bytes.len() <= MAX_DATA_BYTES, "data source exceeds 256 KiB");
    let DataValue::Object(mut source) =
        serde_json::from_slice(bytes).context("parse data source")?
    else {
        anyhow::bail!("data source must be an object")
    };
    let Some(DataValue::Object(fields)) = source.remove("fields") else {
        anyhow::bail!("data source needs fields object")
    };
    let Some(DataValue::Array(rows)) = source.remove("rows") else {
        anyhow::bail!("data source needs rows array")
    };
    ensure!(source.is_empty(), "unknown data source fields");
    let fields = fields
        .into_iter()
        .map(|(key, value)| Ok((key, DataField::from_spec(value)?)))
        .collect::<Result<_>>()?;
    let mut records = BTreeMap::new();
    for row in rows {
        let DataValue::Object(row) = row else {
            anyhow::bail!("data row must be an object")
        };
        let Some(DataValue::Text(id)) = row.get("id") else {
            anyhow::bail!("data row needs string id")
        };
        ensure!(
            records.insert(id.clone(), row).is_none(),
            "duplicate data row id"
        );
    }
    let value = GameData {
        version: 1,
        fields,
        rows: records,
    };
    value.validate()?;
    Ok(value)
}
