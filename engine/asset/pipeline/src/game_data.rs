//! The authoring fields/rows array is normalized into the CPU record protocol.
use anyhow::{Context, Result, ensure};
use comet_asset_data::{
    game_data::{DataField, DataRecord, DataValue, GameData, MAX_DATA_BYTES},
    product::Product,
};
use comet_core::{AssetId, digest};
use std::{collections::BTreeMap, path::Path};

mod csv;
#[cfg(test)]
mod tests;

pub(super) fn import(bytes: &[u8]) -> Result<GameData> {
    ensure!(bytes.len() <= MAX_DATA_BYTES, "data source exceeds 256 KiB");
    let DataValue::Object(mut source) =
        serde_json::from_slice(bytes).context("parse data source")?
    else {
        anyhow::bail!("data source must be an object")
    };
    let fields = fields(&mut source)?;
    let Some(DataValue::Array(rows)) = source.remove("rows") else {
        anyhow::bail!("data source needs rows array")
    };
    ensure!(source.is_empty(), "unknown data source fields");
    normalize(fields, rows)
}

fn fields(source: &mut DataRecord) -> Result<BTreeMap<String, DataField>> {
    let Some(DataValue::Object(fields)) = source.remove("fields") else {
        anyhow::bail!("data source needs fields object")
    };
    fields
        .into_iter()
        .map(|(key, value)| Ok((key, DataField::from_spec(value)?)))
        .collect()
}

fn normalize(fields: BTreeMap<String, DataField>, rows: Vec<DataValue>) -> Result<GameData> {
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

pub(super) fn import_csv(
    path: &Path,
    bytes: &[u8],
    id: AssetId,
) -> Result<crate::import::Imported> {
    let mut schema = path.as_os_str().to_owned();
    schema.push(".schema.json");
    let schema = std::path::PathBuf::from(schema);
    ensure!(
        std::fs::symlink_metadata(&schema)
            .with_context(|| format!("CSV schema {}", schema.display()))?
            .file_type()
            .is_file(),
        "CSV schema must be a regular file"
    );
    let schema = schema.canonicalize()?;
    ensure!(
        schema.starts_with(
            path.parent()
                .context("CSV source directory")?
                .canonicalize()?
        ),
        "CSV schema escapes source directory"
    );
    let schema_bytes = crate::import::read_bounded(&schema, MAX_DATA_BYTES)?;
    let DataValue::Object(mut source) =
        serde_json::from_slice(&schema_bytes).context("parse CSV schema")?
    else {
        anyhow::bail!("CSV schema must be an object")
    };
    let fields = fields(&mut source)?;
    ensure!(source.is_empty(), "unknown CSV schema fields");
    let records = csv::records(bytes, &fields)?;
    let table = normalize(fields, records)?;
    Ok(crate::import::Imported {
        products: vec![crate::import::ImportedProduct {
            id,
            payload: Product::Data(table),
            label: path
                .file_name()
                .context("CSV filename")?
                .to_string_lossy()
                .into_owned(),
        }],
        files: [(
            schema,
            crate::import::SourceFile {
                digest: digest(&schema_bytes),
                byte_limit: MAX_DATA_BYTES,
            },
        )]
        .into(),
    })
}
