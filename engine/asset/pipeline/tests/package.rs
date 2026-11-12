use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::package::RuntimeIdentity;
use comet_asset_pipeline::{PackageRequest, package_scenes};
use comet_core::{AssetId, digest, file_digest};
use comet_scene::{ContentSource, EntityData, Model, Surface};
use std::{fs, path::Path};
fn source(path: &Path, name: &str, bytes: Vec<u8>) -> Result<AssetId> {
    let id = AssetId::new();
    fs::write(path.join(name), bytes)?;
    fs::write(
        path.join(format!("{name}.meta")),
        serde_json::to_vec(&serde_json::json!({"version":1,"id":id}))?,
    )?;
    Ok(id)
}

#[test]
fn project_icons_are_persistent_portable_and_never_modify_the_app_template() -> Result<()> {
    use comet_asset_pipeline::ProjectSettings;
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let assets = project.join("assets");
    fs::create_dir_all(&assets)?;
    let scene = source(&assets, "main.scene", ContentSource::default().to_json()?)?;
    let mut png = vec![];
    {
        let mut encoder = png::Encoder::new(&mut png, 32, 32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()?
            .write_image_data(&[220, 10, 30, 255].repeat(32 * 32))?;
    }
    let icon = source(&assets, "game.png", png)?;
    let products = temp.path().join("products");
    comet_asset_pipeline::prepare(&project, &products)?;
    let paths = comet_core::ProjectPaths::open(&project)?;
    let mut store = AssetStore::open(&products)?;
    let original = ProjectSettings::for_new_project();
    fs::write(project.join("project.json"), serde_json::to_vec(&original)?)?;
    assert_eq!(ProjectSettings::load(&paths)?, original);
    let settings = ProjectSettings {
        game_icon: Some(icon),
        ..original
    };
    settings.save(&paths, &mut store)?;
    assert_eq!(ProjectSettings::load(&paths)?, settings);
    let invalid = ProjectSettings {
        game_icon: Some(scene),
        ..settings.clone()
    };
    assert!(invalid.save(&paths, &mut store).is_err());
    assert_eq!(ProjectSettings::load(&paths)?, settings);
    fs::rename(assets.join("game.png"), assets.join("renamed.png"))?;
    fs::rename(
        assets.join("game.png.meta"),
        assets.join("renamed.png.meta"),
    )?;
    comet_asset_pipeline::prepare(&project, &products)?;
    let app = std::env::current_exe()?;
    let app_hash = file_digest(&app)?;
    let output = temp.path().join("export");
    let game_id = comet_core::GameId::new();
    let request = |id| PackageRequest {
        products: &products,
        game_id,
        index_revision: None,
        entry_scene: scene,
        additional_scenes: &[],
        app: &app,
        runtime: None,
        app_digest: Some(&app_hash),
        notices: &[],
        app_icon: None,
        game_icon: Some(id),
        output: &output,
    };
    assert!(package_scenes(request(scene)).is_err());
    assert!(!output.exists());
    let package = package_scenes(request(icon))?;
    let image = fs::read(output.join("game-icon.png"))?;
    let decoded = image::load_from_memory(&image)?.to_rgba8();
    assert_eq!(decoded.dimensions(), (256, 256));
    assert_eq!(decoded.get_pixel(128, 128).0, [220, 10, 30, 255]);
    let icns = fs::read(output.join("app.icns"))?;
    assert_eq!(&icns[..4], b"icns");
    assert_eq!(
        u32::from_be_bytes(icns[4..8].try_into()?) as usize,
        icns.len()
    );
    let manifest: comet_asset_data::package::PackageManifest =
        serde_json::from_slice(&fs::read(output.join("package.json"))?)?;
    assert_eq!(manifest.game_icon_sha256, Some(digest(&image)));
    assert_eq!(manifest.game_id, game_id);
    assert_eq!(file_digest(&app)?, app_hash);
    assert_eq!(
        AssetStore::open(package.products)?.index().products.len(),
        1
    );
    #[cfg(unix)]
    {
        fs::remove_file(project.join("project.json"))?;
        let outside = temp.path().join("outside.json");
        fs::write(&outside, serde_json::to_vec(&settings)?)?;
        std::os::unix::fs::symlink(&outside, project.join("project.json"))?;
        assert!(ProjectSettings::load(&paths).is_err());
        assert!(settings.save(&paths, &mut store).is_err());
    }
    Ok(())
}
#[test]
fn scene_union_packaging_is_complete_atomic_and_rejects_stale_programs_or_indices() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let assets = project.join("assets");
    fs::create_dir_all(&assets)?;
    let main = source(&assets, "main.scene", ContentSource::default().to_json()?)?;
    let mesh = source(
        &assets,
        "triangle.mesh",
        serde_json::to_vec(
            &serde_json::json!({"version":comet_asset_data::product::MESH_VERSION,"positions":[[0,0,0],[1,0,0],[0,1,0]],"normals":[[0,0,1],[0,0,1],[0,0,1]],"uvs":[[0,0],[0,0],[0,0]],"indices":[0,1,2]}),
        )?,
    )?;
    let material = source(
        &assets,
        "plain.material",
        serde_json::to_vec(
            &serde_json::json!({"version":1,"base_color":[1,1,1,1],"roughness":0.5,"metallic":0,"base_color_texture":null}),
        )?,
    )?;
    let mut e = EntityData::new("additional-scene-only mesh");
    e.model = Some(Model {
        mesh,
        surface: Surface {
            material,
            roughness: 0.5,
        },
    });
    let mut extra = ContentSource::default();
    extra.base.entities.push(e);
    let additional = source(&assets, "extra.scene", extra.to_json()?)?;
    let unused = source(&assets, "unused.scene", ContentSource::default().to_json()?)?;
    let products = temp.path().join("products");
    comet_asset_pipeline::prepare(&project, &products)?;
    let store = AssetStore::open(&products)?;
    let app = temp.path().join("app");
    fs::write(&app, b"external binary")?;
    let output = temp.path().join("game");
    let notice_file = temp.path().join("OFL.txt");
    fs::write(&notice_file, b"font license")?;
    let notices = [comet_asset_pipeline::PackageFile {
        name: "NotoSansSC-OFL.txt".into(),
        path: notice_file.clone(),
        sha256: digest(b"font license"),
    }];
    let icon_bytes = b"icns\0\0\0\x0cTEST";
    let icon_path = temp.path().join("app.icns");
    fs::write(&icon_path, icon_bytes)?;
    let icon = comet_asset_pipeline::PackageFile {
        name: "app.icns".into(),
        path: icon_path.clone(),
        sha256: digest(icon_bytes),
    };
    let identity = RuntimeIdentity::for_build("a".repeat(64), "fixture-target");
    let game_id = comet_core::GameId::new();
    let request = |revision, hash| PackageRequest {
        products: &products,
        game_id,
        index_revision: revision,
        entry_scene: main,
        additional_scenes: std::slice::from_ref(&additional),
        app: &app,
        runtime: Some(identity.clone()),
        app_digest: hash,
        notices: &notices,
        app_icon: Some(&icon),
        game_icon: None,
        output: &output,
    };
    assert!(package_scenes(request(Some("stale index"), None)).is_err());
    assert!(!output.exists());
    let mut invalid_identity = request(None, None);
    invalid_identity.runtime.as_mut().unwrap().build_id = "unprepared".into();
    assert!(package_scenes(invalid_identity).is_err());
    assert!(!output.exists());
    let bad_hash = "0".repeat(64);
    let app_hash = digest(b"external binary");
    assert!(package_scenes(request(None, Some(&bad_hash))).is_err());
    assert!(!output.exists());
    fs::write(&icon_path, b"changed icon")?;
    assert!(package_scenes(request(None, Some(&app_hash))).is_err());
    assert!(!output.exists());
    fs::write(&icon_path, icon_bytes)?;
    fs::write(&notice_file, b"changed license")?;
    assert!(package_scenes(request(None, Some(&app_hash))).is_err());
    assert!(!output.exists());
    fs::write(&notice_file, b"font license")?;
    let duplicate_notices = [
        notices[0].clone(),
        comet_asset_pipeline::PackageFile {
            name: notices[0].name.clone(),
            ..icon.clone()
        },
    ];
    let mut duplicate = request(None, Some(&app_hash));
    duplicate.notices = &duplicate_notices;
    assert!(package_scenes(duplicate).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read(&notice_file)?, b"font license");
    assert_eq!(fs::read(&icon_path)?, icon_bytes);
    let game = package_scenes(request(Some(store.revision()), Some(&app_hash)))?;
    let packaged = AssetStore::open(&game.products)?;
    assert_eq!(packaged.index().products.len(), 4);
    assert_eq!(
        fs::read(output.join("licenses/NotoSansSC-OFL.txt"))?,
        b"font license"
    );
    assert_eq!(fs::read(output.join("app.icns"))?, icon_bytes);
    for id in [main, additional, mesh, material] {
        assert!(packaged.index().products.contains_key(&id));
    }
    assert!(!packaged.index().products.contains_key(&unused));
    let manifest: comet_asset_data::package::PackageManifest =
        serde_json::from_slice(&fs::read(output.join("package.json"))?)?;
    manifest.validate()?;
    manifest.validate_for(&identity)?;
    assert_eq!(manifest.runtime.as_ref(), Some(&identity));
    assert_eq!(manifest.scenes.len(), 2);
    assert_eq!(manifest.entry_scene, main);
    assert_eq!(manifest.game_id, game_id);
    let sentinel = fs::read(output.join("package.json"))?;
    assert!(package_scenes(request(None, None)).is_err());
    assert_eq!(fs::read(output.join("package.json"))?, sentinel);
    let moved = temp.path().join("relocated/changed-entry");
    fs::create_dir(moved.parent().unwrap())?;
    let alternate = temp.path().join("alternate");
    let additional_scenes = [main];
    let mut next = request(None, Some(&app_hash));
    next.output = &alternate;
    next.entry_scene = additional;
    next.additional_scenes = &additional_scenes;
    next.runtime.as_mut().unwrap().build_id = "b".repeat(64);
    package_scenes(next)?;
    fs::rename(alternate, &moved)?;
    fs::remove_dir_all(&project)?;
    fs::remove_dir_all(&products)?;
    let relocated: comet_asset_data::package::PackageManifest =
        serde_json::from_slice(&fs::read(moved.join("package.json"))?)?;
    relocated.validate()?;
    assert_eq!(relocated.entry_scene, additional);
    assert_eq!(relocated.game_id, game_id);
    assert_ne!(relocated.runtime, manifest.runtime);
    assert_eq!(
        AssetStore::open(moved.join("products"))?
            .index()
            .products
            .len(),
        4
    );
    assert!(fs::read_dir(temp.path())?.all(|p| {
        !p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".comet-")
    }));
    Ok(())
}
