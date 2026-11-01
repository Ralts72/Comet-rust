use crate::Surface;
use anyhow::{Context, Result, ensure};
use ash::vk;
use std::{ffi::CString, sync::Arc};
struct Instance {
    _entry: ash::Entry,
    handle: ash::Instance,
    surface_api: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    _anchor: Option<Surface>,
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            if self.surface != vk::SurfaceKHR::null() {
                self.surface_api.destroy_surface(self.surface, None);
            }
            self.handle.destroy_instance(None);
        }
    }
}
pub(super) struct Device {
    instance: Arc<Instance>,
    pub handle: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub pool: vk::CommandPool,
    pub timestamp_period: f32,
    pub timestamp_bits: u32,
}
impl Device {
    pub fn new(surface: Option<Surface>) -> Result<Arc<Self>> {
        let entry = unsafe { ash::Entry::load() }.context("load native Vulkan loader")?;
        let name = CString::new("Comet")?;
        let app = vk::ApplicationInfo::default()
            .application_name(&name)
            .api_version(vk::API_VERSION_1_1);
        let extensions = if let Some(s) = &surface {
            ash_window::enumerate_required_extensions(s.window.display_handle()?.as_raw())?.to_vec()
        } else {
            vec![]
        };
        let handle = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app)
                    .enabled_extension_names(&extensions),
                None,
            )
        }?;
        let surface_api = ash::khr::surface::Instance::new(&entry, &handle);
        let mut instance = Instance {
            _entry: entry,
            handle,
            surface_api,
            surface: vk::SurfaceKHR::null(),
            _anchor: surface,
        };
        if let Some(s) = &instance._anchor {
            instance.surface = unsafe {
                ash_window::create_surface(
                    &instance._entry,
                    &instance.handle,
                    s.window.display_handle()?.as_raw(),
                    s.window.window_handle()?.as_raw(),
                    None,
                )
            }?;
        }
        let instance = Arc::new(instance);
        let mut chosen = None;
        for physical in unsafe { instance.handle.enumerate_physical_devices() }? {
            let properties = unsafe { instance.handle.get_physical_device_properties(physical) };
            if properties.api_version < vk::API_VERSION_1_1 {
                continue;
            }
            let families = unsafe {
                instance
                    .handle
                    .get_physical_device_queue_family_properties(physical)
            };
            for (index, family) in families.iter().enumerate() {
                if family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                    && (instance.surface == vk::SurfaceKHR::null()
                        || unsafe {
                            instance.surface_api.get_physical_device_surface_support(
                                physical,
                                index as u32,
                                instance.surface,
                            )
                        }?)
                {
                    chosen = Some((physical, index as u32));
                    break;
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        let (physical, family) = chosen.context("Vulkan 1.1 graphics/present queue unavailable")?;
        let format = unsafe {
            instance
                .handle
                .get_physical_device_format_properties(physical, vk::Format::D32_SFLOAT)
        };
        ensure!(
            format.optimal_tiling_features.contains(
                vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT
                    | vk::FormatFeatureFlags::SAMPLED_IMAGE
            ),
            "D32 shadow sampling unavailable"
        );
        let priorities = [1.];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let extensions = if instance.surface != vk::SurfaceKHR::null() {
            vec![ash::khr::swapchain::NAME.as_ptr()]
        } else {
            vec![]
        };
        let handle = unsafe {
            instance.handle.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&extensions),
                None,
            )
        }?;
        let queue = unsafe { handle.get_device_queue(family, 0) };
        let pool = match unsafe {
            handle.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        } {
            Ok(p) => p,
            Err(e) => {
                unsafe {
                    handle.destroy_device(None);
                }
                return Err(e.into());
            }
        };
        let timestamp_period = unsafe { instance.handle.get_physical_device_properties(physical) }
            .limits
            .timestamp_period;
        let timestamp_bits = unsafe {
            instance
                .handle
                .get_physical_device_queue_family_properties(physical)
        }[family as usize]
            .timestamp_valid_bits;
        Ok(Arc::new(Self {
            instance,
            handle,
            physical,
            queue,
            pool,
            timestamp_period,
            timestamp_bits,
        }))
    }
    pub fn instance(&self) -> &ash::Instance {
        &self.instance.handle
    }
    pub fn surface(&self) -> vk::SurfaceKHR {
        self.instance.surface
    }
    pub fn surface_api(&self) -> &ash::khr::surface::Instance {
        &self.instance.surface_api
    }
    pub fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        let properties = unsafe {
            self.instance
                .handle
                .get_physical_device_memory_properties(self.physical)
        };
        (0..properties.memory_type_count)
            .find(|i| {
                bits & (1 << i) != 0
                    && properties.memory_types[*i as usize]
                        .property_flags
                        .contains(flags)
            })
            .context("Vulkan memory type unavailable")
    }
    pub fn begin(&self) -> Result<vk::CommandBuffer> {
        let command = unsafe {
            self.handle.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }?[0];
        if let Err(error) = unsafe {
            self.handle.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        } {
            unsafe {
                self.handle.free_command_buffers(self.pool, &[command]);
            }
            return Err(error.into());
        }
        Ok(command)
    }
    pub fn finish(
        &self,
        command: vk::CommandBuffer,
        wait: &[vk::Semaphore],
        signal: &[vk::Semaphore],
    ) -> Result<()> {
        let result = (|| -> Result<()> {
            unsafe { self.handle.end_command_buffer(command) }?;
            let fence = unsafe {
                self.handle
                    .create_fence(&vk::FenceCreateInfo::default(), None)
            }?;
            let commands = [command];
            let stages = vec![vk::PipelineStageFlags::TRANSFER; wait.len()];
            let submit = [vk::SubmitInfo::default()
                .command_buffers(&commands)
                .wait_semaphores(wait)
                .wait_dst_stage_mask(&stages)
                .signal_semaphores(signal)];
            let result = unsafe {
                self.handle
                    .queue_submit(self.queue, &submit, fence)
                    .and_then(|_| self.handle.wait_for_fences(&[fence], true, u64::MAX))
            };
            unsafe {
                self.handle.destroy_fence(fence, None);
            }
            result?;
            Ok(())
        })();
        unsafe {
            self.handle.free_command_buffers(self.pool, &[command]);
        }
        result
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = self.handle.device_wait_idle();
            self.handle.destroy_command_pool(self.pool, None);
            self.handle.destroy_device(None);
        }
    }
}

