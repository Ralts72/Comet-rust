//! A session uses an immutable, verified Player snapshot instead of a mutable Cargo artifact.
use anyhow::{Context, Result, ensure};
use comet_asset_data::package::{PlayerTemplateManifest, RuntimeIdentity};
use comet_core::digest;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct PackageNotice {
    pub name: String,
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Clone)]
pub struct PlayerTemplate {
    manifest: PlayerTemplateManifest,
    player: PathBuf,
    notices: Vec<PackageNotice>,
}
impl PlayerTemplate {
    pub fn open(path: &Path, editor: &Path) -> Result<Self> {
        Self::open_for(path, editor, &RuntimeIdentity::current())
    }
    fn open_for(path: &Path, editor: &Path, expected: &RuntimeIdentity) -> Result<Self> {
        let manifest: PlayerTemplateManifest = serde_json::from_slice(
            &fs::read(path).context("read Player template; run ./start.sh")?,
        )?;
        manifest
            .validate()
            .context("invalid Player template; run ./start.sh")?;
        ensure!(
            &manifest.identity == expected,
            "Player template does not match this Editor; run ./start.sh"
        );
        ensure!(
            digest(&fs::read(editor)?) == manifest.editor_sha256,
            "Player template belongs to another Editor executable; run ./start.sh"
        );
        let root = path
            .parent()
            .context("Player template directory")?
            .canonicalize()?;
        let player = root.join(&manifest.player).canonicalize()?;
        ensure!(
            player.starts_with(&root),
            "Player template escapes its snapshot"
        );
        let mut notices = vec![];
        for (name, sha256) in &manifest.notices {
            let path = root
                .join("licenses")
                .join(name)
                .canonicalize()
                .context("Player template notice is missing; run ./start.sh")?;
            ensure!(
                path.starts_with(&root),
                "Player template notice escapes its snapshot"
            );
            notices.push(PackageNotice {
                name: name.clone(),
                path,
                sha256: sha256.clone(),
            });
        }
        let result = Self {
            manifest,
            player,
            notices,
        };
        result.verify()?;
        Ok(result)
    }
    pub fn verify(&self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                fs::metadata(&self.player)
                    .context("Player template is missing; run ./start.sh")?
                    .permissions()
                    .mode()
                    & 0o111
                    != 0,
                "Player template is not executable; run ./start.sh"
            );
        }
        ensure!(
            digest(&fs::read(&self.player).context("Player template is missing; run ./start.sh")?)
                == self.manifest.player_sha256,
            "Player template changed; run ./start.sh"
        );
        for notice in &self.notices {
            ensure!(
                digest(
                    &fs::read(&notice.path)
                        .context("Player template notice is missing; run ./start.sh")?
                ) == notice.sha256,
                "Player template notice changed; run ./start.sh"
            );
        }
        Ok(())
    }
    pub fn notices(&self) -> &[PackageNotice] {
        &self.notices
    }
    pub fn player_digest(&self) -> &str {
        &self.manifest.player_sha256
    }
    pub fn player(&self) -> &Path {
        &self.player
    }
    pub fn identity(&self) -> &RuntimeIdentity {
        &self.manifest.identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_reject_mismatched_builds_protocols_editor_and_changed_players() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let player = root.join("comet-player");
        let editor = root.join("editor");
        fs::write(&player, b"player snapshot")?;
        fs::write(&editor, b"editor snapshot")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&player, fs::Permissions::from_mode(0o755))?;
        }
        let mut identity = RuntimeIdentity::current();
        identity.build_id = "a".repeat(64);
        fs::create_dir(root.join("licenses"))?;
        fs::write(root.join("licenses/font.txt"), b"font license")?;
        let manifest = PlayerTemplateManifest {
            version: 2,
            identity: identity.clone(),
            profile: "release".into(),
            player: "comet-player".into(),
            player_sha256: digest(&fs::read(&player)?),
            editor_sha256: digest(&fs::read(&editor)?),
            notices: std::collections::BTreeMap::from([(
                "font.txt".into(),
                digest(b"font license"),
            )]),
        };
        let path = root.join("template.json");
        fs::write(&path, serde_json::to_vec(&manifest)?)?;
        let template = PlayerTemplate::open_for(&path, &editor, &identity)?;
        fs::write(root.join("licenses/font.txt"), b"changed license")?;
        assert!(template.verify().is_err());
        fs::write(root.join("licenses/font.txt"), b"font license")?;
        template.verify()?;
        for mutate in 0..5 {
            let mut candidate = manifest.clone();
            match mutate {
                0 => candidate.identity.build_id = "b".repeat(64),
                1 => candidate.identity.arch = "other".into(),
                2 => candidate.identity.protocols.scene += 1,
                3 => candidate.profile = "dev".into(),
                _ => candidate.player = "../outside".into(),
            }
            fs::write(&path, serde_json::to_vec(&candidate)?)?;
            assert!(PlayerTemplate::open_for(&path, &editor, &identity).is_err());
        }
        fs::write(&path, serde_json::to_vec(&manifest)?)?;
        fs::write(&editor, b"another editor")?;
        assert!(PlayerTemplate::open_for(&path, &editor, &identity).is_err());
        fs::write(&player, b"overwritten cargo binary")?;
        assert!(template.verify().is_err());
        fs::remove_file(&player)?;
        assert!(format!("{:#}", template.verify().unwrap_err()).contains("start.sh"));
        Ok(())
    }
}
