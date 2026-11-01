use super::{
    commands::viewport,
    device::Device,
    pipeline::Pipelines,
    resources::{Buffer, DescriptorPool, Image},
};
use crate::FramePacket;
use anyhow::{Context, Result};
use ash::vk;
use std::sync::Arc;
pub(super) fn draw_ui(
    ctx: &Arc<Device>,
    pipelines: &Pipelines,
    textures: &std::collections::HashMap<egui::TextureId, Image>,
    command: vk::CommandBuffer,
    frame: &FramePacket,
    pool: &DescriptorPool,
    keep: &mut Vec<Buffer>,
) -> Result<()> {
    let ppp = frame.ui.pixels_per_point;
    if frame.ui.primitives.is_empty() {
        return Ok(());
    }
    anyhow::ensure!(ppp.is_finite() && ppp > 0., "invalid UI scale");
    let screen = Buffer::from_slice(
        ctx.clone(),
        &[[
            frame.size[0] as f32 / ppp,
            frame.size[1] as f32 / ppp,
            0.,
            0.,
        ]],
        vk::BufferUsageFlags::UNIFORM_BUFFER,
    )?;
    unsafe {
        ctx.handle
            .cmd_bind_pipeline(command, vk::PipelineBindPoint::GRAPHICS, pipelines.ui);
    }
    viewport(&ctx.handle, command, [0, 0, frame.size[0], frame.size[1]]);
    for p in &frame.ui.primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &p.primitive else {
            anyhow::bail!("native UI callbacks are not supported")
        };
        if mesh.indices.is_empty() {
            continue;
        }
        let clip = p.clip_rect * ppp;
        let x = clip.min.x.clamp(0., frame.size[0] as f32) as u32;
        let y = clip.min.y.clamp(0., frame.size[1] as f32) as u32;
        let right = clip.max.x.clamp(x as f32, frame.size[0] as f32) as u32;
        let bottom = clip.max.y.clamp(y as f32, frame.size[1] as f32) as u32;
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
        let vb = Buffer::from_slice(ctx.clone(), &vertices, vk::BufferUsageFlags::VERTEX_BUFFER)?;
        let ib = Buffer::from_slice(
            ctx.clone(),
            &mesh.indices,
            vk::BufferUsageFlags::INDEX_BUFFER,
        )?;
        let set = pool.allocate(pipelines.ui_set)?;
        let buffer = [vk::DescriptorBufferInfo {
            buffer: screen.handle,
            offset: 0,
            range: screen.size as u64,
        }];
        let image = [vk::DescriptorImageInfo::default()
            .image_view(
                textures
                    .get(&mesh.texture_id)
                    .context("missing UI draw texture")?
                    .view,
            )
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let sampler = [vk::DescriptorImageInfo::default().sampler(pipelines.ui_sampler)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler),
        ];
        unsafe {
            ctx.handle.update_descriptor_sets(&writes, &[]);
            ctx.handle.cmd_set_scissor(
                command,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D {
                        x: x as i32,
                        y: y as i32,
                    },
                    extent: vk::Extent2D {
                        width: right - x,
                        height: bottom - y,
                    },
                }],
            );
            ctx.handle.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::GRAPHICS,
                pipelines.ui_layout,
                0,
                &[set],
                &[],
            );
            ctx.handle
                .cmd_bind_vertex_buffers(command, 0, &[vb.handle], &[0]);
            ctx.handle
                .cmd_bind_index_buffer(command, ib.handle, 0, vk::IndexType::UINT32);
            ctx.handle
                .cmd_draw_indexed(command, mesh.indices.len() as u32, 1, 0, 0, 0);
        }
        keep.extend([vb, ib]);
    }
    keep.push(screen);
    Ok(())
}
