use super::device::Device;
use anyhow::{Result, ensure};
use ash::vk;
use std::{ptr, sync::Arc};
pub(super) struct Buffer {
    ctx: Arc<Device>,
    pub handle: vk::Buffer,
    memory: vk::DeviceMemory,
    pub size: usize,
}
impl Buffer {
    pub fn new(ctx: Arc<Device>, size: usize, usage: vk::BufferUsageFlags) -> Result<Self> {
        let handle = unsafe {
            ctx.handle.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size.max(4) as u64)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }?;
        let requirements = unsafe { ctx.handle.get_buffer_memory_requirements(handle) };
        let result = (|| -> Result<vk::DeviceMemory> {
            let memory_type = ctx.memory_type(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?;
            Ok(unsafe {
                ctx.handle.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirements.size)
                        .memory_type_index(memory_type),
                    None,
                )
            }?)
        })();
        let memory = match result {
            Ok(m) => m,
            Err(e) => {
                unsafe {
                    ctx.handle.destroy_buffer(handle, None);
                }
                return Err(e);
            }
        };
        if let Err(e) = unsafe { ctx.handle.bind_buffer_memory(handle, memory, 0) } {
            unsafe {
                ctx.handle.destroy_buffer(handle, None);
                ctx.handle.free_memory(memory, None);
            }
            return Err(e.into());
        }
        Ok(Self {
            ctx,
            handle,
            memory,
            size,
        })
    }
    pub fn from_slice<T>(
        ctx: Arc<Device>,
        data: &[T],
        usage: vk::BufferUsageFlags,
    ) -> Result<Self> {
        let b = Self::new(ctx, std::mem::size_of_val(data), usage)?;
        let target = unsafe {
            b.ctx
                .handle
                .map_memory(b.memory, 0, b.size as u64, vk::MemoryMapFlags::empty())
        }?;
        // Only initialized numeric arrays and repr(C) numeric uniforms are supplied internally.
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), target.cast::<u8>(), b.size);
            b.ctx.handle.unmap_memory(b.memory);
        }
        Ok(b)
    }
    pub fn read(&self) -> Result<Vec<u8>> {
        let source = unsafe {
            self.ctx.handle.map_memory(
                self.memory,
                0,
                self.size as u64,
                vk::MemoryMapFlags::empty(),
            )
        }?;
        let bytes = unsafe { std::slice::from_raw_parts(source.cast::<u8>(), self.size) }.to_vec();
        unsafe {
            self.ctx.handle.unmap_memory(self.memory);
        }
        Ok(bytes)
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.ctx.handle.destroy_buffer(self.handle, None);
            self.ctx.handle.free_memory(self.memory, None);
        }
    }
}
pub(super) struct Image {
    ctx: Arc<Device>,
    pub handle: vk::Image,
    memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub size: [u32; 2],
    pub layout: vk::ImageLayout,
    pub aspect: vk::ImageAspectFlags,
}
impl Image {
    pub fn new(
        ctx: Arc<Device>,
        size: [u32; 2],
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        depth: bool,
    ) -> Result<Self> {
        let handle = unsafe {
            ctx.handle.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D {
                        width: size[0],
                        height: size[1],
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
        }?;
        let requirements = unsafe { ctx.handle.get_image_memory_requirements(handle) };
        let result = (|| -> Result<vk::DeviceMemory> {
            Ok(unsafe {
                ctx.handle.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirements.size)
                        .memory_type_index(ctx.memory_type(
                            requirements.memory_type_bits,
                            vk::MemoryPropertyFlags::DEVICE_LOCAL,
                        )?),
                    None,
                )
            }?)
        })();
        let memory = match result {
            Ok(m) => m,
            Err(e) => {
                unsafe {
                    ctx.handle.destroy_image(handle, None);
                }
                return Err(e);
            }
        };
        if let Err(e) = unsafe { ctx.handle.bind_image_memory(handle, memory, 0) } {
            unsafe {
                ctx.handle.destroy_image(handle, None);
                ctx.handle.free_memory(memory, None);
            }
            return Err(e.into());
        }
        let aspect = if depth {
            vk::ImageAspectFlags::DEPTH
        } else {
            vk::ImageAspectFlags::COLOR
        };
        let range = vk::ImageSubresourceRange {
            aspect_mask: aspect,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let view = match unsafe {
            ctx.handle.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(handle)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(range),
                None,
            )
        } {
            Ok(v) => v,
            Err(e) => {
                unsafe {
                    ctx.handle.destroy_image(handle, None);
                    ctx.handle.free_memory(memory, None);
                }
                return Err(e.into());
            }
        };
        Ok(Self {
            ctx,
            handle,
            memory,
            view,
            size,
            layout: vk::ImageLayout::UNDEFINED,
            aspect,
        })
    }
    pub fn upload(&mut self, origin: [usize; 2], size: [usize; 2], rgba: &[u8]) -> Result<()> {
        ensure!(
            origin[0] + size[0] <= self.size[0] as usize
                && origin[1] + size[1] <= self.size[1] as usize
                && rgba.len() == size[0] * size[1] * 4,
            "invalid Vulkan texture update"
        );
        let staging =
            Buffer::from_slice(self.ctx.clone(), rgba, vk::BufferUsageFlags::TRANSFER_SRC)?;
        let command = self.ctx.begin()?;
        transition(
            &self.ctx.handle,
            command,
            self.handle,
            self.aspect,
            self.layout,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: self.aspect,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D {
                x: origin[0] as i32,
                y: origin[1] as i32,
                z: 0,
            })
            .image_extent(vk::Extent3D {
                width: size[0] as u32,
                height: size[1] as u32,
                depth: 1,
            });
        unsafe {
            self.ctx.handle.cmd_copy_buffer_to_image(
                command,
                staging.handle,
                self.handle,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
        transition(
            &self.ctx.handle,
            command,
            self.handle,
            self.aspect,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        self.ctx.finish(command, &[], &[])?;
        self.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        Ok(())
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        unsafe {
            self.ctx.handle.destroy_image_view(self.view, None);
            self.ctx.handle.destroy_image(self.handle, None);
            self.ctx.handle.free_memory(self.memory, None);
        }
    }
}
pub(super) fn transition(
    device: &ash::Device,
    command: vk::CommandBuffer,
    image: vk::Image,
    aspect: vk::ImageAspectFlags,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
) {
    let range = vk::ImageSubresourceRange {
        aspect_mask: aspect,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let barrier = vk::ImageMemoryBarrier::default()
        .image(image)
        .old_layout(old)
        .new_layout(new)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .subresource_range(range)
        .src_access_mask(if old == vk::ImageLayout::UNDEFINED {
            vk::AccessFlags::empty()
        } else {
            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE
        })
        .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE);
    unsafe {
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}
pub(super) struct Framebuffer {
    ctx: Arc<Device>,
    pub handle: vk::Framebuffer,
}
impl Framebuffer {
    pub fn new(
        ctx: Arc<Device>,
        pass: vk::RenderPass,
        views: &[vk::ImageView],
        size: [u32; 2],
    ) -> Result<Self> {
        let handle = unsafe {
            ctx.handle.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(pass)
                    .attachments(views)
                    .width(size[0])
                    .height(size[1])
                    .layers(1),
                None,
            )
        }?;
        Ok(Self { ctx, handle })
    }
}
impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe {
            self.ctx.handle.destroy_framebuffer(self.handle, None);
        }
    }
}
pub(super) struct DescriptorPool {
    ctx: Arc<Device>,
    pub handle: vk::DescriptorPool,
}
impl DescriptorPool {
    pub fn new(ctx: Arc<Device>, sets: u32) -> Result<Self> {
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: sets * 2,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: sets * 2,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: sets * 2,
            },
        ];
        let handle = unsafe {
            ctx.handle.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(sets)
                    .pool_sizes(&sizes),
                None,
            )
        }?;
        Ok(Self { ctx, handle })
    }
    pub fn allocate(&self, layout: vk::DescriptorSetLayout) -> Result<vk::DescriptorSet> {
        Ok(unsafe {
            self.ctx.handle.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.handle)
                    .set_layouts(&[layout]),
            )
        }?[0])
    }
}
impl Drop for DescriptorPool {
    fn drop(&mut self) {
        unsafe {
            self.ctx.handle.destroy_descriptor_pool(self.handle, None);
        }
    }
}

