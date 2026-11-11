//! Optional editor user state; never enabled by constructors used for automated runs.
use anyhow::{Context, Result, ensure};
use comet_core::{ProjectPaths, atomic_write};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::File,
    path::{Component, Path, PathBuf},
};

const VERSION: u64 = 1;
const MAX_ENTRIES: usize = 10;
const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecentProject {
    pub(crate) root: PathBuf,
    pub(crate) source: PathBuf,
}

#[derive(Default)]
pub(crate) struct RecentProjects {
    storage: Option<PathBuf>,
    entries: Vec<RecentProject>,
}
impl RecentProjects {
    pub(crate) fn user() -> Result<Self> {
        Self::load(user_path(std::env::consts::OS, |name| {
            std::env::var_os(name)
        })?)
    }

    pub(crate) fn load(storage: PathBuf) -> Result<Self> {
        check_storage(&storage)?;
        let file = match File::open(&storage) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    storage: Some(storage),
                    entries: Vec::new(),
                });
            }
            Err(error) => return Err(error).context("read recent projects"),
        };
        let bytes = comet_core::read_bounded(file, MAX_FILE_BYTES)
            .context("read recent projects within 64 KiB")?;
        let entries = decode(&bytes).context("invalid recent projects configuration")?;
        Ok(Self {
            storage: Some(storage),
            entries,
        })
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.storage.is_some()
    }
    pub(crate) fn entries(&self) -> &[RecentProject] {
        &self.entries
    }

    /// Call only after accepting a document. A preference failure cannot undo that acceptance.
    pub(crate) fn record(&mut self, paths: &ProjectPaths, source: &Path) -> Result<()> {
        if self.storage.is_none() {
            return Ok(());
        }
        ensure!(
            source.is_absolute(),
            "recent document must be an absolute accepted source"
        );
        let source = paths.resolve_asset_path(source)?;
        ensure!(
            source.is_file(),
            "recent document must be an existing author file"
        );
        let entry = RecentProject {
            root: paths.root().to_owned(),
            source: source
                .strip_prefix(paths.assets())
                .context("recent document is outside assets")?
                .to_owned(),
        };
        entry.validate()?;
        if self.entries.first() == Some(&entry) {
            return Ok(());
        }
        let mut candidate = Vec::with_capacity(MAX_ENTRIES);
        candidate.push(entry);
        candidate.extend(
            self.entries
                .iter()
                .filter(|entry| entry.root.as_path() != paths.root())
                .take(MAX_ENTRIES - 1)
                .cloned(),
        );
        self.publish(candidate)
    }

    /// Removing a missing project does not need to resolve or visit its old directory.
    pub(crate) fn remove(&mut self, root: &Path) -> Result<()> {
        if self.storage.is_none() || !self.entries.iter().any(|entry| entry.root == root) {
            return Ok(());
        }
        self.publish(
            self.entries
                .iter()
                .filter(|entry| entry.root != root)
                .cloned()
                .collect(),
        )
    }

    fn publish(&mut self, candidate: Vec<RecentProject>) -> Result<()> {
        let storage = self
            .storage
            .as_ref()
            .context("recent projects are disabled")?;
        ensure!(
            candidate.len() <= MAX_ENTRIES,
            "recent projects exceed the entry budget"
        );
        let projects = candidate
            .iter()
            .map(|entry| {
                entry.validate()?;
                let root = entry
                    .root
                    .to_str()
                    .context("recent project path is not UTF-8")?;
                let source = entry
                    .source
                    .to_str()
                    .context("recent document path is not UTF-8")?;
                Ok(json!({"root": root, "source": source}))
            })
            .collect::<Result<Vec<_>>>()?;
        let bytes = serde_json::to_vec_pretty(&json!({"version":VERSION,"projects":projects}))?;
        ensure!(
            bytes.len() <= MAX_FILE_BYTES,
            "recent projects encoding exceeds 64 KiB"
        );
        check_storage(storage)?;
        atomic_write(storage, &bytes).context("save recent projects")?;
        self.entries = candidate;
        Ok(())
    }
}

impl RecentProject {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.root.is_absolute()
                && !self
                    .root
                    .components()
                    .any(|part| matches!(part, Component::ParentDir)),
            "recent project root must be absolute without parent traversal"
        );
        ensure!(
            !self.source.as_os_str().is_empty()
                && self
                    .source
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "recent document must stay relative to assets without traversal"
        );
        ensure!(
            self.source
                .extension()
                .is_some_and(|ext| ext == "scene" || ext == "prefab"),
            "recent document must be a scene or native Prefab"
        );
        for path in [&self.root, &self.source] {
            let text = path.to_str().context("recent path is not UTF-8")?;
            ensure!(!text.contains('\0'), "recent path contains NUL");
        }
        Ok(())
    }
}

fn check_storage(storage: &Path) -> Result<()> {
    crate::preferences::check_file(storage)
}

fn decode(bytes: &[u8]) -> Result<Vec<RecentProject>> {
    let value: Value = serde_json::from_slice(bytes)?;
    let object = value
        .as_object()
        .context("recent projects must be an object")?;
    ensure!(
        object.len() == 2
            && object
                .keys()
                .all(|key| matches!(key.as_str(), "version" | "projects")),
        "unknown or missing recent projects field"
    );
    ensure!(
        value["version"].as_u64() == Some(VERSION),
        "unsupported recent projects version"
    );
    let projects = value["projects"]
        .as_array()
        .context("recent projects must be an array")?;
    ensure!(
        projects.len() <= MAX_ENTRIES,
        "recent projects exceed the entry budget"
    );
    let mut seen = BTreeSet::new();
    let mut entries = Vec::with_capacity(projects.len());
    for value in projects {
        let object = value
            .as_object()
            .context("recent project must be an object")?;
        ensure!(
            object.len() == 2
                && object
                    .keys()
                    .all(|key| matches!(key.as_str(), "root" | "source")),
            "unknown or missing recent project field"
        );
        let entry = RecentProject {
            root: PathBuf::from(
                value["root"]
                    .as_str()
                    .context("recent project root must be text")?,
            ),
            source: PathBuf::from(
                value["source"]
                    .as_str()
                    .context("recent document must be text")?,
            ),
        };
        entry.validate()?;
        ensure!(
            seen.insert(entry.root.clone()),
            "duplicate recent project root"
        );
        entries.push(entry);
    }
    Ok(entries)
}

fn user_path(os: &str, get: impl FnMut(&str) -> Option<OsString>) -> Result<PathBuf> {
    Ok(crate::preferences::directory(os, get)?.join("recent-projects.json"))
}

#[cfg(test)]
mod tests;