pub(super) struct Swapchain {
    ctx: Arc<Device>,
    pub api: ash::khr::swapchain::Device,
    pub handle: vk::SwapchainKHR,
    pub images: Vec<vk::Image>,
    pub extent: vk::Extent2D,
}
impl Swapchain {
    pub fn new(ctx: Arc<Device>, size: [u32; 2]) -> Result<Self> {
        let caps = unsafe {
            ctx.surface_api()
                .get_physical_device_surface_capabilities(ctx.physical, ctx.surface())
        }?;
        ensure!(
            caps.supported_usage_flags
                .contains(vk::ImageUsageFlags::TRANSFER_DST),
            "swapchain transfer destination unavailable"
        );
        let formats = unsafe {
            ctx.surface_api()
                .get_physical_device_surface_formats(ctx.physical, ctx.surface())
        }?;
        let format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .or_else(|| {
                formats
                    .iter()
                    .find(|f| f.format == vk::Format::R8G8B8A8_UNORM)
            })
            .context("UNORM presentation format unavailable")?;
        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: size[0].clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: size[1].clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };
        let count = (caps.min_image_count + 1).min(if caps.max_image_count == 0 {
            u32::MAX
        } else {
            caps.max_image_count
        });
        let alpha = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::INHERIT,
        ]
        .into_iter()
        .find(|a| caps.supported_composite_alpha.contains(*a))
        .context("composite alpha mode")?;
        let api = ash::khr::swapchain::Device::new(ctx.instance(), &ctx.handle);
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(ctx.surface())
            .min_image_count(count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(alpha)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true);
        let handle = unsafe { api.create_swapchain(&info, None) }?;
        let images = match unsafe { api.get_swapchain_images(handle) } {
            Ok(i) => i,
            Err(e) => {
                unsafe {
                    api.destroy_swapchain(handle, None);
                }
                return Err(e.into());
            }
        };
        Ok(Self {
            ctx,
            api,
            handle,
            images,
            extent,
        })
    }
}
impl Drop for Swapchain {
    fn drop(&mut self) {
        unsafe {
            let _ = self.ctx.handle.queue_wait_idle(self.ctx.queue);
            self.api.destroy_swapchain(self.handle, None);
        }
    }
}