pub(super) struct Sync {
    ctx: Arc<Device>,
    pub available: vk::Semaphore,
    pub finished: vk::Semaphore,
}
impl Sync {
    pub(super) fn new(ctx: Arc<Device>) -> Result<Self> {
        let available = unsafe {
            ctx.handle
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        }?;
        let finished = match unsafe {
            ctx.handle
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        } {
            Ok(s) => s,
            Err(e) => {
                unsafe {
                    ctx.handle.destroy_semaphore(available, None);
                }
                return Err(e.into());
            }
        };
        Ok(Self {
            ctx,
            available,
            finished,
        })
    }
}
impl Drop for Sync {
    fn drop(&mut self) {
        unsafe {
            // Also retire a failed/out-of-date presentation before destroying its wait semaphores.
            let _ = self.ctx.handle.queue_wait_idle(self.ctx.queue);
            self.ctx.handle.destroy_semaphore(self.available, None);
            self.ctx.handle.destroy_semaphore(self.finished, None);
        }
    }
}
pub(super) struct Timestamp {
    ctx: Arc<Device>,
    pool: vk::QueryPool,
}
impl Timestamp {
    pub(super) fn new(ctx: Arc<Device>) -> Result<Self> {
        let pool = if ctx.timestamp_bits > 0 {
            unsafe {
                ctx.handle.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(2),
                    None,
                )
            }?
        } else {
            vk::QueryPool::null()
        };
        Ok(Self { ctx, pool })
    }
    pub(super) fn start(&self, command: vk::CommandBuffer) {
        if self.pool != vk::QueryPool::null() {
            unsafe {
                self.ctx
                    .handle
                    .cmd_reset_query_pool(command, self.pool, 0, 2);
                self.ctx.handle.cmd_write_timestamp(
                    command,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    self.pool,
                    0,
                );
            }
        }
    }
    pub(super) fn end(&self, command: vk::CommandBuffer) {
        if self.pool != vk::QueryPool::null() {
            unsafe {
                self.ctx.handle.cmd_write_timestamp(
                    command,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    self.pool,
                    1,
                );
            }
        }
    }
    pub(super) fn milliseconds(&self) -> Result<f64> {
        if self.pool == vk::QueryPool::null() {
            return Ok(0.);
        }
        let mut values = [0_u64; 2];
        unsafe {
            self.ctx.handle.get_query_pool_results(
                self.pool,
                0,
                &mut values,
                vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
            )
        }?;
        let mask = if self.ctx.timestamp_bits == 64 {
            u64::MAX
        } else {
            (1_u64 << self.ctx.timestamp_bits) - 1
        };
        Ok(
            (values[1].wrapping_sub(values[0]) & mask) as f64 * self.ctx.timestamp_period as f64
                / 1e6,
        )
    }
}
impl Drop for Timestamp {
    fn drop(&mut self) {
        unsafe {
            self.ctx.handle.destroy_query_pool(self.pool, None);
        }
    }
}
