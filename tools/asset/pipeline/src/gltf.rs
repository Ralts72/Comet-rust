//! Static glTF/GLB -> prefab + stable mesh/material/image subproducts.
use anyhow::{Context, Result, ensure};
use base64::Engine;
use comet_asset_data::product::{MaterialData, MeshData, Product, TextureData};
use comet_core::{AssetId, EntityId, digest};
use comet_scene::{ContentSource, EntityData, Model, Surface, Transform};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub(crate) struct Imported {
    pub products: Vec<(AssetId, Product)>,
    pub files: BTreeMap<PathBuf, String>,
}
pub(crate) fn import(path: &Path, bytes: &[u8], id: AssetId) -> Result<Imported> {
    let gltf = gltf::Gltf::from_slice(bytes).context("parse glTF/GLB")?;
    let doc = &gltf.document;
    ensure!(
        doc.extensions_required().next().is_none(),
        "required glTF extensions are not supported"
    );
    ensure!(
        doc.skins().next().is_none() && doc.animations().next().is_none(),
        "skinning and animation import are not implemented"
    );
    let base = path
        .parent()
        .context("glTF source directory")?
        .canonicalize()?;
    let mut files = BTreeMap::new();
    let mut buffers = vec![];
    for b in doc.buffers() {
        let data = match b.source() {
            gltf::buffer::Source::Bin => gltf.blob.clone().context("missing GLB binary chunk")?,
            gltf::buffer::Source::Uri(uri) => read_uri(&base, uri, &mut files)?,
        };
        ensure!(
            data.len() >= b.length(),
            "truncated glTF buffer {}",
            b.index()
        );
        buffers.push(data);
    }
    for view in doc.views() {
        ensure!(
            view.offset()
                .checked_add(view.length())
                .is_some_and(|n| n <= buffers[view.buffer().index()].len()),
            "buffer view out of bounds"
        );
    }
    for a in doc.accessors() {
        if let Some(v) = a.view() {
            let stride = v.stride().unwrap_or(a.size());
            let length = a
                .count()
                .saturating_sub(1)
                .checked_mul(stride)
                .and_then(|n| n.checked_add(a.size()))
                .and_then(|n| n.checked_add(a.offset()))
                .context("accessor size overflow")?;
            ensure!(
                length <= v.length(),
                "accessor {} exceeds buffer view",
                a.index()
            );
        }
        ensure!(
            a.sparse().is_none(),
            "sparse glTF accessors are not supported"
        );
    }
    let mut products = vec![];
    let mut textures = BTreeMap::new();
    for image in doc.images() {
        let data = match image.source() {
            gltf::image::Source::Uri { uri, .. } => read_uri(&base, uri, &mut files)?,
            gltf::image::Source::View { view, .. } => buffers[view.buffer().index()]
                [view.offset()..view.offset() + view.length()]
                .to_vec(),
        };
        let decoded = image::load_from_memory(&data)
            .context("decode glTF image (PNG/JPEG)")?
            .into_rgba8();
        let texture = TextureData {
            width: decoded.width(),
            height: decoded.height(),
            rgba8: decoded.into_raw(),
        };
        let key = named_key(
            "image",
            image.index(),
            image.name(),
            doc.images().filter(|v| v.name() == image.name()).count(),
        );
        let image_id = id.child(&key);
        textures.insert(image.index(), image_id);
        products.push((image_id, Product::Texture(texture)));
    }
    let default_material = id.child("material:default");
    products.push((
        default_material,
        Product::Material(MaterialData {
            version: 1,
            base_color: [1.; 4],
            roughness: 1.,
            metallic: 1.,
            base_color_texture: None,
        }),
    ));
    let mut materials = BTreeMap::new();
    let mut roughness = BTreeMap::new();
    for material in doc.materials() {
        ensure!(
            material.alpha_mode() == gltf::material::AlphaMode::Opaque,
            "only opaque glTF materials are supported"
        );
        ensure!(
            material.normal_texture().is_none()
                && material.occlusion_texture().is_none()
                && material.emissive_texture().is_none(),
            "normal/occlusion/emissive texture import is not implemented yet"
        );
        let p = material.pbr_metallic_roughness();
        ensure!(
            p.metallic_roughness_texture().is_none(),
            "packed metallic/roughness textures are not implemented yet"
        );
        let texture = if let Some(t) = p.base_color_texture() {
            ensure!(t.tex_coord() == 0, "only TEXCOORD_0 is supported");
            Some(textures[&t.texture().source().index()])
        } else {
            None
        };
        let index = material.index().context("material index")?;
        let key = named_key(
            "material",
            index,
            material.name(),
            doc.materials()
                .filter(|v| v.name() == material.name())
                .count(),
        );
        let material_id = id.child(&key);
        materials.insert(index, material_id);
        roughness.insert(index, p.roughness_factor());
        products.push((
            material_id,
            Product::Material(MaterialData {
                version: 1,
                base_color: p.base_color_factor(),
                roughness: p.roughness_factor(),
                metallic: p.metallic_factor(),
                base_color_texture: texture,
            }),
        ));
    }
    let mut meshes = BTreeMap::new();
    for mesh in doc.meshes() {
        let key = named_key(
            "mesh",
            mesh.index(),
            mesh.name(),
            doc.meshes().filter(|v| v.name() == mesh.name()).count(),
        );
        for primitive in mesh.primitives() {
            ensure!(
                primitive.mode() == gltf::mesh::Mode::Triangles,
                "only glTF triangle primitives are supported"
            );
            ensure!(
                primitive.morph_targets().next().is_none(),
                "morph targets belong to animation import"
            );
            let reader = primitive.reader(|b| Some(buffers[b.index()].as_slice()));
            let positions: Vec<_> = reader
                .read_positions()
                .context("missing POSITION")?
                .collect();
            let indices: Vec<u32> = reader
                .read_indices()
                .map(|r| r.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());
            ensure!(
                indices.iter().all(|i| (*i as usize) < positions.len())
                    && indices.len().is_multiple_of(3),
                "glTF triangle index out of bounds"
            );
            let normals = reader
                .read_normals()
                .map(Iterator::collect)
                .unwrap_or_else(|| generate_normals(&positions, &indices));
            let uvs = reader
                .read_tex_coords(0)
                .map(|v| v.into_f32().collect())
                .unwrap_or_else(|| vec![[0.; 2]; positions.len()]);
            let mesh_id = id.child(&format!("{key}/primitive:{}", primitive.index()));
            products.push((
                mesh_id,
                Product::Mesh(MeshData {
                    version: 1,
                    positions,
                    normals,
                    uvs,
                    indices,
                }),
            ));
            meshes.insert((mesh.index(), primitive.index()), mesh_id);
        }
    }
    let mut source = ContentSource::default();
    let mut visited = BTreeSet::new();
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .context("glTF has no scene")?;
    struct Nodes<'a> {
        doc: &'a gltf::Document,
        id: AssetId,
        meshes: &'a BTreeMap<(usize, usize), AssetId>,
        materials: &'a BTreeMap<usize, AssetId>,
        roughness: &'a BTreeMap<usize, f32>,
        default_material: AssetId,
    }
    fn node(
        n: gltf::Node<'_>,
        parent: Option<EntityId>,
        ctx: &Nodes<'_>,
        source: &mut ContentSource,
        visited: &mut BTreeSet<usize>,
    ) -> Result<()> {
        ensure!(
            visited.insert(n.index()),
            "glTF node cycle or multiple parents at {}",
            n.index()
        );
        let key = named_key(
            "node",
            n.index(),
            n.name(),
            ctx.doc.nodes().filter(|v| v.name() == n.name()).count(),
        );
        let entity_id = EntityId::in_asset(ctx.id, &key);
        let mut entity = EntityData::new(n.name().unwrap_or("Node"));
        entity.id = entity_id;
        entity.parent = parent;
        let (translation, rotation, scale) = n.transform().decomposed();
        entity.transform = Transform {
            translation,
            rotation,
            scale,
        };
        source.base.entities.push(entity);
        if let Some(mesh) = n.mesh() {
            for primitive in mesh.primitives() {
                let mut child = EntityData::new(format!(
                    "{} / {}",
                    n.name().unwrap_or("Mesh"),
                    primitive.index()
                ));
                child.id =
                    EntityId::in_asset(ctx.id, &format!("{key}/primitive:{}", primitive.index()));
                child.parent = Some(entity_id);
                let material = primitive.material().index();
                child.model = Some(Model {
                    mesh: ctx.meshes[&(mesh.index(), primitive.index())],
                    surface: Surface {
                        material: material.map_or(ctx.default_material, |i| ctx.materials[&i]),
                        roughness: material.map_or(1., |i| ctx.roughness[&i]),
                    },
                });
                source.base.entities.push(child);
            }
        }
        for child in n.children() {
            node(child, Some(entity_id), ctx, source, visited)?;
        }
        Ok(())
    }
    let ctx = Nodes {
        doc,
        id,
        meshes: &meshes,
        materials: &materials,
        roughness: &roughness,
        default_material,
    };
    for root in scene.nodes() {
        node(root, None, &ctx, &mut source, &mut visited)?;
    }
    source.canonicalize();
    products.push((id, Product::Prefab(source)));
    for (_, p) in &products {
        p.validate()?;
    }
    Ok(Imported { products, files })
}
fn named_key(kind: &str, index: usize, name: Option<&str>, count: usize) -> String {
    match name.filter(|n| !n.is_empty() && count == 1) {
        Some(name) => format!("{kind}:name:{name}"),
        None => format!("{kind}:index:{index}"),
    }
}
fn read_uri(base: &Path, uri: &str, files: &mut BTreeMap<PathBuf, String>) -> Result<Vec<u8>> {
    if let Some(data) = uri.strip_prefix("data:") {
        let (header, data) = data.split_once(',').context("invalid data URI")?;
        ensure!(
            header.ends_with(";base64"),
            "only base64 glTF data URIs are supported"
        );
        return Ok(base64::engine::general_purpose::STANDARD.decode(data)?);
    }
    ensure!(
        !uri.contains([':', '?', '#']) && !uri.starts_with('/'),
        "only relative local glTF resources are supported"
    );
    let decoded = percent_encoding::percent_decode_str(uri).decode_utf8()?;
    let path = base
        .join(decoded.as_ref())
        .canonicalize()
        .with_context(|| format!("glTF resource {uri}"))?;
    ensure!(
        path.starts_with(base),
        "glTF resource escapes its source directory"
    );
    let data = fs::read(&path)?;
    files.insert(path, digest(&data));
    Ok(data)
}
fn generate_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.; 3]; positions.len()];
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        for index in t {
            for (dst, value) in normals[*index as usize].iter_mut().zip(n) {
                *dst += value;
            }
        }
    }
    for n in &mut normals {
        let len = n.iter().map(|v| v * v).sum::<f32>().sqrt();
        if len > 0. {
            for v in n {
                *v /= len;
            }
        }
    }
    normals
}
