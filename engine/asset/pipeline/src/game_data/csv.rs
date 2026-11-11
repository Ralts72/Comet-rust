//! CSV framing only; field types and record rules use the same protocol as JSON.
use anyhow::{Context, Result, ensure};
use comet_asset_data::game_data::{
    DataField, DataValue, MAX_DATA_BYTES, MAX_DATA_FIELDS, MAX_DATA_RECORDS,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn records(
    bytes: &[u8],
    fields: &BTreeMap<String, DataField>,
) -> Result<Vec<DataValue>> {
    ensure!(bytes.len() <= MAX_DATA_BYTES, "CSV source exceeds 256 KiB");
    let text = std::str::from_utf8(bytes).context("CSV must be UTF-8")?;
    let rows = rows(text.strip_prefix('\u{feff}').unwrap_or(text).as_bytes())?;
    let header = rows.first().context("CSV header is missing")?;
    let names: BTreeSet<_> = header.iter().map(String::as_str).collect();
    ensure!(
        names.len() == header.len()
            && names.contains("id")
            && header.len() == fields.len() + 1
            && names
                .iter()
                .all(|key| *key == "id" || fields.contains_key(*key)),
        "CSV header must contain id and all fields exactly once"
    );
    rows.iter()
        .skip(1)
        .enumerate()
        .map(|(index, row)| {
            ensure!(
                row.len() == header.len(),
                "CSV record {} has the wrong column count",
                index + 1
            );
            let record = header
                .iter()
                .zip(row)
                .map(|(key, cell)| {
                    let value = if key == "id"
                        || matches!(
                            fields.get(key),
                            Some(DataField::String | DataField::Asset { .. })
                        ) {
                        DataValue::Text(cell.clone())
                    } else {
                        serde_json::from_str(cell)
                            .with_context(|| format!("CSV record {} field {key}", index + 1))?
                    };
                    Ok((key.clone(), value))
                })
                .collect::<Result<_>>()?;
            Ok(DataValue::Object(record))
        })
        .collect()
}

fn rows(bytes: &[u8]) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let mut cell = Vec::new();
        if bytes[cursor] == b'"' {
            cursor += 1;
            loop {
                let value = *bytes
                    .get(cursor)
                    .context("CSV has an unclosed quoted cell")?;
                cursor += 1;
                if value != b'"' {
                    cell.push(value);
                } else if bytes.get(cursor) == Some(&b'"') {
                    cell.push(b'"');
                    cursor += 1;
                } else {
                    break;
                }
            }
        } else {
            while let Some(&value) = bytes.get(cursor) {
                if matches!(value, b',' | b'\r' | b'\n') {
                    break;
                }
                ensure!(value != b'"', "CSV quote must start a cell");
                cell.push(value);
                cursor += 1;
            }
        }
        row.push(String::from_utf8(cell).context("CSV cell must be UTF-8")?);
        ensure!(
            row.len() <= MAX_DATA_FIELDS + 1,
            "CSV exceeds column budget"
        );
        let separator = bytes.get(cursor).copied();
        cursor += usize::from(separator.is_some());
        if separator == Some(b',') {
            if cursor < bytes.len() {
                continue;
            }
            row.push(String::new());
            ensure!(
                row.len() <= MAX_DATA_FIELDS + 1,
                "CSV exceeds column budget"
            );
        } else {
            ensure!(
                separator.is_none() || matches!(separator, Some(b'\r' | b'\n')),
                "unexpected text after CSV quote"
            );
            if separator == Some(b'\r') && bytes.get(cursor) == Some(&b'\n') {
                cursor += 1;
            }
        }
        ensure!(rows.len() <= MAX_DATA_RECORDS, "CSV exceeds record budget");
        rows.push(std::mem::take(&mut row));
    }
    Ok(rows)
}
