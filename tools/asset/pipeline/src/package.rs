use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::{
    package::{PACKAGE_VERSION, PackageManifest, RuntimeIdentity},
    product::{AssetKind, ProductIndex},
};
use comet_core::{AssetId, DirectoryCandidate, atomic_write, resolve_destination};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub struct PackageRequest<'a> {
    pub products: &'a Path,
    pub index_revision: Option<&'a str>,
    pub entry_scene: AssetId,
    pub additional_scenes: &'a [AssetId],
    pub player: &'a Path,
    pub runtime: Option<RuntimeIdentity>,
    pub player_digest: Option<&'a str>,
    pub notices: &'a [crate::PackageNotice],
    pub output: &'a Path,
}
#[derive(Clone, Debug)]
pub struct PackageOutput {
    pub root: PathBuf,
    pub executable: PathBuf,
    pub products: PathBuf,
}
/// Low-level packing for an already-built binary. Editor exports use a verified PlayerTemplate.
pub fn package(products: &Path, scene: AssetId, player: &Path, output: &Path) -> Result<()> {
    package_scenes(PackageRequest {
        products,
        index_revision: None,
        entry_scene: scene,
        additional_scenes: &[],
        player,
        runtime: None,
        player_digest: None,
        notices: &[],
        output,
    })?;
    Ok(())
}
pub fn package_scenes(request: PackageRequest<'_>) -> Result<PackageOutput> {
    let PackageRequest {
        products,
        index_revision,
        entry_scene,
        additional_scenes,
        player,
        runtime,
        player_digest,
        notices,
        output,
    } = request;
    ensure!(
        player.is_file(),
        "Player executable does not exist: {}",
        player.display()
    );
    let output = resolve_destination(output)?;
    let products = products.canonicalize()?;
    let player = player.canonicalize()?;
    ensure!(
        !output.starts_with(&products)
            && !products.starts_with(&output)
            && !player.starts_with(&output),
        "package destination overlaps its inputs"
    );
    let store = AssetStore::open(&products)?;
    ensure!(
        index_revision.is_none_or(|revision| revision == store.revision()),
        "product index changed before export; retry"
    );
    let scenes: BTreeSet<_> = std::iter::once(entry_scene)
        .chain(additional_scenes.iter().copied())
        .collect();
    let mut closure = BTreeSet::new();
    for scene in &scenes {
        ensure!(
            store
                .index()
                .products
                .get(scene)
                .is_some_and(|p| p.kind == AssetKind::Scene),
            "package entry or additional asset {scene} is not a scene"
        );
        closure.extend(store.dependency_closure(*scene)?);
    }
    let candidate = DirectoryCandidate::new(&output)?;
    let bundle = cfg!(target_os = "macos") && output.extension().is_some_and(|e| e == "app");
    let resources = if bundle {
        candidate.path().join("Contents/Resources")
    } else {
        candidate.path().to_owned()
    };
    let programs = if bundle {
        candidate.path().join("Contents/MacOS")
    } else {
        candidate.path().to_owned()
    };
    fs::create_dir_all(&resources)?;
    fs::create_dir_all(&programs)?;
    let index = ProductIndex {
        version: 1,
        products: store
            .index()
            .products
            .iter()
            .filter(|(id, _)| closure.contains(id))
            .map(|(id, e)| (*id, e.clone()))
            .collect(),
    };
    index.validate()?;
    for (id, entry) in &index.products {
        let relative = PathBuf::from("products")
            .join(id.to_string())
            .join(format!("{}.json", entry.digest));
        let destination = resources.join("products").join(&relative);
        fs::create_dir_all(destination.parent().context("product destination")?)?;
        fs::copy(products.join(&relative), destination)?;
    }
    atomic_write(
        &resources.join("products/index.json"),
        &serde_json::to_vec_pretty(&index)?,
    )?;
    AssetStore::open(resources.join("products"))?;
    let executable = if cfg!(windows) {
        "comet-player.exe"
    } else {
        "comet-player"
    };
    fs::copy(&player, programs.join(executable))?;
    if let Some(expected) = player_digest {
        ensure!(
            comet_core::digest(&fs::read(programs.join(executable))?) == expected,
            "Player changed while exporting; run ./start.sh"
        );
    }
    for notice in notices {
        let components: Vec<_> = Path::new(&notice.name).components().collect();
        ensure!(
            matches!(components.as_slice(), [std::path::Component::Normal(_)])
                && !notice.name.contains(['\\', ':']),
            "invalid package notice name"
        );
        let destination = resources.join("licenses").join(&notice.name);
        fs::create_dir_all(destination.parent().context("notice destination")?)?;
        fs::copy(&notice.path, &destination)?;
        ensure!(
            comet_core::digest(&fs::read(destination)?) == notice.sha256,
            "program notice changed while packaging"
        );
    }
    let manifest = PackageManifest {
        version: PACKAGE_VERSION,
        entry_scene,
        scenes: scenes.into_iter().collect(),
        executable: executable.into(),
        runtime,
    };
    manifest.validate()?;
    atomic_write(
        &resources.join("package.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    if bundle {
        write_bundle(candidate.path(), &output, entry_scene, executable)?;
    }
    let root = candidate.publish()?;
    Ok(PackageOutput {
        executable: if bundle {
            root.join("Contents/MacOS").join(executable)
        } else {
            root.join(executable)
        },
        products: if bundle {
            root.join("Contents/Resources/products")
        } else {
            root.join("products")
        },
        root,
    })
}
fn write_bundle(candidate: &Path, output: &Path, scene: AssetId, executable: &str) -> Result<()> {
    let name = output.file_stem().context("App name")?.to_string_lossy();
    let name = name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>{executable}</string>
<key>CFBundleName</key><string>{name}</string>
<key>CFBundleDisplayName</key><string>{name}</string>
<key>CFBundleIdentifier</key><string>org.comet.game.{scene}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#
    );
    atomic_write(&candidate.join("Contents/Info.plist"), plist.as_bytes())?;
    #[cfg(target_os = "macos")]
    for args in [
        vec!["--force", "--sign", "-"],
        vec!["--verify", "--deep", "--strict"],
    ] {
        let result = std::process::Command::new("/usr/bin/codesign")
            .args(args)
            .arg(candidate)
            .output()
            .context("sign exported App")?;
        ensure!(
            result.status.success(),
            "App signature failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(())
}
