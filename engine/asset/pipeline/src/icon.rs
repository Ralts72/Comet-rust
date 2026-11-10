use anyhow::{Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::{Product, TextureData};
use comet_core::AssetId;
use image::{
    ImageBuffer, ImageEncoder, Rgba,
    imageops::{FilterType, resize},
};

#[cfg(windows)]
mod windows;

pub(crate) struct GameIcon {
    pub png: Vec<u8>,
    pub icns: Vec<u8>,
    #[cfg(windows)]
    images: Vec<(u16, Vec<u8>)>,
}
impl GameIcon {
    pub fn load(assets: &mut AssetStore, id: AssetId) -> Result<Self> {
        let product = assets.load(id)?;
        let texture = validated_texture(&product.payload)?;
        let image = ImageBuffer::<Rgba<u8>, _>::from_raw(
            texture.width,
            texture.height,
            texture.rgba8.as_slice(),
        )
        .unwrap();
        let encode = |size| -> Result<Vec<u8>> {
            let scaled = resize(&image, size, size, FilterType::Lanczos3);
            let mut bytes = vec![];
            image::codecs::png::PngEncoder::new(&mut bytes).write_image(
                &scaled,
                size,
                size,
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(bytes)
        };
        let png = encode(256)?;
        let mut icns = b"icns\0\0\0\0".to_vec();
        for (size, kind) in [
            (128, b"ic07"),
            (256, b"ic08"),
            (512, b"ic09"),
            (1024, b"ic10"),
        ] {
            let bytes = encode(size)?;
            icns.extend_from_slice(kind);
            icns.extend_from_slice(&((bytes.len() + 8) as u32).to_be_bytes());
            icns.extend_from_slice(&bytes);
        }
        let length = icns.len() as u32;
        icns[4..8].copy_from_slice(&length.to_be_bytes());
        #[cfg(windows)]
        let images = [16, 24, 32, 48, 64, 128, 256]
            .into_iter()
            .map(|size| Ok((size as u16, encode(size)?)))
            .collect::<Result<_>>()?;
        Ok(Self {
            png,
            icns,
            #[cfg(windows)]
            images,
        })
    }
    #[cfg(windows)]
    pub fn embed(&self, binary: &std::path::Path) -> Result<()> {
        windows::embed(binary, &self.images)
    }
}
pub(crate) fn validate(assets: &mut AssetStore, id: AssetId) -> Result<()> {
    validated_texture(&assets.load(id)?.payload)?;
    Ok(())
}
fn validated_texture(product: &Product) -> Result<&TextureData> {
    let Product::Texture(texture) = product else {
        anyhow::bail!("游戏图标必须引用纹理资产");
    };
    texture.validate()?;
    ensure!(
        texture.width == texture.height && (16..=1024).contains(&texture.width),
        "游戏图标必须为 16～1024 像素的正方形，建议使用 1024 像素 RGBA PNG"
    );
    Ok(texture)
}
