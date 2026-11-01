//! Native Vulkan device and queue diagnostics.
use anyhow::{Context, Result};
use ash::vk;
use std::ffi::{CStr, CString};
pub fn run() -> Result<serde_json::Value> {
    // Entry dynamically loads the platform Vulkan loader; no C/C++ implementation is built.
    let entry = unsafe { ash::Entry::load() }?;
    let name = CString::new("Comet GPU diagnostics")?;
    let app = vk::ApplicationInfo::default()
        .application_name(&name)
        .api_version(vk::API_VERSION_1_2);
    let instance = unsafe {
        entry.create_instance(
            &vk::InstanceCreateInfo::default().application_info(&app),
            None,
        )
    }?;
    let result = (|| -> Result<serde_json::Value> {
        let devices = unsafe { instance.enumerate_physical_devices() }?;
        let physical = *devices.first().context("no Vulkan physical device")?;
        let properties = unsafe { instance.get_physical_device_properties(physical) };
        let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
        let graphics = families
            .iter()
            .position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .context("no graphics queue")?;
        let compute = families
            .iter()
            .position(|f| {
                f.queue_flags.contains(vk::QueueFlags::COMPUTE)
                    && !f.queue_flags.contains(vk::QueueFlags::GRAPHICS)
            })
            .unwrap_or(graphics);
        let priorities = [1.0, 1.0];
        let same = graphics == compute;
        let count = if same {
            families[graphics].queue_count.min(2) as usize
        } else {
            1
        };
        let mut queues = vec![
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(graphics as u32)
                .queue_priorities(&priorities[..count]),
        ];
        if !same {
            queues.push(
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(compute as u32)
                    .queue_priorities(&priorities[..1]),
            );
        }
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
                None,
            )
        }?;
        unsafe {
            let _graphics = device.get_device_queue(graphics as u32, 0);
            let _compute =
                device.get_device_queue(compute as u32, if same && count > 1 { 1 } else { 0 });
            let completion = device.device_wait_idle();
            device.destroy_device(None);
            completion?;
        }
        Ok(
            serde_json::json!({"status":"passed","device":unsafe{CStr::from_ptr(properties.device_name.as_ptr())}.to_string_lossy(),"graphics_family":graphics,"compute_family":compute,"queue_count":if same{count}else{2},"shader_format":"SPIR-V from Naga"}),
        )
    })();
    unsafe {
        instance.destroy_instance(None);
    }
    result
}
