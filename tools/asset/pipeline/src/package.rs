use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::{AssetKind, ProductIndex};
use comet_core::{AssetId, atomic_write};
use std::{fs, path::Path};
/// Export a product closure and an already-built Player to a new, movable directory.
pub fn package(products: &Path, scene: AssetId, player: &Path, output: &Path) -> Result<()> {
    ensure!(
        player.is_file(),
        "Player executable does not exist: {}",
        player.display()
    );
    ensure!(
        !output.try_exists()?,
        "package destination already exists: {}",
        output.display()
    );
    let store = AssetStore::open(products)?;
    ensure!(
        store
            .index()
            .products
            .get(&scene)
            .is_some_and(|p| p.kind == AssetKind::Scene),
        "package entry must be a scene product"
    );
    let closure = store.dependency_closure(scene)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(output)?;
    let result = (|| -> Result<()> {
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
            let relative = std::path::PathBuf::from("products")
                .join(id.to_string())
                .join(format!("{}.json", entry.digest));
            let destination = output.join("products").join(&relative);
            fs::create_dir_all(destination.parent().context("product destination")?)?;
            fs::copy(products.join(&relative), destination)?;
        }
        atomic_write(
            &output.join("products/index.json"),
            &serde_json::to_vec_pretty(&index)?,
        )?;
        AssetStore::open(output.join("products"))?;
        let executable = if cfg!(windows) {
            "comet-player.exe"
        } else {
            "comet-player"
        };
        fs::copy(player, output.join(executable))?;
        atomic_write(
            &output.join("package.json"),
            &serde_json::to_vec_pretty(
                &serde_json::json!({"version":1,"entry_scene":scene,"executable":executable}),
            )?,
        )?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}
