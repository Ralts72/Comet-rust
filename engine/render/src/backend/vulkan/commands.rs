use ash::vk;
pub(super) fn begin_pass(
    device: &ash::Device,
    command: vk::CommandBuffer,
    pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    size: [u32; 2],
    clears: &[vk::ClearValue],
) {
    unsafe {
        device.cmd_begin_render_pass(
            command,
            &vk::RenderPassBeginInfo::default()
                .render_pass(pass)
                .framebuffer(framebuffer)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent: vk::Extent2D {
                        width: size[0],
                        height: size[1],
                    },
                })
                .clear_values(clears),
            vk::SubpassContents::INLINE,
        );
    }
}
pub(super) fn viewport(device: &ash::Device, command: vk::CommandBuffer, rect: [u32; 4]) {
    let [x, y, w, h] = rect;
    unsafe {
        device.cmd_set_viewport(
            command,
            0,
            &[vk::Viewport {
                x: x as f32,
                y: (y + h) as f32,
                width: w as f32,
                height: -(h as f32),
                min_depth: 0.,
                max_depth: 1.,
            }],
        );
        device.cmd_set_scissor(
            command,
            0,
            &[vk::Rect2D {
                offset: vk::Offset2D {
                    x: x as i32,
                    y: y as i32,
                },
                extent: vk::Extent2D {
                    width: w,
                    height: h,
                },
            }],
        );
    }
}
