use super::resources::*;
use anyhow::{Context, Result, ensure};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::*;
use std::collections::HashMap;
pub(super) struct UiPainter {
    pipeline: Retained<Pipeline>,
    sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    textures: HashMap<egui::TextureId, Retained<Texture>>,
}
impl UiPainter {
    pub fn new(device: &Device) -> Result<Self> {
        let compiled = crate::shader::compile(true)?;
        let pipeline = pipeline(device, &compiled, "ui_vs", Some("ui_fs"), true)?;
        let desc = MTLSamplerDescriptor::new();
        desc.setMinFilter(MTLSamplerMinMagFilter::Linear);
        desc.setMagFilter(MTLSamplerMinMagFilter::Linear);
        desc.setSAddressMode(MTLSamplerAddressMode::ClampToEdge);
        desc.setTAddressMode(MTLSamplerAddressMode::ClampToEdge);
        Ok(Self {
            pipeline,
            sampler: device
                .newSamplerStateWithDescriptor(&desc)
                .context("UI sampler")?,
            textures: HashMap::new(),
        })
    }
    pub fn update(&mut self, device: &Device, frame: &crate::UiFrame) -> Result<()> {
        for (id, deltas) in &frame.textures.set {
            for delta in deltas {
                let egui::ImageData::Color(image) = &delta.image;
                if delta.pos.is_none() {
                    self.textures.insert(
                        *id,
                        texture(
                            device,
                            [image.size[0] as u32, image.size[1] as u32],
                            MTLPixelFormat::RGBA8Unorm,
                            false,
                        )?,
                    );
                }
                let target = self
                    .textures
                    .get(id)
                    .context("unknown partial UI texture")?;
                let data: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                upload(target, delta.pos.unwrap_or([0, 0]), image.size, &data)?;
            }
        }
        Ok(())
    }
    pub fn free(&mut self, frame: &crate::UiFrame) {
        for id in &frame.textures.free {
            self.textures.remove(id);
        }
    }
    pub fn draw(
        &self,
        device: &Device,
        command: &ProtocolObject<dyn MTLCommandBuffer>,
        target: &Texture,
        frame: &crate::FramePacket,
    ) -> Result<()> {
        if frame.ui.primitives.is_empty() {
            return Ok(());
        }
        let pass = MTLRenderPassDescriptor::new();
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(target));
        color.setLoadAction(MTLLoadAction::Load);
        color.setStoreAction(MTLStoreAction::Store);
        let encoder = command
            .renderCommandEncoderWithDescriptor(&pass)
            .context("UI encoder")?;
        encoder.setRenderPipelineState(&self.pipeline);
        let ppp = frame.ui.pixels_per_point;
        ensure!(ppp.is_finite() && ppp > 0., "invalid UI scale");
        vertex_uniform(
            &encoder,
            &[
                frame.size[0] as f32 / ppp,
                frame.size[1] as f32 / ppp,
                0.,
                0.,
            ],
            1,
        );
        unsafe {
            encoder.setFragmentSamplerState_atIndex(Some(&self.sampler), 0);
        }
        let mut keep_alive = vec![];
        for p in &frame.ui.primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &p.primitive else {
                anyhow::bail!("native UI callbacks are not supported")
            };
            if mesh.indices.is_empty() {
                continue;
            }
            let clip = p.clip_rect * ppp;
            let x = clip.min.x.clamp(0., frame.size[0] as f32) as usize;
            let y = clip.min.y.clamp(0., frame.size[1] as f32) as usize;
            let right = clip.max.x.clamp(x as f32, frame.size[0] as f32) as usize;
            let bottom = clip.max.y.clamp(y as f32, frame.size[1] as f32) as usize;
            if right == x || bottom == y {
                continue;
            }
            let vertices: Vec<[f32; 8]> = mesh
                .vertices
                .iter()
                .map(|v| {
                    let c = v.color.to_array();
                    [
                        v.pos.x,
                        v.pos.y,
                        v.uv.x,
                        v.uv.y,
                        c[0] as f32 / 255.,
                        c[1] as f32 / 255.,
                        c[2] as f32 / 255.,
                        c[3] as f32 / 255.,
                    ]
                })
                .collect();
            let vb = buffer(device, &vertices)?;
            let ib = buffer(device, &mesh.indices)?;
            encoder.setScissorRect(MTLScissorRect {
                x,
                y,
                width: right - x,
                height: bottom - y,
            });
            unsafe {
                encoder.setVertexBuffer_offset_atIndex(Some(&vb), 0, 0);
                encoder.setFragmentTexture_atIndex(
                    Some(
                        self.textures
                            .get(&mesh.texture_id)
                            .context("missing UI texture")?,
                    ),
                    0,
                );
                encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
                    MTLPrimitiveType::Triangle,
                    mesh.indices.len(),
                    MTLIndexType::UInt32,
                    &ib,
                    0,
                );
            }
            keep_alive.extend([vb, ib]);
        }
        encoder.endEncoding();
        Ok(())
    }
}
