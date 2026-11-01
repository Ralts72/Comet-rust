use super::resources::*;
use crate::{
    FramePacket, Surface,
    shader::{Globals, Object},
    worker::FrameRenderer,
};
use anyhow::{Context, Result, ensure};
use comet_asset_data::product::Product;
use comet_core::AssetId;
use objc2::{
    rc::{Retained, autoreleasepool},
    runtime::ProtocolObject,
};
use objc2_core_foundation::CGSize;
use objc2_metal::*;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use std::collections::BTreeMap;
pub(crate) struct Renderer {
    device: Retained<Device>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    scene: Retained<Pipeline>,
    shadow_pipeline: Retained<Pipeline>,
    depth_state: Retained<ProtocolObject<dyn MTLDepthStencilState>>,
    sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    shadow_sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    shadow: Retained<Texture>,
    white: Retained<Texture>,
    meshes: BTreeMap<(AssetId, String), Mesh>,
    textures: BTreeMap<(AssetId, String), Retained<Texture>>,
    ui: super::ui::UiPainter,
    surface: Option<Surface>,
}
impl Renderer {
    pub fn new(surface: Option<Surface>) -> Result<Self> {
        autoreleasepool(|_| {
            let device = MTLCreateSystemDefaultDevice().context("Metal device unavailable")?;
            if let Some(surface) = &surface {
                let _anchor = &surface.window;
                let layer = unsafe { &*surface.layer.as_ptr().as_ptr().cast::<CAMetalLayer>() };
                layer.setDevice(Some(&device));
                layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
                layer.setFramebufferOnly(false);
            }
            let queue = device.newCommandQueue().context("Metal graphics queue")?;
            let compiled = crate::shader::compile(false)?;
            let scene = pipeline(&device, &compiled, "scene_vs", Some("scene_fs"), false)?;
            let shadow_pipeline = pipeline(&device, &compiled, "shadow_vs", None, false)?;
            let depth = MTLDepthStencilDescriptor::new();
            depth.setDepthCompareFunction(MTLCompareFunction::LessEqual);
            depth.setDepthWriteEnabled(true);
            let depth_state = device
                .newDepthStencilStateWithDescriptor(&depth)
                .context("depth state")?;
            let sampler_desc = MTLSamplerDescriptor::new();
            sampler_desc.setMinFilter(MTLSamplerMinMagFilter::Linear);
            sampler_desc.setMagFilter(MTLSamplerMinMagFilter::Linear);
            sampler_desc.setSAddressMode(MTLSamplerAddressMode::Repeat);
            sampler_desc.setTAddressMode(MTLSamplerAddressMode::Repeat);
            let sampler = device
                .newSamplerStateWithDescriptor(&sampler_desc)
                .context("material sampler")?;
            sampler_desc.setCompareFunction(MTLCompareFunction::LessEqual);
            sampler_desc.setSAddressMode(MTLSamplerAddressMode::ClampToEdge);
            sampler_desc.setTAddressMode(MTLSamplerAddressMode::ClampToEdge);
            let shadow_sampler = device
                .newSamplerStateWithDescriptor(&sampler_desc)
                .context("shadow sampler")?;
            let shadow = texture(&device, [1024, 1024], MTLPixelFormat::Depth32Float, true)?;
            let white = texture(&device, [1, 1], MTLPixelFormat::RGBA8Unorm_sRGB, false)?;
            upload(&white, [0, 0], [1, 1], &[255; 4])?;
            let ui = super::ui::UiPainter::new(&device)?;
            Ok(Self {
                device,
                queue,
                scene,
                shadow_pipeline,
                depth_state,
                sampler,
                shadow_sampler,
                shadow,
                white,
                meshes: BTreeMap::new(),
                textures: BTreeMap::new(),
                ui,
                surface,
            })
        })
    }
    fn prepare(&mut self, frame: &FramePacket) -> Result<()> {
        for draw in &frame.draws {
            let key = (draw.mesh.id, draw.mesh_revision.clone());
            if !self.meshes.contains_key(&key) {
                let Product::Mesh(mesh) = &draw.mesh.payload else {
                    anyhow::bail!("draw uses a non-mesh product")
                };
                let vertices = crate::shader::vertices(mesh);
                self.meshes.insert(
                    key,
                    Mesh {
                        vertices: buffer(&self.device, &vertices)?,
                        indices: buffer(&self.device, &mesh.indices)?,
                        count: mesh.indices.len(),
                    },
                );
            }
            if let (Some(product), Some(revision)) = (&draw.texture, &draw.texture_revision) {
                let key = (product.id, revision.clone());
                if !self.textures.contains_key(&key) {
                    let Product::Texture(t) = &product.payload else {
                        anyhow::bail!("draw uses a non-texture product")
                    };
                    let image = texture(
                        &self.device,
                        [t.width, t.height],
                        MTLPixelFormat::RGBA8Unorm_sRGB,
                        false,
                    )?;
                    upload(
                        &image,
                        [0, 0],
                        [t.width as usize, t.height as usize],
                        &t.rgba8,
                    )?;
                    self.textures.insert(key, image);
                }
            }
        }
        Ok(())
    }
    fn draws(
        &self,
        encoder: &Encoder,
        frame: &FramePacket,
        globals: &Globals,
        shadow: bool,
    ) -> Result<()> {
        encoder.setRenderPipelineState(if shadow {
            &self.shadow_pipeline
        } else {
            &self.scene
        });
        encoder.setDepthStencilState(Some(&self.depth_state));
        encoder.setCullMode(MTLCullMode::None);
        vertex_uniform(encoder, globals, 1);
        if !shadow {
            fragment_uniform(encoder, globals, 1);
        }
        for draw in &frame.draws {
            let object = Object::for_draw(draw, frame.shadows)?;
            vertex_uniform(encoder, &object, 2);
            let mesh = &self.meshes[&(draw.mesh.id, draw.mesh_revision.clone())];
            unsafe {
                encoder.setVertexBuffer_offset_atIndex(Some(&mesh.vertices), 0, 0);
                if !shadow {
                    fragment_uniform(encoder, &object, 2);
                    let image =
                        if let (Some(t), Some(rev)) = (&draw.texture, &draw.texture_revision) {
                            &self.textures[&(t.id, rev.clone())]
                        } else {
                            &self.white
                        };
                    encoder.setFragmentTexture_atIndex(Some(image), 0);
                    encoder.setFragmentTexture_atIndex(Some(&self.shadow), 1);
                    encoder.setFragmentSamplerState_atIndex(Some(&self.sampler), 0);
                    encoder.setFragmentSamplerState_atIndex(Some(&self.shadow_sampler), 1);
                }
                encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
                    MTLPrimitiveType::Triangle,
                    mesh.count,
                    MTLIndexType::UInt32,
                    &mesh.indices,
                    0,
                );
            }
        }
        Ok(())
    }
    fn draw_frame(&mut self, frame: &FramePacket) -> Result<(bool, f64)> {
        self.ui.update(&self.device, &frame.ui)?;
        if frame.size.contains(&0) {
            self.ui.free(&frame.ui);
            return Ok((false, 0.));
        }
        self.prepare(frame)?;
        let drawable = if let Some(surface) = &self.surface {
            let layer = unsafe { &*surface.layer.as_ptr().as_ptr().cast::<CAMetalLayer>() };
            layer.setDrawableSize(CGSize {
                width: frame.size[0] as f64,
                height: frame.size[1] as f64,
            });
            layer.nextDrawable()
        } else {
            None
        };
        if self.surface.is_some() && drawable.is_none() && frame.capture.is_none() {
            self.ui.free(&frame.ui);
            return Ok((false, 0.));
        }
        let target = if let Some(d) = &drawable {
            d.texture()
        } else {
            texture(&self.device, frame.size, MTLPixelFormat::BGRA8Unorm, false)?
        };
        let depth = texture(&self.device, frame.size, MTLPixelFormat::Depth32Float, true)?;
        let globals = Globals::for_frame(frame);
        let command = self.queue.commandBuffer().context("Metal frame command")?;
        let shadow_pass = MTLRenderPassDescriptor::new();
        shadow_pass.depthAttachment().setTexture(Some(&self.shadow));
        shadow_pass.depthAttachment().setClearDepth(1.);
        shadow_pass
            .depthAttachment()
            .setLoadAction(MTLLoadAction::Clear);
        shadow_pass
            .depthAttachment()
            .setStoreAction(MTLStoreAction::Store);
        let encoder = command
            .renderCommandEncoderWithDescriptor(&shadow_pass)
            .context("shadow encoder")?;
        encoder.setViewport(MTLViewport {
            originX: 0.,
            originY: 0.,
            width: 1024.,
            height: 1024.,
            znear: 0.,
            zfar: 1.,
        });
        if frame.shadows {
            self.draws(&encoder, frame, &globals, true)?;
        }
        encoder.endEncoding();
        let pass = MTLRenderPassDescriptor::new();
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(&target));
        color.setLoadAction(MTLLoadAction::Clear);
        color.setStoreAction(MTLStoreAction::Store);
        color.setClearColor(MTLClearColor {
            red: 0.075,
            green: 0.09,
            blue: 0.13,
            alpha: 1.,
        });
        pass.depthAttachment().setTexture(Some(&depth));
        pass.depthAttachment().setClearDepth(1.);
        pass.depthAttachment().setLoadAction(MTLLoadAction::Clear);
        pass.depthAttachment()
            .setStoreAction(MTLStoreAction::DontCare);
        let encoder = command
            .renderCommandEncoderWithDescriptor(&pass)
            .context("scene encoder")?;
        let [x, y, w, h] = frame.viewport;
        if w > 0 && h > 0 {
            encoder.setViewport(MTLViewport {
                originX: x as f64,
                originY: y as f64,
                width: w as f64,
                height: h as f64,
                znear: 0.,
                zfar: 1.,
            });
            encoder.setScissorRect(MTLScissorRect {
                x: x as usize,
                y: y as usize,
                width: w as usize,
                height: h as usize,
            });
            self.draws(&encoder, frame, &globals, false)?;
        }
        encoder.endEncoding();
        self.ui.draw(&self.device, &command, &target, frame)?;
        let row = (frame.size[0] as usize * 4).next_multiple_of(256);
        let readback = if frame.capture.is_some() {
            let b = self
                .device
                .newBufferWithLength_options(
                    row * frame.size[1] as usize,
                    MTLResourceOptions::StorageModeShared,
                )
                .context("readback buffer")?;
            let encoder = command.blitCommandEncoder().context("readback encoder")?;
            unsafe {
                encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(&target,0,0,MTLOrigin {x:0,y:0,z:0},MTLSize {width:frame.size[0] as usize,height:frame.size[1] as usize,depth:1},&b,0,row,row*frame.size[1] as usize);
            }
            encoder.endEncoding();
            Some(b)
        } else {
            None
        };
        if let Some(d) = &drawable {
            command.presentDrawable(ProtocolObject::from_ref(&**d));
        }
        command.commit();
        command.waitUntilCompleted();
        ensure!(
            command.status() == MTLCommandBufferStatus::Completed,
            "Metal frame failed: {:?}",
            command.error()
        );
        if let (Some(buffer), Some(path)) = (readback, &frame.capture) {
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    buffer.contents().as_ptr().cast::<u8>(),
                    row * frame.size[1] as usize,
                )
            };
            let mut rgba = vec![];
            for line in bytes.chunks_exact(row) {
                for p in line[..frame.size[0] as usize * 4].as_chunks::<4>().0 {
                    rgba.extend([p[2], p[1], p[0], p[3]]);
                }
            }
            super::super::write_png(path, frame.size, &rgba)?;
        }
        self.ui.free(&frame.ui);
        self.meshes.retain(|key, _| {
            frame
                .draws
                .iter()
                .any(|d| key.0 == d.mesh.id && key.1 == d.mesh_revision)
        });
        self.textures.retain(|key, _| {
            frame.draws.iter().any(|d| {
                d.texture.as_ref().is_some_and(|t| t.id == key.0)
                    && d.texture_revision.as_ref() == Some(&key.1)
            })
        });
        Ok((
            drawable.is_some(),
            (command.GPUEndTime() - command.GPUStartTime()) * 1000.,
        ))
    }
}
impl FrameRenderer for Renderer {
    fn name(&self) -> &'static str {
        "Metal"
    }
    fn render(&mut self, frame: &FramePacket) -> Result<(bool, f64)> {
        autoreleasepool(|_| self.draw_frame(frame))
    }
}
