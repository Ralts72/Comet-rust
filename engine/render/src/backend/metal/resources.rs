use anyhow::{Context, Result};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::*;
use std::{ffi::c_void, ptr::NonNull};
pub(super) type Device = ProtocolObject<dyn MTLDevice>;
pub(super) type Buffer = ProtocolObject<dyn MTLBuffer>;
pub(super) type Texture = ProtocolObject<dyn MTLTexture>;
pub(super) type Pipeline = ProtocolObject<dyn MTLRenderPipelineState>;
pub(super) type Encoder = ProtocolObject<dyn MTLRenderCommandEncoder>;
pub(super) struct Mesh {
    pub vertices: Retained<Buffer>,
    pub indices: Retained<Buffer>,
    pub influences: Option<Retained<Buffer>>,
    pub count: usize,
}
pub(super) fn buffer<T>(device: &Device, data: &[T]) -> Result<Retained<Buffer>> {
    // Internal callers pass initialized numeric arrays/u32, with no padding or pointers.
    unsafe {
        device.newBufferWithBytes_length_options(
            NonNull::new(data.as_ptr() as *mut c_void).context("empty buffer")?,
            std::mem::size_of_val(data),
            MTLResourceOptions::StorageModeShared,
        )
    }
    .context("Metal buffer allocation")
}
pub(super) fn texture(
    device: &Device,
    size: [u32; 2],
    format: MTLPixelFormat,
    depth: bool,
) -> Result<Retained<Texture>> {
    let desc = MTLTextureDescriptor::new();
    desc.setPixelFormat(format);
    unsafe {
        desc.setWidth(size[0] as usize);
        desc.setHeight(size[1] as usize);
    }
    desc.setStorageMode(if depth {
        MTLStorageMode::Private
    } else {
        MTLStorageMode::Shared
    });
    desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
    device
        .newTextureWithDescriptor(&desc)
        .context("Metal texture allocation")
}
pub(super) fn upload(
    texture: &Texture,
    origin: [usize; 2],
    size: [usize; 2],
    rgba: &[u8],
) -> Result<()> {
    anyhow::ensure!(
        origin[0] + size[0] <= texture.width()
            && origin[1] + size[1] <= texture.height()
            && rgba.len() == size[0] * size[1] * 4,
        "invalid texture upload"
    );
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
            MTLRegion {
                origin: MTLOrigin {
                    x: origin[0],
                    y: origin[1],
                    z: 0,
                },
                size: MTLSize {
                    width: size[0],
                    height: size[1],
                    depth: 1,
                },
            },
            0,
            NonNull::new(rgba.as_ptr() as *mut c_void).context("empty texture upload")?,
            size[0] * 4,
        );
    }
    Ok(())
}
pub(super) fn pipeline(
    device: &Device,
    compiled: &crate::shader::Compiled,
    vertex: &str,
    fragment: Option<&str>,
    ui: bool,
) -> Result<Retained<Pipeline>> {
    let library = device
        .newLibraryWithSource_options_error(&NSString::from_str(&compiled.msl), None)
        .map_err(|e| anyhow::anyhow!("MSL compilation: {e}"))?;
    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setVertexFunction(
        library
            .newFunctionWithName(&NSString::from_str(&compiled.names[vertex]))
            .as_deref(),
    );
    if let Some(fragment) = fragment {
        descriptor.setFragmentFunction(
            library
                .newFunctionWithName(&NSString::from_str(&compiled.names[fragment]))
                .as_deref(),
        );
    }
    let layout = MTLVertexDescriptor::new();
    let stride = 32;
    for (index, (offset, format)) in (if ui {
        vec![
            (0, MTLVertexFormat::Float2),
            (8, MTLVertexFormat::Float2),
            (16, MTLVertexFormat::Float4),
        ]
    } else {
        vec![
            (0, MTLVertexFormat::Float3),
            (12, MTLVertexFormat::Float3),
            (24, MTLVertexFormat::Float2),
        ]
    })
    .into_iter()
    .enumerate()
    {
        let attribute = unsafe { layout.attributes().objectAtIndexedSubscript(index) };
        unsafe {
            attribute.setOffset(offset);
            attribute.setBufferIndex(0);
        }
        attribute.setFormat(format);
    }
    unsafe {
        layout
            .layouts()
            .objectAtIndexedSubscript(0)
            .setStride(stride);
    }
    if vertex.starts_with("skinned_") {
        for (index, offset, format) in [
            (3, 0, MTLVertexFormat::UInt4),
            (4, 16, MTLVertexFormat::Float4),
        ] {
            let attribute = unsafe { layout.attributes().objectAtIndexedSubscript(index) };
            unsafe {
                attribute.setOffset(offset);
                attribute.setBufferIndex(3);
            }
            attribute.setFormat(format);
        }
        unsafe {
            layout.layouts().objectAtIndexedSubscript(3).setStride(32);
        }
    }
    descriptor.setVertexDescriptor(Some(&layout));
    if fragment.is_some() {
        let color = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        color.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        if ui {
            color.setBlendingEnabled(true);
            color.setSourceRGBBlendFactor(MTLBlendFactor::One);
            color.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            color.setSourceAlphaBlendFactor(MTLBlendFactor::One);
            color.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        }
    }
    if !ui {
        descriptor.setDepthAttachmentPixelFormat(MTLPixelFormat::Depth32Float);
    }
    device
        .newRenderPipelineStateWithDescriptor_error(&descriptor)
        .map_err(|e| anyhow::anyhow!("Metal render pipeline: {e}"))
}
pub(super) fn vertex_uniform<T>(encoder: &Encoder, value: &T, index: usize) {
    // The encoder copies these repr(C), numeric-only uniform bytes immediately.
    unsafe {
        encoder.setVertexBytes_length_atIndex(
            NonNull::from(value).cast::<c_void>(),
            std::mem::size_of::<T>(),
            index,
        );
    }
}
pub(super) fn fragment_uniform<T>(encoder: &Encoder, value: &T, index: usize) {
    unsafe {
        encoder.setFragmentBytes_length_atIndex(
            NonNull::from(value).cast::<c_void>(),
            std::mem::size_of::<T>(),
            index,
        );
    }
}
