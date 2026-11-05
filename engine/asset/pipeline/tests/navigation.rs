use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::{
    NavigationData, NavigationInput,
    product::{AssetKind, INDEX_VERSION, PRODUCT_VERSION, Product, ProductIndex},
};
use comet_asset_pipeline::{
    NavigationTarget, SourceMeta, bake_navigation, bake_navigation_source, create_project, package,
    prepare, publish_navigation,
};
use comet_core::AssetId;
use comet_scene::{
    ColliderShape, ContentSource, EntityData, NavigationRegion, PhysicsBody, SceneSnapshot,
};
use std::{collections::BTreeSet, fs, path::Path};

fn body(name: &str, position: [f32; 3], half: [f32; 3]) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.transform.translation = position;
    entity.physics = Some(PhysicsBody {
        shape: ColliderShape::Box { half_extents: half },
        ..Default::default()
    });
    entity
}
fn scene() -> SceneSnapshot {
    let mut region = EntityData::new("导航区域");
    region.navigation_region = Some(NavigationRegion {
        half_extents: [6., 4., 6.],
        cell_size: 0.2,
        ..Default::default()
    });
    SceneSnapshot {
        entities: vec![region, body("地板", [0., -0.25, 0.], [6., 0.25, 6.])],
        ..Default::default()
    }
}
fn polygon_at(data: &NavigationData, point: [f32; 3]) -> Option<usize> {
    data.polygons
        .iter()
        .enumerate()
        .find_map(|(index, polygon)| {
            let height = polygon
                .iter()
                .map(|&i| data.vertices[i as usize][1])
                .sum::<f32>()
                / polygon.len() as f32;
            if (height - point[1]).abs() > 0.3 {
                return None;
            }
            polygon
                .iter()
                .zip(polygon.iter().cycle().skip(1))
                .all(|(&a, &b)| {
                    let a = data.vertices[a as usize];
                    let b = data.vertices[b as usize];
                    (b[0] - a[0]) * (point[2] - a[2]) - (b[2] - a[2]) * (point[0] - a[0]) >= -1e-4
                })
                .then_some(index)
        })
}
fn connected(data: &NavigationData, start: [f32; 3], end: [f32; 3]) -> bool {
    let (Some(start), Some(end)) = (polygon_at(data, start), polygon_at(data, end)) else {
        return false;
    };
    let mut seen = BTreeSet::from([start]);
    let mut pending = vec![start];
    while let Some(current) = pending.pop() {
        if current == end {
            return true;
        }
        let edges: BTreeSet<_> = data.polygons[current]
            .iter()
            .zip(data.polygons[current].iter().cycle().skip(1))
            .map(|(&a, &b)| (a.min(b), a.max(b)))
            .collect();
        for (index, polygon) in data.polygons.iter().enumerate() {
            if !seen.contains(&index)
                && polygon
                    .iter()
                    .zip(polygon.iter().cycle().skip(1))
                    .any(|(&a, &b)| edges.contains(&(a.min(b), a.max(b))))
            {
                seen.insert(index);
                pending.push(index);
            }
        }
    }
    false
}

#[test]
fn real_box_bakes_normalize_winding_preserve_freshness_and_route_around_walls() -> Result<()> {
    let mut scene = scene();
    scene
        .entities
        .push(body("墙", [0., 1.5, 0.], [0.3, 1.5, 2.]));
    let input = NavigationInput::capture(&scene)?;
    let data = bake_navigation(&input)?;
    data.validate_for(&scene)?;
    assert!(connected(&data, [-4., 0.1, 0.], [4., 0.1, 0.]));
    assert!(polygon_at(&data, [0., 0.1, 0.]).is_none());
    assert!(data.vertices.iter().all(|vertex| vertex[1].is_finite()));
    let bytes = serde_json::to_vec(&data)?;
    let decoded: NavigationData = serde_json::from_slice(&bytes)?;
    assert_eq!(decoded, data);
    scene.entities.last_mut().unwrap().transform.rotation =
        comet_core::math::Quat::from_rotation_y(0.37).to_array();
    let rotated = bake_navigation(&NavigationInput::capture(&scene)?)?;
    rotated.validate_for(&scene)?;
    assert!(connected(&rotated, [-4., 0.1, 0.], [4., 0.1, 0.]));
    Ok(())
}

