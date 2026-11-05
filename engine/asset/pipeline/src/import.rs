use anyhow::{Context, Result, ensure};
use comet_asset_data::product::{
    AssetKind, MAX_SCRIPT_BYTES, Product, SCRIPT_VERSION, ScriptData, TextureData,
};
use comet_core::AssetId;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub(crate) struct ImportedProduct {
    pub id: AssetId,
    pub payload: Product,
    pub label: String,
}
pub(crate) struct Imported {
    pub products: Vec<ImportedProduct>,
    pub files: BTreeMap<PathBuf, String>,
}

pub(crate) fn source_kind(path: &Path) -> Option<AssetKind> {
    match path.extension()?.to_str()? {
        "scene" => Some(AssetKind::Scene),
        "prefab" | "gltf" | "glb" => Some(AssetKind::Prefab),
        "mesh" => Some(AssetKind::Mesh),
        "material" => Some(AssetKind::Material),
        "png" => Some(AssetKind::Texture),
        "rhai" => Some(AssetKind::Script),
        "nav" => Some(AssetKind::Navigation),
        "wav" => Some(AssetKind::Audio),
        _ => None,
    }
}
pub(crate) fn bundle(path: &Path, bytes: &[u8], id: AssetId) -> Result<Imported> {
    if path.extension().is_some_and(|e| e == "gltf" || e == "glb") {
        crate::gltf::import(path, bytes, id)
    } else {
        Ok(Imported {
            products: vec![ImportedProduct {
                id,
                payload: import(path, bytes)?,
                label: path
                    .file_name()
                    .context("missing source filename")?
                    .to_string_lossy()
                    .into_owned(),
            }],
            files: Default::default(),
        })
    }
}
pub(crate) fn import(path: &Path, bytes: &[u8]) -> Result<Product> {
    let name = path
        .file_name()
        .context("missing source filename")?
        .to_string_lossy();
    let value = if name.ends_with(".scene") {
        Product::Scene(serde_json::from_slice(bytes)?)
    } else if name.ends_with(".prefab") {
        Product::Prefab(serde_json::from_slice(bytes)?)
    } else if name.ends_with(".mesh") {
        Product::Mesh(serde_json::from_slice(bytes)?)
    } else if name.ends_with(".material") {
        Product::Material(serde_json::from_slice(bytes)?)
    } else if name.ends_with(".png") {
        Product::Texture(png(bytes)?)
    } else if name.ends_with(".rhai") {
        anyhow::ensure!(
            bytes.len() <= MAX_SCRIPT_BYTES,
            "script source exceeds 64 KiB"
        );
        Product::Script(ScriptData {
            version: SCRIPT_VERSION,
            source: std::str::from_utf8(bytes)
                .context("script source must be UTF-8")?
                .to_owned(),
        })
    } else if name.ends_with(".wav") {
        Product::Audio(crate::audio::import(bytes)?)
    } else if name.ends_with(".nav") {
        ensure!(
            bytes.len() <= comet_asset_data::navigation::MAX_NAVIGATION_BYTES,
            "navigation source exceeds its size budget"
        );
        Product::Navigation(serde_json::from_slice(bytes)?)
    } else {
        anyhow::bail!("unsupported source {}", path.display())
    };
    value.validate()?;
    Ok(value)
}
pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    if path.extension().is_some_and(|extension| extension == "wav") {
        crate::audio::read(path)
    } else if path
        .extension()
        .is_some_and(|extension| extension == "gltf" || extension == "glb")
    {
        read_bounded(path, comet_asset_data::MAX_ANIMATION_BYTES)
    } else {
        std::fs::read(path).with_context(|| format!("read {}", path.display()))
    }
}
pub(super) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= limit as u64,
        "source byte budget exceeded: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "source grew beyond byte budget: {}",
        path.display()
    );
    Ok(bytes)
}
fn png(bytes: &[u8]) -> Result<TextureData> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .context("PNG dimensions overflow")?
    ];
    let frame = reader.next_frame(&mut buffer)?;
    ensure!(
        frame.bit_depth == png::BitDepth::Eight,
        "expected 8-bit decoded PNG"
    );
    let source = &buffer[..frame.buffer_size()];
    let mut rgba8 = Vec::new();
    match frame.color_type {
        png::ColorType::Rgba => rgba8.extend_from_slice(source),
        png::ColorType::Rgb => {
            for p in source.as_chunks::<3>().0 {
                rgba8.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for g in source {
                rgba8.extend_from_slice(&[*g, *g, *g, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in source.as_chunks::<2>().0 {
                rgba8.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        png::ColorType::Indexed => anyhow::bail!("PNG palette was not expanded"),
    }
    let value = TextureData {
        width: frame.width,
        height: frame.height,
        rgba8,
    };
    value.validate()?;
    Ok(value)
}
