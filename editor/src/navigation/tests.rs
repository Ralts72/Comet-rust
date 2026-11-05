use super::*;
use comet_scene::{BodyMotion, ColliderShape, PhysicsBody};
use std::{
    fs,
    time::{Duration, Instant},
};

fn fixture() -> Result<(tempfile::TempDir, Editor, AssetStore, EntityId)> {
    let (temp, mut editor, assets) = crate::test_support::fixture()?;
    let mut region = EntityData::new("导航区域对象");
    region.navigation_region = Some(NavigationRegion {
        half_extents: [4., 3., 4.],
        cell_size: 0.25,
        ..Default::default()
    });
    let id = region.id;
    let mut floor = EntityData::new("静态地面");
    floor.transform.translation = [0., -0.25, 0.];
    floor.physics = Some(PhysicsBody {
        motion: BodyMotion::Fixed,
        shape: ColliderShape::Box {
            half_extents: [3.5, 0.25, 3.5],
        },
        ..Default::default()
    });
    editor.document = Document::open(SceneSnapshot {
        entities: vec![region, floor],
        ..Default::default()
    })?;
    Ok((temp, editor, assets, id))
}
fn guard(editor: &Editor, assets: &AssetStore, input: &NavigationInput) -> NavigationGuard {
    NavigationGuard {
        world: editor.document.world_id(),
        document_revision: editor.document.revision(),
        index_revision: assets.revision().to_owned(),
        prefab_asset: editor.document.prefab_asset(),
        region: input.region(),
        input_digest: input.digest().to_owned(),
    }
}
fn wait(controller: &mut NavigationController) -> Result<NavigationResult> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = controller.poll() {
            return result.map_err(anyhow::Error::msg);
        }
        ensure!(Instant::now() < deadline, "navigation task timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn bake(editor: &mut Editor, assets: &mut AssetStore, path: &str) -> Result<BakedNavigation> {
    let snapshot = editor.document.expand(assets)?;
    let input = editor
        .navigation
        .input(&editor.document, &snapshot, assets.revision())
        .map_err(anyhow::Error::msg)?;
    let guard = guard(editor, assets, &input);
    editor.navigation.start(
        &ProjectPaths::open(editor.project_root())?,
        Path::new(path),
        guard,
        input,
    )?;
    let NavigationResult::Baked(baked) = wait(&mut editor.navigation)? else {
        anyhow::bail!("expected pure bake result");
    };
    Ok(baked)
}

#[test]
fn pure_baking_does_not_publish_until_preparation_and_never_changes_the_live_document_or_store()
-> Result<()> {
    let (_temp, mut editor, mut assets, _) = fixture()?;
    let original = editor.document.shared_source();
    let revision = assets.revision().to_owned();
    let source_bytes = fs::read(&editor.source)?;
    let paths = ProjectPaths::open(editor.project_root())?;
    let output = paths.assets().join("navigation-test.nav");
    let baked = bake(&mut editor, &mut assets, "navigation-test.nav")?;
    assert!(!output.exists() && !comet_asset_pipeline::SourceMeta::path(&output).exists());
    editor.navigation.prepare(
        &paths,
        &editor.products,
        baked,
        editor.document.retained_sources().collect(),
    )?;
    let NavigationResult::Prepared(mut candidate) = wait(&mut editor.navigation)? else {
        anyhow::bail!("expected prepared products");
    };
    assert_eq!(candidate.output, output);
    assert_eq!(
        comet_asset_pipeline::SourceMeta::load(&output)?.id,
        candidate.asset
    );
    let Product::Navigation(data) = &candidate.assets.load(candidate.asset)?.payload else {
        anyhow::bail!("expected navigation product");
    };
    assert_eq!(data.input_digest, candidate.guard.input_digest);
    data.validate_for(&editor.document.expand(&mut candidate.assets)?)?;
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    assert_eq!(assets.revision(), revision);
    assert_eq!(fs::read(&editor.source)?, source_bytes);
    Ok(())
}

