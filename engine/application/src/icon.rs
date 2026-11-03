use anyhow::{Context, Result, ensure};
use std::io::Cursor;
use winit::window::Icon;

/// Hosts supply their own embedded RGBA PNG; macOS uses the enclosing App's ICNS instead.
pub(crate) fn decode(bytes: &[u8]) -> Result<Icon> {
    let mut reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .context("读取应用图标失败，请确认已执行 git lfs pull")?;
    ensure!(
        reader.info().width <= 1024 && reader.info().height <= 1024,
        "应用图标尺寸超过 1024 像素"
    );
    let mut rgba = vec![0; reader.output_buffer_size().context("应用图标尺寸无效")?];
    let info = reader.next_frame(&mut rgba)?;
    ensure!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "应用图标必须使用 8 位 RGBA PNG"
    );
    rgba.truncate(info.buffer_size());
    Ok(Icon::from_rgba(rgba, info.width, info.height)?)
}