#[test]
fn clearance_slope_and_step_profile_filter_the_actual_collision_geometry() -> Result<()> {
    let mut low = scene();
    low.entities
        .push(body("低顶", [0., 1.1, 0.], [2., 0.2, 2.]));
    let data = bake_navigation(&NavigationInput::capture(&low)?)?;
    assert!(polygon_at(&data, [0., 0.1, 0.]).is_none());
    assert!(polygon_at(&data, [4., 0.1, 0.]).is_some());

    for (degrees, walkable) in [(15_f32, true), (55., false)] {
        let mut slope = scene();
        slope.entities[0]
            .navigation_region
            .as_mut()
            .unwrap()
            .max_slope_degrees = 30.;
        let mut ramp = body("坡面", [0., 2., 0.], [2., 0.1, 1.5]);
        ramp.transform.rotation =
            comet_core::math::Quat::from_rotation_z(degrees.to_radians()).to_array();
        slope.entities.push(ramp);
        let data = bake_navigation(&NavigationInput::capture(&slope)?)?;
        assert_eq!(data.vertices.iter().any(|vertex| vertex[1] > 1.), walkable);
    }
    for (height, reachable) in [(0.2, true), (0.8, false)] {
        let mut step = scene();
        step.entities
            .push(body("台阶", [0., height / 2., 0.], [2., height / 2., 2.]));
        let data = bake_navigation(&NavigationInput::capture(&step)?)?;
        assert_eq!(
            connected(&data, [-4., 0.1, 0.], [0., height + 0.1, 0.]),
            reachable
        );
    }
    Ok(())
}

#[test]
fn stable_navigation_sources_keep_identity_and_load_from_a_relocated_source_free_package()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let paths = create_project(&project)?;
    let products = temp.path().join("products");
    let source = paths.assets().join("main.scene");
    let mut content = ContentSource {
        base: scene(),
        ..Default::default()
    };
    let authored = content.to_json()?;
    fs::write(&source, &authored)?;
    let scene_id = SourceMeta::load(&source)?.id;
    let scene_meta = fs::read(SourceMeta::path(&source))?;
    let id = bake_navigation_source(
        &project,
        Path::new("main.scene"),
        &products,
        Path::new("baked/main.nav"),
    )?;
    assert_eq!(
        fs::read(&source)?,
        authored,
        "CLI-oriented entry must not change bindings"
    );
    assert_eq!(fs::read(SourceMeta::path(&source))?, scene_meta);
    let nav_path = paths.assets().join("baked/main.nav");
    let metadata = fs::read(SourceMeta::path(&nav_path))?;
    let first = fs::read(&nav_path)?;
    assert_eq!(SourceMeta::load(&nav_path)?.id, id);
    let mut store = AssetStore::open(&products)?;
    assert_eq!(store.index().version, INDEX_VERSION);
    assert_eq!(store.index().products[&id].kind, AssetKind::Navigation);
    assert_eq!(store.load(id)?.version, PRODUCT_VERSION);
    content.base.entities[0]
        .navigation_region
        .as_mut()
        .unwrap()
        .data = Some(id);
    fs::write(&source, content.to_json()?)?;
    prepare(&project, &products)?;
    store.refresh()?;
    let snapshot = store.expand_scene(scene_id)?;
    let Product::Navigation(data) = &store.load(id)?.payload else {
        unreachable!()
    };
    data.validate_for(&snapshot)?;
    assert_eq!(store.dependency_closure(scene_id)?.len(), 2);
    let target = NavigationTarget::open(&paths, Path::new("baked/main.nav"))?;
    assert_eq!(publish_navigation(&paths, &target, data)?, id);
    assert_eq!(fs::read(SourceMeta::path(&nav_path))?, metadata);
    assert_eq!(fs::read(&nav_path)?, first);
    let output = temp.path().join("game");
    package(
        &products,
        comet_core::GameId::new(),
        scene_id,
        &std::env::current_exe()?,
        &output,
    )?;
    let relocated = temp.path().join("elsewhere");
    fs::rename(output, &relocated)?;
    fs::remove_dir_all(&project)?;
    fs::remove_dir_all(&products)?;
    let mut packaged = AssetStore::open(relocated.join("products"))?;
    assert_eq!(packaged.index().products.len(), 2);
    assert!(
        packaged
            .index()
            .products
            .values()
            .all(|entry| entry.editor.is_none())
    );
    let Product::Navigation(data) = &packaged.load(id)?.payload else {
        unreachable!()
    };
    data.validate_for(&packaged.expand_scene(scene_id)?)?;
    Ok(())
}