#[test]
fn failed_product_preparation_keeps_a_complete_navigation_file_and_retry_reuses_its_identity()
-> Result<()> {
    let (_temp, mut editor, mut assets, _) = fixture()?;
    let source = editor.document.shared_source();
    let revision = assets.revision().to_owned();
    let paths = ProjectPaths::open(editor.project_root())?;
    let broken = paths.assets().join("broken.scene");
    let baked = bake(&mut editor, &mut assets, "retry.nav")?;
    fs::write(&broken, b"unrelated invalid scene")?;
    editor.navigation.prepare(
        &paths,
        &editor.products,
        baked,
        editor.document.retained_sources().collect(),
    )?;
    let error = wait(&mut editor.navigation)
        .err()
        .context("prepare should fail")?;
    assert!(format!("{error:#}").contains("完整保存并保留"));
    let output = paths.assets().join("retry.nav");
    serde_json::from_slice::<NavigationData>(&fs::read(&output)?)?.validate()?;
    let identity = comet_asset_pipeline::SourceMeta::load(&output)?.id;
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    assert_eq!(assets.revision(), revision);
    fs::remove_file(broken)?;
    let baked = bake(&mut editor, &mut assets, "retry.nav")?;
    editor.navigation.prepare(
        &paths,
        &editor.products,
        baked,
        editor.document.retained_sources().collect(),
    )?;
    let NavigationResult::Prepared(candidate) = wait(&mut editor.navigation)? else {
        anyhow::bail!("expected retry candidate");
    };
    assert_eq!(candidate.asset, identity);
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    assert_eq!(assets.revision(), revision);
    Ok(())
}

#[test]
fn idle_navigation_status_reuses_input_and_does_not_reload_an_evicted_product() -> Result<()> {
    let (_temp, mut editor, mut assets, region) = fixture()?;
    let baked = bake(&mut editor, &mut assets, "cached.nav")?;
    editor.navigation.prepare(
        &ProjectPaths::open(editor.project_root())?,
        &editor.products,
        baked,
        editor.document.retained_sources().collect(),
    )?;
    let NavigationResult::Prepared(candidate) = wait(&mut editor.navigation)? else {
        anyhow::bail!("expected products");
    };
    assets = candidate.assets;
    let mut value = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .find(|entity| entity.id == region)
        .unwrap()
        .navigation_region
        .clone()
        .unwrap();
    value.data = Some(candidate.asset);
    editor.edit_navigation(region, Some(value.clone()), None, &mut assets)?;
    editor.commit()?;
    let snapshot = editor.document.expand(&mut assets)?;
    let input = editor
        .navigation
        .input(&editor.document, &snapshot, assets.revision())
        .map_err(anyhow::Error::msg)?;
    editor
        .navigation
        .status(&editor.document, &snapshot, &mut assets)
        .map_err(anyhow::Error::msg)?;
    assets.evict(candidate.asset);
    let resident = assets.resident_count();
    for _ in 0..20 {
        assert!(Arc::ptr_eq(
            &input,
            &editor
                .navigation
                .input(&editor.document, &snapshot, assets.revision())
                .map_err(anyhow::Error::msg)?
        ));
        editor
            .navigation
            .status(&editor.document, &snapshot, &mut assets)
            .map_err(anyhow::Error::msg)?;
    }
    assert_eq!(assets.resident_count(), resident);
    value.cell_size *= 2.;
    editor.edit_navigation(region, Some(value), None, &mut assets)?;
    editor.commit()?;
    let snapshot = editor.document.expand(&mut assets)?;
    let changed = editor
        .navigation
        .input(&editor.document, &snapshot, assets.revision())
        .map_err(anyhow::Error::msg)?;
    assert!(!Arc::ptr_eq(&input, &changed));
    assert_ne!(input.digest(), changed.digest());
    assert!(
        editor
            .navigation
            .status(&editor.document, &snapshot, &mut assets)
            .is_err()
    );
    Ok(())
}
