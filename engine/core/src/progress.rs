//! Explicit gameplay values, scoped by persistent source and owner identities rather than a VM.
use crate::{AssetId, EntityId};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_PROGRESS_ENTRIES: usize = 128;
pub const MAX_PROGRESS_KEY_BYTES: usize = 128;
pub const MAX_PROGRESS_TEXT_BYTES: usize = 4096;
pub const MAX_PROGRESS_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ProgressValue {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
}
impl ProgressValue {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Number(value) => ensure!(value.is_finite(), "progress number must be finite"),
            Self::Text(value) => ensure!(
                value.len() <= MAX_PROGRESS_TEXT_BYTES && !value.contains('\0'),
                "progress text exceeds its budget or contains NUL"
            ),
            _ => {}
        }
        Ok(())
    }
    pub fn same_type(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressEntry {
    pub scene: AssetId,
    pub owner: EntityId,
    pub key: String,
    pub value: ProgressValue,
}
pub fn validate_progress_key(key: &str) -> Result<()> {
    ensure!(
        !key.trim().is_empty()
            && key.len() <= MAX_PROGRESS_KEY_BYTES
            && !key.chars().any(char::is_control),
        "invalid progress key"
    );
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProgressValues {
    #[serde(deserialize_with = "deserialize_entries")]
    pub entries: Vec<ProgressEntry>,
}
impl ProgressValues {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.entries.len() <= MAX_PROGRESS_ENTRIES,
            "progress entry budget exceeded"
        );
        let mut keys = BTreeSet::new();
        for entry in &self.entries {
            validate_progress_key(&entry.key)?;
            entry.value.validate()?;
            ensure!(
                keys.insert((entry.scene, entry.owner, entry.key.as_str())),
                "duplicate scene/owner progress key"
            );
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_PROGRESS_BYTES,
            "progress encoded byte budget exceeded"
        );
        Ok(())
    }
    pub fn canonicalize(&mut self) {
        self.entries
            .sort_by(|a, b| (a.scene, a.owner, &a.key).cmp(&(b.scene, b.owner, &b.key)));
    }
    pub fn get(&self, scene: AssetId, owner: EntityId, key: &str) -> Option<&ProgressValue> {
        self.entries
            .iter()
            .find(|entry| entry.scene == scene && entry.owner == owner && entry.key == key)
            .map(|entry| &entry.value)
    }
}
fn deserialize_entries<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> std::result::Result<Vec<ProgressEntry>, D::Error> {
    struct Entries;
    impl<'de> serde::de::Visitor<'de> for Entries {
        type Value = Vec<ProgressEntry>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {MAX_PROGRESS_ENTRIES} progress entries")
        }
        fn visit_seq<S: serde::de::SeqAccess<'de>>(
            self,
            mut seq: S,
        ) -> std::result::Result<Self::Value, S::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = seq.next_element()? {
                if entries.len() == MAX_PROGRESS_ENTRIES {
                    return Err(serde::de::Error::custom("progress entry budget exceeded"));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    value.deserialize_seq(Entries)
}
