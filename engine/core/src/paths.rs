//! Project authoring paths share one canonical boundary for reads and writes.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct ProjectPaths {
    root: PathBuf,
    assets: PathBuf,
}
impl ProjectPaths {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root
            .as_ref()
            .canonicalize()
            .context("resolve project directory")?;
        let assets = assets_directory(&root)?;
        Ok(Self { root, assets })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn assets(&self) -> &Path {
        &self.assets
    }
    /// Existing paths follow symlinks. New paths resolve their existing ancestors first.
    pub fn resolve_asset_path(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        self.check_assets_directory()?;
        let path = path.as_ref();
        ensure!(!path.as_os_str().is_empty(), "asset path cannot be empty");
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            self.assets.join(path)
        };
        let resolved = resolve_destination(&path)?;
        ensure!(
            resolved != self.assets && resolved.starts_with(&self.assets),
            "path is outside project assets: {}",
            path.display()
        );
        Ok(resolved)
    }
    pub fn export_destination(&self, output: impl AsRef<Path>, products: &Path) -> Result<PathBuf> {
        self.check_assets_directory()?;
        let output = resolve_destination(output.as_ref())?;
        let products = resolve_destination(products)?;
        // Reject both nesting directions: publishing must not capture or replace its inputs.
        for reserved in [&self.assets, &products] {
            ensure!(
                !output.starts_with(reserved) && !reserved.starts_with(&output),
                "export destination overlaps an input directory: {}",
                output.display()
            );
        }
        Ok(output)
    }
    fn check_assets_directory(&self) -> Result<()> {
        // An assets alias may have been replaced since the project was opened.
        ensure!(
            assets_directory(&self.root)? == self.assets,
            "project assets directory changed; reopen the project"
        );
        Ok(())
    }
}

fn assets_directory(root: &Path) -> Result<PathBuf> {
    let assets = root
        .join("assets")
        .canonicalize()
        .context("resolve project assets")?;
    ensure!(assets.is_dir(), "project assets must be a directory");
    ensure!(
        assets != root && assets.starts_with(root),
        "project assets must remain inside the project directory"
    );
    Ok(assets)
}

/// Resolve a destination without requiring its final components to exist.
pub fn resolve_destination(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path).context("resolve destination")?;
    resolve_absolute(&absolute)
}
fn resolve_absolute(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => path
            .canonicalize()
            .with_context(|| format!("resolve {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .context("destination requires a file name")?;
            let parent = path.parent().context("destination requires a parent")?;
            Ok(resolve_absolute(parent)?.join(name))
        }
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}
