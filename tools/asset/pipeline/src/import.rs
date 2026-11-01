use anyhow::{Context, Result, ensure};
use comet_asset_data::product::{Product, TextureData};
use std::{io::Cursor, path::Path};

pub(crate) fn supported(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    [
        ".scene",
        ".prefab",
        ".mesh",
        ".material",
        ".png",
        ".gltf",
        ".glb",
    ]
    .iter()
    .any(|s| name.ends_with(s))
}
pub(crate) fn bundle(
    path: &Path,
    bytes: &[u8],
    id: comet_core::AssetId,
) -> Result<crate::gltf::Imported> {
    if path.extension().is_some_and(|e| e == "gltf" || e == "glb") {
        crate::gltf::import(path, bytes, id)
    } else {
        Ok(crate::gltf::Imported {
            products: vec![(id, import(path, bytes)?)],
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
    } else {
        anyhow::bail!("unsupported source {}", path.display())
    };
    value.validate()?;
    Ok(value)
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