#[test]
fn publication_rejects_external_source_or_metadata_changes_and_reuses_reserved_identity()
-> Result<()> {
    let data = bake_navigation(&NavigationInput::capture(&scene())?)?;
    for change in 0..3 {
        let temp = tempfile::tempdir()?;
        let paths = create_project(&temp.path().join("project"))?;
        let relative = Path::new("baked/main.nav");
        let target = NavigationTarget::open(&paths, relative)?;
        fs::create_dir_all(target.path().parent().unwrap())?;
        match change {
            0 => fs::write(target.path(), serde_json::to_vec(&data)?)?,
            1 => fs::write(
                SourceMeta::path(target.path()),
                serde_json::to_vec(&SourceMeta {
                    version: 1,
                    id: AssetId::new(),
                })?,
            )?,
            _ => fs::create_dir(target.path())?,
        }
        assert!(publish_navigation(&paths, &target, &data).is_err());
    }
    let temp = tempfile::tempdir()?;
    let paths = create_project(&temp.path().join("project"))?;
    let relative = Path::new("reserved.nav");
    let path = paths.assets().join(relative);
    let reserved = SourceMeta {
        version: 1,
        id: AssetId::new(),
    };
    let bytes = serde_json::to_vec(&reserved)?;
    fs::write(SourceMeta::path(&path), &bytes)?;
    let target = NavigationTarget::open(&paths, relative)?;
    assert_eq!(target.asset(), reserved.id);
    publish_navigation(&paths, &target, &data)?;
    assert_eq!(fs::read(SourceMeta::path(&path))?, bytes);
    let target = NavigationTarget::open(&paths, relative)?;
    let external = b"external source modification";
    fs::write(&path, external)?;
    assert!(publish_navigation(&paths, &target, &data).is_err());
    assert_eq!(fs::read(&path)?, external);
    assert_eq!(fs::read(SourceMeta::path(&path))?, bytes);
    Ok(())
}

#[test]
fn navigation_targets_reject_bad_identity_existing_unidentified_sources_and_outside_aliases()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let paths = create_project(&temp.path().join("project"))?;
    for relative in [
        Path::new("bad.scene"),
        Path::new("../outside.nav"),
        temp.path().join("outside.nav").as_path(),
    ] {
        assert!(NavigationTarget::open(&paths, relative).is_err());
    }
    let data = bake_navigation(&NavigationInput::capture(&scene())?)?;
    let path = paths.assets().join("bad.nav");
    fs::write(&path, serde_json::to_vec(&data)?)?;
    assert!(NavigationTarget::open(&paths, Path::new("bad.nav")).is_err());
    fs::write(SourceMeta::path(&path), b"broken metadata")?;
    assert!(NavigationTarget::open(&paths, Path::new("bad.nav")).is_err());
    #[cfg(unix)]
    {
        let outside = temp.path().join("external.nav");
        fs::write(&outside, serde_json::to_vec(&data)?)?;
        std::os::unix::fs::symlink(&outside, paths.assets().join("link.nav"))?;
        assert!(NavigationTarget::open(&paths, Path::new("link.nav")).is_err());
        let target = NavigationTarget::open(&paths, Path::new("new.nav"))?;
        std::os::unix::fs::symlink(&outside, SourceMeta::path(target.path()))?;
        assert!(publish_navigation(&paths, &target, &data).is_err());
        assert!(!target.path().exists());
    }
    Ok(())
}

#[test]
fn old_index_versions_cannot_smuggle_navigation_and_invalid_nav_keeps_the_accepted_index()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let paths = create_project(&project)?;
    let data = bake_navigation(&NavigationInput::capture(&scene())?)?;
    let target = NavigationTarget::open(&paths, Path::new("main.nav"))?;
    let id = publish_navigation(&paths, &target, &data)?;
    let products = temp.path().join("products");
    prepare(&project, &products)?;
    let index_path = products.join("index.json");
    let before = fs::read(&index_path)?;
    let current: ProductIndex = serde_json::from_slice(&before)?;
    for version in 1..=3 {
        let mut old = current.clone();
        old.version = version;
        if version < 3 {
            for entry in old.products.values_mut() {
                entry.editor = None;
            }
        }
        let bytes = serde_json::to_vec(&old)?;
        fs::write(&index_path, &bytes)?;
        assert!(prepare(&project, &products).is_err());
        assert_eq!(fs::read(&index_path)?, bytes);
    }
    fs::write(&index_path, &before)?;
    let mut invalid = data.clone();
    invalid.polygons[0].reverse();
    fs::write(target.path(), serde_json::to_vec(&invalid)?)?;
    assert!(prepare(&project, &products).is_err());
    assert_eq!(fs::read(&index_path)?, before);
    let mut accepted = AssetStore::open(&products)?;
    assert_eq!(accepted.load(id)?.payload, Product::Navigation(data));
    Ok(())
}
