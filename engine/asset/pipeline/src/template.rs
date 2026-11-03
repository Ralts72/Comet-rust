//! A session uses an immutable, verified App snapshot instead of a mutable Cargo artifact.
use anyhow::{Context, Result, ensure};
use comet_asset_data::package::{AppTemplateManifest, RuntimeIdentity};
use comet_core::digest;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct PackageFile {
    pub name: String,
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Clone)]
pub struct AppTemplate {
    manifest: AppTemplateManifest,
    app: PathBuf,
    notices: Vec<PackageFile>,
    app_icon: PackageFile,
}
impl AppTemplate {
    pub fn open(path: &Path, editor: &Path) -> Result<Self> {
        Self::open_for(path, editor, &RuntimeIdentity::current())
    }
    fn open_for(path: &Path, editor: &Path, expected: &RuntimeIdentity) -> Result<Self> {
        let manifest: AppTemplateManifest =
            serde_json::from_slice(&fs::read(path).context("read App template; run ./start.sh")?)?;
        manifest
            .validate()
            .context("invalid App template; run ./start.sh")?;
        ensure!(
            &manifest.identity == expected,
            "App template does not match this Editor; run ./start.sh"
        );
        ensure!(
            digest(&fs::read(editor)?) == manifest.editor_sha256,
            "App template belongs to another Editor executable; run ./start.sh"
        );
        let root = path
            .parent()
            .context("App template directory")?
            .canonicalize()?;
        let app = root.join(&manifest.app).canonicalize()?;
        ensure!(app.starts_with(&root), "App template escapes its snapshot");
        let mut notices = vec![];
        for (name, sha256) in &manifest.notices {
            let path = root
                .join("licenses")
                .join(name)
                .canonicalize()
                .context("App template notice is missing; run ./start.sh")?;
            ensure!(
                path.starts_with(&root),
                "App template notice escapes its snapshot"
            );
            notices.push(PackageFile {
                name: name.clone(),
                path,
                sha256: sha256.clone(),
            });
        }
        let icon_path = root
            .join("icons/app.icns")
            .canonicalize()
            .context("App template icon is missing; run ./start.sh")?;
        ensure!(
            icon_path.starts_with(&root),
            "App template icon escapes its snapshot"
        );
        let app_icon = PackageFile {
            name: "app.icns".into(),
            path: icon_path,
            sha256: manifest.app_icon_sha256.clone(),
        };
        let result = Self {
            manifest,
            app,
            notices,
            app_icon,
        };
        result.verify()?;
        Ok(result)
    }
    pub fn verify(&self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                fs::metadata(&self.app)
                    .context("App template is missing; run ./start.sh")?
                    .permissions()
                    .mode()
                    & 0o111
                    != 0,
                "App template is not executable; run ./start.sh"
            );
        }
        ensure!(
            digest(&fs::read(&self.app).context("App template is missing; run ./start.sh")?)
                == self.manifest.app_sha256,
            "App template changed; run ./start.sh"
        );
        for file in self.notices.iter().chain(std::iter::once(&self.app_icon)) {
            ensure!(
                digest(&fs::read(&file.path).with_context(|| format!(
                    "App template file {} is missing; run ./start.sh",
                    file.name
                ))?) == file.sha256,
                "App template file {} changed; run ./start.sh",
                file.name
            );
        }
        Ok(())
    }
    pub fn notices(&self) -> &[PackageFile] {
        &self.notices
    }
    pub fn app_icon(&self) -> &PackageFile {
        &self.app_icon
    }
    pub fn app_digest(&self) -> &str {
        &self.manifest.app_sha256
    }
    pub fn app(&self) -> &Path {
        &self.app
    }
    pub fn identity(&self) -> &RuntimeIdentity {
        &self.manifest.identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_reject_mismatched_builds_protocols_editor_and_changed_apps() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let app = root.join("comet-app");
        let editor = root.join("editor");
        fs::write(&app, b"app snapshot")?;
        fs::write(&editor, b"editor snapshot")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&app, fs::Permissions::from_mode(0o755))?;
        }
        let mut identity = RuntimeIdentity::current();
        identity.build_id = "a".repeat(64);
        fs::create_dir(root.join("licenses"))?;
        fs::write(root.join("licenses/font.txt"), b"font license")?;
        fs::create_dir(root.join("icons"))?;
        fs::write(root.join("icons/app.icns"), b"app icon")?;
        let manifest = AppTemplateManifest {
            version: 4,
            identity: identity.clone(),
            profile: "release".into(),
            app: "comet-app".into(),
            app_sha256: digest(&fs::read(&app)?),
            editor_sha256: digest(&fs::read(&editor)?),
            app_icon_sha256: digest(b"app icon"),
            notices: std::collections::BTreeMap::from([(
                "font.txt".into(),
                digest(b"font license"),
            )]),
        };
        let path = root.join("template.json");
        fs::write(&path, serde_json::to_vec(&manifest)?)?;
        let template = AppTemplate::open_for(&path, &editor, &identity)?;
        fs::write(root.join("icons/app.icns"), b"changed icon")?;
        assert!(template.verify().is_err());
        fs::write(root.join("icons/app.icns"), b"app icon")?;
        fs::write(root.join("licenses/font.txt"), b"changed license")?;
        assert!(template.verify().is_err());
        fs::write(root.join("licenses/font.txt"), b"font license")?;
        template.verify()?;
        for mutate in 0..6 {
            let mut candidate = manifest.clone();
            match mutate {
                0 => candidate.identity.build_id = "b".repeat(64),
                1 => candidate.identity.arch = "other".into(),
                2 => candidate.identity.protocols.scene += 1,
                3 => candidate.profile = "dev".into(),
                4 => candidate.app = "../outside".into(),
                _ => candidate.version = 3,
            }
            fs::write(&path, serde_json::to_vec(&candidate)?)?;
            assert!(AppTemplate::open_for(&path, &editor, &identity).is_err());
        }
        fs::write(&path, serde_json::to_vec(&manifest)?)?;
        fs::write(&editor, b"another editor")?;
        assert!(AppTemplate::open_for(&path, &editor, &identity).is_err());
        fs::write(&app, b"overwritten cargo binary")?;
        assert!(template.verify().is_err());
        fs::remove_file(&app)?;
        assert!(format!("{:#}", template.verify().unwrap_err()).contains("start.sh"));
        Ok(())
    }
}
