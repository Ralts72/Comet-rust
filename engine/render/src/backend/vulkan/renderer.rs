use super::commands::{begin_pass, viewport};
use super::device::{Device, Swapchain};
use super::pipeline::Pipelines;
use super::resources::*;
use crate::{
    FramePacket, Surface,
    shader::{Globals, Object},
    worker::FrameRenderer,
};
use anyhow::{Context, Result};
use ash::vk;
use comet_asset_data::product::Product;
use comet_core::AssetId;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
struct Mesh {
    vertices: Buffer,
    indices: Buffer,
    count: u32,
}
pub(crate) struct Renderer {
    pipelines: Pipelines,
    meshes: BTreeMap<(AssetId, String), Mesh>,
    textures: BTreeMap<(AssetId, String), Image>,
    ui_textures: HashMap<egui::TextureId, Image>,
    shadow: Image,
    white: Image,
    swapchain: Option<Swapchain>,
    last_size: [u32; 2],
    ctx: Arc<Device>,
}
impl Renderer {
    pub fn new(surface: Option<Surface>) -> Result<Self> {
        let ctx = Device::new(surface)?;
        let pipelines = Pipelines::new(ctx.clone())?;
        let shadow = Image::new(
            ctx.clone(),
            [1024, 1024],
            vk::Format::D32_SFLOAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            true,
        )?;
        let mut white = Image::new(
            ctx.clone(),
            [1, 1],
            vk::Format::R8G8B8A8_SRGB,
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
            false,
        )?;
        white.upload([0, 0], [1, 1], &[255; 4])?;
        Ok(Self {
            pipelines,
            meshes: BTreeMap::new(),
            textures: BTreeMap::new(),
            ui_textures: HashMap::new(),
            shadow,
            white,
            swapchain: None,
            last_size: [0, 0],
            ctx,
        })
    }
    fn prepare(&mut self, frame: &FramePacket) -> Result<()> {
        for (id, deltas) in &frame.ui.textures.set {
            for delta in deltas {
                let egui::ImageData::Color(image) = &delta.image;
                if delta.pos.is_none() {
                    self.ui_textures.insert(
                        *id,
                        Image::new(
                            self.ctx.clone(),
                            [image.size[0] as u32, image.size[1] as u32],
                            vk::Format::R8G8B8A8_UNORM,
                            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
                            false,
                        )?,
                    );
                }
                let rgba: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                self.ui_textures
                    .get_mut(id)
                    .context("unknown UI texture update")?
                    .upload(delta.pos.unwrap_or([0, 0]), image.size, &rgba)?;
            }
        }
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
                        vertices: Buffer::from_slice(
                            self.ctx.clone(),
                            &vertices,
                            vk::BufferUsageFlags::VERTEX_BUFFER,
                        )?,
                        indices: Buffer::from_slice(
                            self.ctx.clone(),
                            &mesh.indices,
                            vk::BufferUsageFlags::INDEX_BUFFER,
                        )?,
                        count: mesh.indices.len() as u32,
                    },
                );
            }
            if let (Some(p), Some(revision)) = (&draw.texture, &draw.texture_revision) {
                let key = (p.id, revision.clone());
                if !self.textures.contains_key(&key) {
                    let Product::Texture(t) = &p.payload else {
                        anyhow::bail!("draw uses a non-texture product")
                    };
                    let mut image = Image::new(
                        self.ctx.clone(),
                        [t.width, t.height],
                        vk::Format::R8G8B8A8_SRGB,
                        vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
                        false,
                    )?;
                    image.upload([0, 0], [t.width as usize, t.height as usize], &t.rgba8)?;
                    self.textures.insert(key, image);
                }
            }
        }
        Ok(())
    }

    fn draws(
        &self,
        command: vk::CommandBuffer,
        frame: &FramePacket,
        sets: &[vk::DescriptorSet],
        shadow: bool,
    ) {
        unsafe {
            self.ctx.handle.cmd_bind_pipeline(
                command,
                vk::PipelineBindPoint::GRAPHICS,
                if shadow {
                    self.pipelines.shadow
                } else {
                    self.pipelines.scene
                },
            );
            for (draw, set) in frame.draws.iter().zip(sets) {
                let mesh = &self.meshes[&(draw.mesh.id, draw.mesh_revision.clone())];
                self.ctx.handle.cmd_bind_descriptor_sets(
                    command,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipelines.scene_layout,
                    0,
                    &[*set],
                    &[],
                );
                self.ctx
                    .handle
                    .cmd_bind_vertex_buffers(command, 0, &[mesh.vertices.handle], &[0]);
                self.ctx.handle.cmd_bind_index_buffer(
                    command,
                    mesh.indices.handle,
                    0,
                    vk::IndexType::UINT32,
                );
                self.ctx
                    .handle
                    .cmd_draw_indexed(command, mesh.count, 1, 0, 0, 0);
            }
        }
    }

    fn draw_frame(&mut self, frame: &FramePacket) -> Result<(bool, f64)> {
        self.prepare(frame)?;
        if frame.size.contains(&0) {
            for id in &frame.ui.textures.free {
                self.ui_textures.remove(id);
            }
            return Ok((false, 0.));
        }
        if self.ctx.surface() != vk::SurfaceKHR::null()
            && (self.swapchain.is_none() || self.last_size != frame.size)
        {
            self.swapchain = Some(Swapchain::new(self.ctx.clone(), frame.size)?);
            self.last_size = frame.size;
        }
        let sync = Sync::new(self.ctx.clone())?;
        let acquired = if let Some(swapchain) = &self.swapchain {
            match unsafe {
                swapchain.api.acquire_next_image(
                    swapchain.handle,
                    u64::MAX,
                    sync.available,
                    vk::Fence::null(),
                )
            } {
                Ok(value) => Some(value),
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.swapchain = None;
                    return Ok((false, 0.));
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            None
        };
        let target = Image::new(
            self.ctx.clone(),
            frame.size,
            vk::Format::B8G8R8A8_UNORM,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            false,
        )?;
        let depth = Image::new(
            self.ctx.clone(),
            frame.size,
            vk::Format::D32_SFLOAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            true,
        )?;
        let shadow_fb = Framebuffer::new(
            self.ctx.clone(),
            self.pipelines.shadow_pass,
            &[self.shadow.view],
            [1024, 1024],
        )?;
        let main_fb = Framebuffer::new(
            self.ctx.clone(),
            self.pipelines.main_pass,
            &[target.view, depth.view],
            frame.size,
        )?;
        let ui_fb = Framebuffer::new(
            self.ctx.clone(),
            self.pipelines.ui_pass,
            &[target.view],
            frame.size,
        )?;
        let pool = DescriptorPool::new(
            self.ctx.clone(),
            (frame.draws.len() + frame.ui.primitives.len() + 1) as u32,
        )?;
        let globals = Globals::for_frame(frame);
        let global = Buffer::from_slice(
            self.ctx.clone(),
            &[globals],
            vk::BufferUsageFlags::UNIFORM_BUFFER,
        )?;
        let mut keep = vec![global];
        let mut sets = vec![];
        for draw in &frame.draws {
            let object = Object::for_draw(draw, frame.shadows)?;
            let buffer = Buffer::from_slice(
                self.ctx.clone(),
                &[object],
                vk::BufferUsageFlags::UNIFORM_BUFFER,
            )?;
            let image = if let (Some(p), Some(rev)) = (&draw.texture, &draw.texture_revision) {
                &self.textures[&(p.id, rev.clone())]
            } else {
                &self.white
            };
            sets.push(
                self.pipelines
                    .scene_set(&pool, &keep[0], &buffer, image, &self.shadow)?,
            );
            keep.push(buffer);
        }
        let command = self.ctx.begin()?;
        let query = Timestamp::new(self.ctx.clone())?;
        query.start(command);
        let depth_clear = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.,
                stencil: 0,
            },
        };
        begin_pass(
            &self.ctx.handle,
            command,
            self.pipelines.shadow_pass,
            shadow_fb.handle,
            [1024, 1024],
            &[depth_clear],
        );
        viewport(&self.ctx.handle, command, [0, 0, 1024, 1024]);
        if frame.shadows {
            self.draws(command, frame, &sets, true);
        }
        unsafe {
            self.ctx.handle.cmd_end_render_pass(command);
        }
        let clear = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.075, 0.09, 0.13, 1.],
            },
        };
        begin_pass(
            &self.ctx.handle,
            command,
            self.pipelines.main_pass,
            main_fb.handle,
            frame.size,
            &[clear, depth_clear],
        );
        if frame.viewport[2] > 0 && frame.viewport[3] > 0 {
            viewport(&self.ctx.handle, command, frame.viewport);
            self.draws(command, frame, &sets, false);
        }
        unsafe {
            self.ctx.handle.cmd_end_render_pass(command);
        }
        begin_pass(
            &self.ctx.handle,
            command,
            self.pipelines.ui_pass,
            ui_fb.handle,
            frame.size,
            &[],
        );
        super::ui::draw_ui(
            &self.ctx,
            &self.pipelines,
            &self.ui_textures,
            command,
            frame,
            &pool,
            &mut keep,
        )?;
        unsafe {
            self.ctx.handle.cmd_end_render_pass(command);
        }
        let readback = if frame.capture.is_some() {
            let buffer = Buffer::new(
                self.ctx.clone(),
                frame.size[0] as usize * frame.size[1] as usize * 4,
                vk::BufferUsageFlags::TRANSFER_DST,
            )?;
            let region = vk::BufferImageCopy::default()
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_extent(vk::Extent3D {
                    width: frame.size[0],
                    height: frame.size[1],
                    depth: 1,
                });
            unsafe {
                self.ctx.handle.cmd_copy_image_to_buffer(
                    command,
                    target.handle,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    buffer.handle,
                    &[region],
                );
            }
            Some(buffer)
        } else {
            None
        };
        if let (Some((index, _)), Some(swap)) = (acquired, &self.swapchain) {
            let image = swap.images[index as usize];
            transition(
                &self.ctx.handle,
                command,
                image,
                vk::ImageAspectFlags::COLOR,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            );
            let layers = vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            };
            let region = vk::ImageBlit::default()
                .src_subresource(layers)
                .src_offsets([
                    vk::Offset3D::default(),
                    vk::Offset3D {
                        x: frame.size[0] as i32,
                        y: frame.size[1] as i32,
                        z: 1,
                    },
                ])
                .dst_subresource(layers)
                .dst_offsets([
                    vk::Offset3D::default(),
                    vk::Offset3D {
                        x: swap.extent.width as i32,
                        y: swap.extent.height as i32,
                        z: 1,
                    },
                ]);
            unsafe {
                self.ctx.handle.cmd_blit_image(
                    command,
                    target.handle,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                    vk::Filter::NEAREST,
                );
            }
            transition(
                &self.ctx.handle,
                command,
                image,
                vk::ImageAspectFlags::COLOR,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::PRESENT_SRC_KHR,
            );
        }
        query.end(command);
        self.ctx.finish(
            command,
            if acquired.is_some() {
                std::slice::from_ref(&sync.available)
            } else {
                &[]
            },
            if acquired.is_some() {
                std::slice::from_ref(&sync.finished)
            } else {
                &[]
            },
        )?;
        let mut recreate = acquired.is_some_and(|(_, suboptimal)| suboptimal);
        if let (Some((index, _)), Some(swap)) = (acquired, &self.swapchain) {
            let indices = [index];
            let handles = [swap.handle];
            let wait = [sync.finished];
            match unsafe {
                swap.api.queue_present(
                    self.ctx.queue,
                    &vk::PresentInfoKHR::default()
                        .wait_semaphores(&wait)
                        .swapchains(&handles)
                        .image_indices(&indices),
                )
            } {
                Ok(suboptimal) => recreate |= suboptimal,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => recreate = true,
                Err(e) => return Err(e.into()),
            }
            // GPU frames are serialized; presentation has consumed its semaphore before reuse.
            unsafe { self.ctx.handle.queue_wait_idle(self.ctx.queue) }?;
        }
        if recreate {
            self.swapchain = None;
        }
        if let (Some(buffer), Some(path)) = (readback, &frame.capture) {
            let data = buffer.read()?;
            let mut rgba = Vec::with_capacity(data.len());
            for p in data.as_chunks::<4>().0 {
                rgba.extend([p[2], p[1], p[0], p[3]]);
            }
            super::super::write_png(path, frame.size, &rgba)?;
        }
        for id in &frame.ui.textures.free {
            self.ui_textures.remove(id);
        }
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
        Ok((acquired.is_some(), query.milliseconds()?))
    }
}
impl FrameRenderer for Renderer {
    fn name(&self) -> &'static str {
        "Vulkan"
    }
    fn render(&mut self, frame: &FramePacket) -> Result<(bool, f64)> {
        self.draw_frame(frame)
    }
}
