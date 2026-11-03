use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_pipeline::{PackageRequest, package_scenes};
use comet_core::{AssetId, digest};
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
            &serde_json::json!({"version":1,"positions":[[0,0,0],[1,0,0],[0,1,0]],"normals":[[0,0,1],[0,0,1],[0,0,1]],"uvs":[[0,0],[0,0],[0,0]],"indices":[0,1,2]}),
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
    let player = temp.path().join("player");
    fs::write(&player, b"external binary")?;
    let output = temp.path().join("game");
    let notice_file = temp.path().join("OFL.txt");
    fs::write(&notice_file, b"font license")?;
    let notices = [comet_asset_pipeline::PackageNotice {
        name: "NotoSansSC-OFL.txt".into(),
        path: notice_file.clone(),
        sha256: digest(b"font license"),
    }];
    let request = |revision, hash| PackageRequest {
        products: &products,
        index_revision: revision,
        entry_scene: main,
        additional_scenes: std::slice::from_ref(&additional),
        player: &player,
        runtime: None,
        player_digest: hash,
        notices: &notices,
        output: &output,
    };
    assert!(package_scenes(request(Some("stale index"), None)).is_err());
    assert!(!output.exists());
    let bad_hash = "0".repeat(64);
    let player_hash = digest(b"external binary");
    assert!(package_scenes(request(None, Some(&bad_hash))).is_err());
    assert!(!output.exists());
    let game = package_scenes(request(Some(store.revision()), Some(&player_hash)))?;
    let packaged = AssetStore::open(&game.products)?;
    assert_eq!(packaged.index().products.len(), 4);
    assert_eq!(
        fs::read(output.join("licenses/NotoSansSC-OFL.txt"))?,
        b"font license"
    );
    for id in [main, additional, mesh, material] {
        assert!(packaged.index().products.contains_key(&id));
    }
    assert!(!packaged.index().products.contains_key(&unused));
    let manifest: comet_asset_data::package::PackageManifest =
        serde_json::from_slice(&fs::read(output.join("package.json"))?)?;
    manifest.validate()?;
    assert_eq!(manifest.scenes.len(), 2);
    assert_eq!(manifest.entry_scene, main);
    let sentinel = fs::read(output.join("package.json"))?;
    assert!(package_scenes(request(None, None)).is_err());
    assert_eq!(fs::read(output.join("package.json"))?, sentinel);
    assert!(fs::read_dir(temp.path())?.all(|p| {
        !p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".comet-")
    }));
    Ok(())
}
