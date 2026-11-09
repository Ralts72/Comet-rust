//! Rerecast and author-file publication remain producer details; games consume CPU products.
use anyhow::{Context, Result, ensure};
use comet_asset_data::navigation::{
    MAX_NAVIGATION_BYTES, MAX_NAVIGATION_POLYGONS, MAX_NAVIGATION_VERTICES, NAVIGATION_VERSION,
    NavigationData, NavigationInput,
};
use comet_core::{AssetId, ProjectPaths, atomic_create, atomic_write, digest};
use rerecast::{Aabb3d, AreaType, BuildContoursFlags, HeightfieldBuilder, TriMesh};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_SPANS: usize = 1_048_576;
const MAX_COMPACT_SPANS: usize = 262_144;

pub fn bake_navigation(input: &NavigationInput) -> Result<NavigationData> {
    let profile = input.profile();
    let bounds = input.bounds();
    let walkable_height = (profile.agent_height / profile.cell_height).ceil() as u16;
    let walkable_climb = (profile.max_climb / profile.cell_height).floor() as u16;
    let walkable_radius = (profile.agent_radius / profile.cell_size).ceil() as u16;
    let mut mesh = TriMesh {
        vertices: input
            .triangles()
            .iter()
            .flatten()
            .map(|vertex| (*vertex).into())
            .collect(),
        indices: (0..input.triangles().len())
            .map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32].into())
            .collect(),
        area_types: vec![AreaType::NOT_WALKABLE; input.triangles().len()],
    };
    mesh.mark_walkable_triangles(profile.max_slope_degrees.to_radians());
    let mut field = HeightfieldBuilder {
        aabb: Aabb3d {
            min: bounds[0].into(),
            max: bounds[1].into(),
        },
        cell_size: profile.cell_size,
        cell_height: profile.cell_height,
    }
    .build()
    .context("create navigation heightfield")?;
    for (indices, area) in mesh.indices.iter().zip(&mesh.area_types) {
        field
            .rasterize_triangle(
                indices.to_array().map(|i| mesh.vertices[i as usize]),
                *area,
                walkable_climb,
            )
            .context("rasterize navigation collision geometry")?;
        ensure!(
            field.allocated_spans.len() <= MAX_SPANS,
            "navigation heightfield exceeds its span budget"
        );
    }
    field.filter_low_hanging_walkable_obstacles(walkable_climb);
    field.filter_ledge_spans(walkable_height, walkable_climb);
    field.filter_walkable_low_height_spans(walkable_height);
    let mut compact = field
        .into_compact(walkable_height, walkable_climb)
        .context("compact navigation heightfield")?;
    ensure!(
        compact.spans.len() <= MAX_COMPACT_SPANS,
        "navigation compact field exceeds its span budget"
    );
    compact.erode_walkable_area(walkable_radius);
    compact.build_distance_field();
    compact
        .build_regions(0, 0, 400)
        .context("partition navigation regions")?;
    let contours = compact.build_contours(
        1.3,
        (12. / profile.cell_size).floor() as u16,
        BuildContoursFlags::default(),
    );
    let polygon = contours
        .into_polygon_mesh(6)
        .context("build navigation polygons")?;
    ensure!(
        polygon.vertices.len() <= MAX_NAVIGATION_VERTICES
            && polygon.polygon_count() <= MAX_NAVIGATION_POLYGONS,
        "navigation mesh exceeds its output index budget"
    );
    let data = NavigationData {
        version: NAVIGATION_VERSION,
        input_digest: input.digest().into(),
        vertices: polygon
            .vertices
            .iter()
            .map(|v| {
                [
                    polygon.aabb.min.x + f32::from(v.x) * polygon.cell_size,
                    polygon.aabb.min.y + f32::from(v.y) * polygon.cell_height,
                    polygon.aabb.min.z + f32::from(v.z) * polygon.cell_size,
                ]
            })
            .collect(),
        // Native top-facing polygons use the opposite mathematical XZ order to our CPU protocol.
        polygons: polygon
            .polygons()
            .map(|indices| {
                let mut indices: Vec<_> = indices.map(u32::from).collect();
                indices.reverse();
                indices
            })
            .collect(),
    };
    data.validate_input(input)
        .context("validate baked navigation mesh")?;
    ensure!(
        serde_json::to_vec(&data)?.len() <= MAX_NAVIGATION_BYTES,
        "navigation output exceeds its serialized size budget"
    );
    Ok(data)
}

