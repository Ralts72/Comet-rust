//! Versioned read-only project records. Mutable gameplay state belongs to a runtime instance.
use crate::product::AssetKind;
use anyhow::{Context, Result, ensure};
use comet_core::AssetId;
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

pub const MAX_DATA_BYTES: usize = 256 * 1024;
pub const MAX_DATA_NODES: usize = 4096;
pub const MAX_DATA_DEPTH: usize = 16;
pub const MAX_DATA_RECORDS: usize = 256;
pub const MAX_DATA_FIELDS: usize = 32;
pub const MAX_DATA_TEXT: usize = 4096;
pub const MAX_EXACT_INTEGER: i64 = 9_007_199_254_740_991;
pub type DataRecord = BTreeMap<String, DataValue>;
pub type DataLibrary = BTreeMap<AssetId, Arc<GameData>>;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DataValue {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    Array(Vec<DataValue>),
    Object(DataRecord),
}
fn name(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
impl<'de> Deserialize<'de> for DataValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Value;
        impl<'de> de::Visitor<'de> for Value {
            type Value = DataValue;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bounded data value without null or duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<DataValue, E> {
                Ok(DataValue::Bool(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<DataValue, E> {
                if v.unsigned_abs() > MAX_EXACT_INTEGER as u64 {
                    return Err(E::custom("data integer exceeds exact numeric range"));
                }
                Ok(DataValue::Integer(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<DataValue, E> {
                let v = i64::try_from(v)
                    .map_err(|_| E::custom("data integer exceeds exact numeric range"))?;
                self.visit_i64(v)
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<DataValue, E> {
                if !v.is_finite() {
                    return Err(E::custom("data number must be finite"));
                }
                Ok(DataValue::Number(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<DataValue, E> {
                if v.len() > MAX_DATA_TEXT || v.contains('\0') {
                    return Err(E::custom("data text exceeds its budget or contains NUL"));
                }
                Ok(DataValue::Text(v.into()))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<DataValue, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    if values.len() == 1024 {
                        return Err(de::Error::custom("data array exceeds 1024 elements"));
                    }
                    values.push(value);
                }
                Ok(DataValue::Array(values))
            }
            fn visit_map<A: de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<DataValue, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !name(&key) || values.len() == 256 || values.contains_key(&key) {
                        return Err(de::Error::custom(
                            "invalid, duplicate or excessive data object key",
                        ));
                    }
                    values.insert(key, map.next_value()?);
                }
                Ok(DataValue::Object(values))
            }
        }
        deserializer.deserialize_any(Value)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataField {
    String,
    Bool,
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Json,
    Asset {
        asset_type: AssetKind,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameData {
    pub version: u32,
    pub fields: BTreeMap<String, DataField>,
    pub rows: BTreeMap<String, DataRecord>,
}
impl<'de> Deserialize<'de> for GameData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Self::decode(DataValue::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}
fn object(value: DataValue) -> Result<DataRecord> {
    let DataValue::Object(value) = value else {
        anyhow::bail!("expected data object")
    };
    Ok(value)
}
fn take(values: &mut DataRecord, key: &str) -> Result<DataValue> {
    values
        .remove(key)
        .with_context(|| format!("missing data {key}"))
}
impl GameData {
    fn decode(value: DataValue) -> Result<Self> {
        let mut root = object(value)?;
        ensure!(
            take(&mut root, "version")? == DataValue::Integer(1),
            "unsupported data version"
        );
        let fields = object(take(&mut root, "fields")?)?
            .into_iter()
            .map(|(key, value)| Ok((key, DataField::from_spec(value)?)))
            .collect::<Result<BTreeMap<String, DataField>>>()?;
        let rows_value = take(&mut root, "rows")?;
        ensure!(root.is_empty(), "unknown data source fields");
        let rows = object(rows_value)?
            .into_iter()
            .map(|(id, row)| Ok((id, object(row)?)))
            .collect::<Result<_>>()?;
        let value = Self {
            version: 1,
            fields,
            rows,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported data version");
        ensure!(
            self.fields.len() <= MAX_DATA_FIELDS,
            "data field budget exceeded"
        );
        ensure!(
            self.rows.len() <= MAX_DATA_RECORDS,
            "data row budget exceeded"
        );
        for (key, field) in &self.fields {
            ensure!(name(key) && key != "id", "invalid data field name");
            match field {
                DataField::Integer { min, max } | DataField::Number { min, max } => {
                    ensure!(
                        min.iter().chain(max).all(|v| v.is_finite())
                            && min.zip(*max).is_none_or(|(a, b)| a <= b),
                        "invalid data field bounds"
                    );
                }
                DataField::Asset { asset_type } => ensure!(
                    *asset_type != AssetKind::Scene,
                    "data asset fields cannot reference scenes"
                ),
                _ => {}
            }
        }
        let mut remaining = MAX_DATA_NODES;
        let mut bytes = MAX_DATA_BYTES;
        for (id, row) in &self.rows {
            ensure!(name(id), "invalid data record id");
            ensure!(
                row.get("id") == Some(&DataValue::Text(id.clone())),
                "data record identity mismatch"
            );
            ensure!(
                row.len() == self.fields.len() + 1,
                "data row has missing or unknown fields: {id}"
            );
            for (key, field) in &self.fields {
                let value = row
                    .get(key)
                    .with_context(|| format!("record {id} missing {key}"))?;
                let valid = match (field, value) {
                    (DataField::String, DataValue::Text(_))
                    | (DataField::Bool, DataValue::Bool(_))
                    | (DataField::Json, _) => true,
                    (DataField::Integer { min, max }, DataValue::Integer(n)) => {
                        within(*n as f64, *min, *max)
                    }
                    (DataField::Number { min, max }, DataValue::Integer(n)) => {
                        within(*n as f64, *min, *max)
                    }
                    (DataField::Number { min, max }, DataValue::Number(n)) => {
                        within(*n, *min, *max)
                    }
                    (DataField::Asset { .. }, DataValue::Text(id)) => id.parse::<AssetId>().is_ok(),
                    _ => false,
                };
                ensure!(
                    valid,
                    "record {id} field {key} has invalid type, range or asset identity"
                );
            }
            charge_bytes(&mut bytes, id.len())?;
            for (key, value) in row {
                charge_bytes(&mut bytes, key.len())?;
                validate_value(value, 0, &mut remaining, &mut bytes)?;
            }
        }
        Ok(())
    }
    pub fn references(&self) -> impl Iterator<Item = (AssetId, AssetKind)> + '_ {
        self.rows.values().flat_map(|row| {
            self.fields.iter().filter_map(move |(key, field)| {
                if let DataField::Asset { asset_type } = field {
                    let DataValue::Text(id) = row.get(key)? else {
                        return None;
                    };
                    Some((id.parse().ok()?, *asset_type))
                } else {
                    None
                }
            })
        })
    }
    pub fn dependencies(&self) -> BTreeSet<AssetId> {
        self.references().map(|(id, _)| id).collect()
    }
}
fn within(value: f64, min: Option<f64>, max: Option<f64>) -> bool {
    value.is_finite() && min.is_none_or(|m| value >= m) && max.is_none_or(|m| value <= m)
}
fn validate_value(
    value: &DataValue,
    depth: usize,
    remaining: &mut usize,
    bytes: &mut usize,
) -> Result<()> {
    ensure!(
        depth <= MAX_DATA_DEPTH && *remaining > 0,
        "data depth or node budget exceeded"
    );
    *remaining -= 1;
    match value {
        DataValue::Integer(n) => ensure!(
            n.unsigned_abs() <= MAX_EXACT_INTEGER as u64,
            "data integer exceeds exact numeric range"
        ),
        DataValue::Number(n) => ensure!(n.is_finite(), "data number must be finite"),
        DataValue::Text(s) => {
            ensure!(
                s.len() <= MAX_DATA_TEXT && !s.contains('\0'),
                "data text exceeds its budget or contains NUL"
            );
            charge_bytes(bytes, s.len())?;
        }
        DataValue::Array(values) => {
            ensure!(values.len() <= 1024, "data array budget exceeded");
            for v in values {
                validate_value(v, depth + 1, remaining, bytes)?;
            }
        }
        DataValue::Object(values) => {
            ensure!(
                values.len() <= 256 && values.keys().all(|k| name(k)),
                "data object budget or key invalid"
            );
            for (key, v) in values {
                charge_bytes(bytes, key.len())?;
                validate_value(v, depth + 1, remaining, bytes)?;
            }
        }
        DataValue::Bool(_) => {}
    }
    Ok(())
}

fn charge_bytes(remaining: &mut usize, count: usize) -> Result<()> {
    *remaining = remaining
        .checked_sub(count)
        .context("data text byte budget exceeded")?;
    Ok(())
}
impl DataField {
    pub fn from_spec(value: DataValue) -> Result<Self> {
        let mut spec = object(value)?;
        let DataValue::Text(kind) = take(&mut spec, "type")? else {
            anyhow::bail!("data field type must be text")
        };
        let mut bound = |key| -> Result<Option<f64>> {
            spec.remove(key)
                .map(|v| match v {
                    DataValue::Integer(n) => Ok(n as f64),
                    DataValue::Number(n) => Ok(n),
                    _ => anyhow::bail!("data bounds must be numeric"),
                })
                .transpose()
        };
        let min = bound("min")?;
        let max = bound("max")?;
        let asset = spec.remove("asset_type");
        ensure!(spec.is_empty(), "unknown data field properties");
        ensure!(
            matches!(kind.as_str(), "integer" | "number") || (min.is_none() && max.is_none()),
            "bounds require a numeric data field"
        );
        ensure!(
            kind == "asset" || asset.is_none(),
            "asset_type requires an asset field"
        );
        Ok(match kind.as_str() {
            "string" => Self::String,
            "bool" => Self::Bool,
            "json" => Self::Json,
            "integer" => Self::Integer { min, max },
            "number" => Self::Number { min, max },
            "asset" => {
                let Some(DataValue::Text(kind)) = asset else {
                    anyhow::bail!("asset field requires asset_type")
                };
                Self::Asset {
                    asset_type: AssetKind::deserialize(de::value::StrDeserializer::<
                        de::value::Error,
                    >::new(&kind))?,
                }
            }
            _ => anyhow::bail!("unknown data field type {kind}"),
        })
    }
}
