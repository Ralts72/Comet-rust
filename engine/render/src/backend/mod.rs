use anyhow::Result;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;
pub trait SurfaceWindow: HasWindowHandle + HasDisplayHandle + Send + Sync {}
impl<T: HasWindowHandle + HasDisplayHandle + Send + Sync> SurfaceWindow for T {}
pub struct Surface {
    #[cfg(target_os = "macos")]
    pub(crate) layer: raw_window_metal::Layer,
    pub(crate) window: Arc<dyn SurfaceWindow>,
}
impl Surface {
    pub fn new(window: Arc<dyn SurfaceWindow>) -> Result<Self> {
        #[cfg(target_os = "macos")]
        {
            use anyhow::Context;
            objc2::MainThreadMarker::new().context("create Metal surface on the main thread")?;
            let raw_window_handle::RawWindowHandle::AppKit(handle) =
                window.window_handle()?.as_raw()
            else {
                anyhow::bail!("AppKit window required")
            };
            // The borrowed handle is valid; Layer retains its sublayer and observes view sizing.
            // Its upstream Send/Sync contract allows moving it to the worker. The window anchor
            // remains alive until the renderer and layer have been destroyed.
            let layer = unsafe { raw_window_metal::Layer::from_ns_view(handle.ns_view) };
            Ok(Self { window, layer })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self { window })
        }
    }
}
#[cfg(target_os = "macos")]
mod metal;
#[cfg(target_os = "macos")]
pub(crate) use metal::Renderer;
#[cfg(not(target_os = "macos"))]
mod vulkan;
#[cfg(not(target_os = "macos"))]
pub(crate) use vulkan::Renderer;

pub(crate) fn write_png(path: &std::path::Path, size: [u32; 2], rgba: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut encoder = png::Encoder::new(std::fs::File::create(path)?, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