/// A one-shot publication candidate captures both author bytes and the preserved identity.
pub struct NavigationTarget {
    requested: PathBuf,
    path: PathBuf,
    metadata: PathBuf,
    previous_source: Option<String>,
    previous_metadata: Option<String>,
    asset: AssetId,
}
impl NavigationTarget {
    pub fn open(paths: &ProjectPaths, relative: &Path) -> Result<Self> {
        let _lock = crate::build::lock(&paths.root().join(".assetc.lock"), "project")?;
        let requested = if relative.is_absolute() {
            relative.to_owned()
        } else {
            paths.assets().join(relative)
        };
        reject_link(&requested)?;
        let path = paths.resolve_asset_path(&requested)?;
        ensure!(
            path.extension().is_some_and(|extension| extension == "nav"),
            "navigation author file must use .nav"
        );
        let metadata = crate::SourceMeta::path(&path);
        reject_link(&metadata)?;
        ensure!(
            paths.resolve_asset_path(&metadata)? == metadata,
            "navigation metadata location changed"
        );
        let source = read_optional(&path)?;
        if let Some(bytes) = &source {
            ensure!(
                bytes.len() <= MAX_NAVIGATION_BYTES,
                "navigation source exceeds its size budget"
            );
            let data: NavigationData =
                serde_json::from_slice(bytes).context("read existing navigation author file")?;
            data.validate()?;
        }
        let meta = read_optional(&metadata)?;
        ensure!(
            source.is_none() || meta.is_some(),
            "existing navigation author file has no metadata; restore its identity before baking"
        );
        let asset = match &meta {
            Some(bytes) => crate::SourceMeta::from_bytes(bytes)?.id,
            None => AssetId::new(),
        };
        Ok(Self {
            requested,
            path,
            metadata,
            previous_source: source.as_deref().map(digest),
            previous_metadata: meta.as_deref().map(digest),
            asset,
        })
    }
    pub fn asset(&self) -> AssetId {
        self.asset
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn publish_navigation(
    paths: &ProjectPaths,
    target: &NavigationTarget,
    data: &NavigationData,
) -> Result<AssetId> {
    data.validate()?;
    let bytes = serde_json::to_vec_pretty(data)?;
    ensure!(
        bytes.len() <= MAX_NAVIGATION_BYTES,
        "navigation output exceeds its size budget"
    );
    let _lock = crate::build::lock(&paths.root().join(".assetc.lock"), "project")?;
    reject_link(&target.requested)?;
    reject_link(&target.metadata)?;
    ensure!(
        paths.resolve_asset_path(&target.requested)? == target.path
            && paths.resolve_asset_path(&target.metadata)? == target.metadata,
        "navigation destination changed while baking"
    );
    ensure!(
        read_optional(&target.path)?.as_deref().map(digest) == target.previous_source,
        "navigation author file changed while baking"
    );
    ensure!(
        read_optional(&target.metadata)?.as_deref().map(digest) == target.previous_metadata,
        "navigation metadata changed while baking"
    );
    if target.previous_metadata.is_none() {
        crate::SourceMeta {
            version: 1,
            id: target.asset,
        }
        .reserve(&target.path)?;
    }
    // Metadata is reserved separately; a failed source publication can reuse it on the next attempt.
    if target.previous_source.is_none() {
        atomic_create(&target.path, &bytes)?;
    } else {
        atomic_write(&target.path, &bytes)?;
    }
    Ok(target.asset)
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "navigation destination is not a regular file: {}",
                path.display()
            );
            ensure!(
                metadata.len() <= MAX_NAVIGATION_BYTES as u64,
                "navigation destination exceeds its size budget"
            );
            Ok(Some(fs::read(path)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}
fn reject_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink(),
            "navigation author file and metadata cannot use symbolic links"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect navigation destination"),
    }
    Ok(())
}

/// Bake an author source for the CLI without changing its navigation binding.
pub fn bake_navigation_source(
    project: &Path,
    author: &Path,
    products: &Path,
    output: &Path,
) -> Result<AssetId> {
    use crate::SourceMeta;
    use comet_asset::AssetStore;
    let paths = comet_core::ProjectPaths::open(project)?;
    let source = paths.resolve_asset_path(author)?;
    let prefab = source.extension().is_some_and(|ext| ext == "prefab");
    ensure!(
        prefab || source.extension().is_some_and(|ext| ext == "scene"),
        "navigation baking requires a native .scene or .prefab author file"
    );
    let products = comet_core::resolve_destination(products)?;
    ensure!(
        !products.starts_with(paths.assets()) && !paths.assets().starts_with(&products),
        "navigation products must not overlap project assets"
    );
    let target = NavigationTarget::open(&paths, output)?;
    let bytes = std::fs::read(&source)?;
    crate::prepare(paths.root(), &products)?;
    ensure!(
        std::fs::read(&source)? == bytes,
        "navigation author source changed during preparation"
    );
    let metadata = std::fs::read(SourceMeta::path(&source))?;
    let content = comet_scene::ContentSource::from_json(&bytes)?;
    let identity = SourceMeta::load(&source)?.id;
    let capture = |store: &mut AssetStore| -> Result<NavigationInput> {
        let snapshot = if prefab {
            content.expand_prefab(identity, store)?
        } else {
            content.expand(store)?
        };
        NavigationInput::capture(&snapshot)
    };
    let input = capture(&mut AssetStore::open(&products)?)?;
    let data = bake_navigation(&input)?;
    // Rebuild and recapture referenced prefabs before publishing a long-running bake.
    crate::prepare(paths.root(), &products)?;
    ensure!(
        paths.resolve_asset_path(author)? == source
            && std::fs::read(&source)? == bytes
            && std::fs::read(SourceMeta::path(&source))? == metadata,
        "navigation author source or identity changed while baking"
    );
    let current = capture(&mut AssetStore::open(&products)?)?;
    ensure!(
        current.digest() == input.digest(),
        "navigation geometry or profile changed while baking"
    );
    let id = publish_navigation(&paths, &target, &data)?;
    crate::prepare(paths.root(), &products).with_context(|| {
        format!(
            "navigation author file was published at {}, but product preparation failed",
            target.path().display()
        )
    })?;
    ensure!(
        paths.resolve_asset_path(author)? == source
            && std::fs::read(&source)? == bytes
            && std::fs::read(SourceMeta::path(&source))? == metadata
            && SourceMeta::load(target.path())?.id == id,
        "navigation file was published, but authoring content or identity changed during final preparation"
    );
    let mut accepted = AssetStore::open(&products)?;
    ensure!(
        capture(&mut accepted)?.digest() == input.digest(),
        "navigation file was published, but collision geometry changed during final preparation"
    );
    ensure!(
        matches!(&accepted.load(id)?.payload, comet_asset_data::product::Product::Navigation(current) if current == &data),
        "navigation author data changed after publication"
    );
    Ok(id)
}
