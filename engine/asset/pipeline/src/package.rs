use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::{
    package::{PACKAGE_VERSION, PackageManifest, RuntimeIdentity},
    product::{AssetKind, INDEX_VERSION, ProductIndex},
};
use comet_core::{AssetId, DirectoryCandidate, atomic_write, resolve_destination};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub struct PackageRequest<'a> {
    pub products: &'a Path,
    pub game_id: comet_core::GameId,
    pub index_revision: Option<&'a str>,
    pub entry_scene: AssetId,
    pub additional_scenes: &'a [AssetId],
    pub app: &'a Path,
    pub runtime: Option<RuntimeIdentity>,
    pub app_digest: Option<&'a str>,
    pub notices: &'a [crate::PackageFile],
    pub app_icon: Option<&'a crate::PackageFile>,
    pub game_icon: Option<AssetId>,
    pub output: &'a Path,
}
#[derive(Clone, Debug)]
pub struct PackageOutput {
    pub root: PathBuf,
    pub executable: PathBuf,
    pub products: PathBuf,
}
/// Low-level packing for an already-built binary. Editor exports use a verified AppTemplate.
pub fn package(
    products: &Path,
    game_id: comet_core::GameId,
    scene: AssetId,
    app: &Path,
    output: &Path,
) -> Result<()> {
    package_scenes(PackageRequest {
        products,
        game_id,
        index_revision: None,
        entry_scene: scene,
        additional_scenes: &[],
        app,
        runtime: None,
        app_digest: None,
        notices: &[],
        app_icon: None,
        game_icon: None,
        output,
    })?;
    Ok(())
}
pub fn package_scenes(request: PackageRequest<'_>) -> Result<PackageOutput> {
    let PackageRequest {
        products,
        game_id,
        index_revision,
        entry_scene,
        additional_scenes,
        app,
        runtime,
        app_digest,
        notices,
        app_icon,
        game_icon,
        output,
    } = request;
    ensure!(
        app.is_file(),
        "App executable does not exist: {}",
        app.display()
    );
    let output = resolve_destination(output)?;
    let products = products.canonicalize()?;
    let app = app.canonicalize()?;
    ensure!(
        !output.starts_with(&products)
            && !products.starts_with(&output)
            && !app.starts_with(&output),
        "package destination overlaps its inputs"
    );
    let mut store = AssetStore::open(&products)?;
    let game_icon = game_icon
        .map(|id| crate::icon::GameIcon::load(&mut store, id))
        .transpose()?;
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
        version: INDEX_VERSION,
        products: store
            .index()
            .products
            .iter()
            .filter(|(id, _)| closure.contains(id))
            .map(|(id, entry)| {
                let mut entry = entry.clone();
                entry.editor = None;
                (*id, entry)
            })
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
        "comet-app.exe"
    } else {
        "comet-app"
    };
    fs::copy(&app, programs.join(executable))?;
    if let Some(expected) = app_digest {
        ensure!(
            comet_core::digest(&fs::read(programs.join(executable))?) == expected,
            "App changed while exporting; run ./start.sh"
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
    if let Some(icon) = &game_icon {
        atomic_write(&resources.join("game-icon.png"), &icon.png)?;
        atomic_write(&resources.join("app.icns"), &icon.icns)?;
        #[cfg(windows)]
        icon.embed(&programs.join(executable))?;
    } else if let Some(icon) = app_icon {
        let bytes = fs::read(&icon.path).context("read package icon")?;
        ensure!(
            comet_core::digest(&bytes) == icon.sha256,
            "application icon changed while packaging"
        );
        ensure!(
            bytes.starts_with(b"icns")
                && bytes.len() >= 8
                && u32::from_be_bytes(bytes[4..8].try_into()?) as usize == bytes.len(),
            "invalid ICNS application icon"
        );
        atomic_write(&resources.join("app.icns"), &bytes)?;
    }
    let manifest = PackageManifest {
        version: PACKAGE_VERSION,
        game_id,
        entry_scene,
        scenes: scenes.into_iter().collect(),
        executable: executable.into(),
        runtime,
        game_icon_sha256: game_icon.as_ref().map(|icon| comet_core::digest(&icon.png)),
    };
    manifest.validate()?;
    atomic_write(
        &resources.join("package.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    if bundle {
        write_bundle(
            candidate.path(),
            &output,
            game_id,
            executable,
            game_icon.is_some() || app_icon.is_some(),
        )?;
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
fn write_bundle(
    candidate: &Path,
    output: &Path,
    game_id: comet_core::GameId,
    executable: &str,
    has_icon: bool,
) -> Result<()> {
    let name = output.file_stem().context("App name")?.to_string_lossy();
    let name = name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let icon = if has_icon {
        "<key>CFBundleIconFile</key><string>app.icns</string>"
    } else {
        ""
    };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>{executable}</string>
<key>CFBundleName</key><string>{name}</string>
<key>CFBundleDisplayName</key><string>{name}</string>
<key>CFBundleIdentifier</key><string>org.comet.game.{game_id}</string>
<key>CFBundlePackageType</key><string>APPL</string>
{icon}
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>14.2</string>
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
