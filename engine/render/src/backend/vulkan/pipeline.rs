use super::device::Device;
use super::resources::{Buffer, DescriptorPool, Image};
use anyhow::Result;
use ash::vk;
use std::{ffi::CString, sync::Arc};
pub(super) struct Pipelines {
    ctx: Arc<Device>,
    pub scene_set: vk::DescriptorSetLayout,
    pub ui_set: vk::DescriptorSetLayout,
    pub scene_layout: vk::PipelineLayout,
    pub ui_layout: vk::PipelineLayout,
    pub main_pass: vk::RenderPass,
    pub shadow_pass: vk::RenderPass,
    pub ui_pass: vk::RenderPass,
    pub scene: vk::Pipeline,
    pub shadow: vk::Pipeline,
    pub skinned_scene: vk::Pipeline,
    pub skinned_shadow: vk::Pipeline,
    pub ui: vk::Pipeline,
    pub sampler: vk::Sampler,
    pub shadow_sampler: vk::Sampler,
    pub ui_sampler: vk::Sampler,
}
impl Pipelines {
    pub(super) fn scene_set(
        &self,
        pool: &DescriptorPool,
        global: &Buffer,
        object: &Buffer,
        image: &Image,
        shadow: &Image,
        palette: Option<&Buffer>,
    ) -> Result<vk::DescriptorSet> {
        let set = pool.allocate(self.scene_set)?;
        let buffers = [
            vk::DescriptorBufferInfo {
                buffer: global.handle,
                offset: 0,
                range: global.size as u64,
            },
            vk::DescriptorBufferInfo {
                buffer: object.handle,
                offset: 0,
                range: object.size as u64,
            },
        ];
        let images = [
            vk::DescriptorImageInfo::default()
                .image_view(image.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            vk::DescriptorImageInfo::default().sampler(self.sampler),
            vk::DescriptorImageInfo::default()
                .image_view(shadow.view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL),
            vk::DescriptorImageInfo::default().sampler(self.shadow_sampler),
        ];
        let mut writes = vec![];
        for i in 0..2 {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(i as u32)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&buffers[i..i + 1]),
            );
        }
        for i in 0_usize..4 {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(i as u32 + 2)
                    .descriptor_type(if i.is_multiple_of(2) {
                        vk::DescriptorType::SAMPLED_IMAGE
                    } else {
                        vk::DescriptorType::SAMPLER
                    })
                    .image_info(&images[i..i + 1]),
            );
        }
        let palette_info = palette.map(|buffer| {
            [vk::DescriptorBufferInfo {
                buffer: buffer.handle,
                offset: 0,
                range: buffer.size as u64,
            }]
        });
        if let Some(info) = &palette_info {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(6)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(info),
            );
        }
        unsafe {
            self.ctx.handle.update_descriptor_sets(&writes, &[]);
        }
        Ok(set)
    }

    pub fn new(ctx: Arc<Device>) -> Result<Self> {
        let mut p = Self {
            ctx,
            scene_set: vk::DescriptorSetLayout::null(),
            ui_set: vk::DescriptorSetLayout::null(),
            scene_layout: vk::PipelineLayout::null(),
            ui_layout: vk::PipelineLayout::null(),
            main_pass: vk::RenderPass::null(),
            shadow_pass: vk::RenderPass::null(),
            ui_pass: vk::RenderPass::null(),
            scene: vk::Pipeline::null(),
            shadow: vk::Pipeline::null(),
            skinned_scene: vk::Pipeline::null(),
            skinned_shadow: vk::Pipeline::null(),
            ui: vk::Pipeline::null(),
            sampler: vk::Sampler::null(),
            shadow_sampler: vk::Sampler::null(),
            ui_sampler: vk::Sampler::null(),
        };
        let scene_types = [
            vk::DescriptorType::UNIFORM_BUFFER,
            vk::DescriptorType::UNIFORM_BUFFER,
            vk::DescriptorType::SAMPLED_IMAGE,
            vk::DescriptorType::SAMPLER,
            vk::DescriptorType::SAMPLED_IMAGE,
            vk::DescriptorType::SAMPLER,
        ];
        let scene_types = [&scene_types[..], &[vk::DescriptorType::UNIFORM_BUFFER]].concat();
        p.scene_set = p.set_layout(&scene_types)?;
        p.ui_set = p.set_layout(&[
            vk::DescriptorType::UNIFORM_BUFFER,
            vk::DescriptorType::SAMPLED_IMAGE,
            vk::DescriptorType::SAMPLER,
        ])?;
        p.scene_layout = unsafe {
            p.ctx.handle.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&[p.scene_set]),
                None,
            )
        }?;
        p.ui_layout = unsafe {
            p.ctx.handle.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&[p.ui_set]),
                None,
            )
        }?;
        p.main_pass = p.render_pass(false, false)?;
        p.shadow_pass = p.render_pass(true, false)?;
        p.ui_pass = p.render_pass(false, true)?;
        let scene = crate::shader::compile(false)?;
        let ui = crate::shader::compile(true)?;
        p.scene = p.pipeline(
            &scene,
            "scene_vs",
            Some("scene_fs"),
            p.main_pass,
            p.scene_layout,
            false,
        )?;
        p.shadow = p.pipeline(
            &scene,
            "shadow_vs",
            None,
            p.shadow_pass,
            p.scene_layout,
            false,
        )?;
        p.skinned_scene = p.pipeline(
            &scene,
            "skinned_scene_vs",
            Some("scene_fs"),
            p.main_pass,
            p.scene_layout,
            false,
        )?;
        p.skinned_shadow = p.pipeline(
            &scene,
            "skinned_shadow_vs",
            None,
            p.shadow_pass,
            p.scene_layout,
            false,
        )?;
        p.ui = p.pipeline(&ui, "ui_vs", Some("ui_fs"), p.ui_pass, p.ui_layout, true)?;
        let sampler = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .max_lod(0.);
        p.sampler = unsafe { p.ctx.handle.create_sampler(&sampler, None) }?;
        p.ui_sampler = unsafe {
            p.ctx.handle.create_sampler(
                &sampler
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
        }?;
        p.shadow_sampler = unsafe {
            p.ctx.handle.create_sampler(
                &sampler
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .compare_enable(true)
                    .compare_op(vk::CompareOp::LESS_OR_EQUAL),
                None,
            )
        }?;
        Ok(p)
    }
    fn set_layout(&self, types: &[vk::DescriptorType]) -> Result<vk::DescriptorSetLayout> {
        let bindings: Vec<_> = types
            .iter()
            .enumerate()
            .map(|(i, t)| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(i as u32)
                    .descriptor_type(*t)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            })
            .collect();
        Ok(unsafe {
            self.ctx.handle.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
        }?)
    }
    fn render_pass(&self, shadow: bool, ui: bool) -> Result<vk::RenderPass> {
        let color = vk::AttachmentDescription::default()
            .format(vk::Format::B8G8R8A8_UNORM)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(if ui {
                vk::AttachmentLoadOp::LOAD
            } else {
                vk::AttachmentLoadOp::CLEAR
            })
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(if ui {
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
            } else {
                vk::ImageLayout::UNDEFINED
            })
            .final_layout(if ui {
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL
            } else {
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
            });
        let depth = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(if shadow {
                vk::AttachmentStoreOp::STORE
            } else {
                vk::AttachmentStoreOp::DONT_CARE
            })
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(if shadow {
                vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL
            } else {
                vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
            });
        let attachments = if shadow {
            vec![depth]
        } else if ui {
            vec![color]
        } else {
            vec![color, depth]
        };
        let colors = [vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        }];
        let depth_ref = vk::AttachmentReference {
            attachment: if shadow { 0 } else { 1 },
            layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        };
        let mut subpass =
            vk::SubpassDescription::default().pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS);
        if !shadow {
            subpass = subpass.color_attachments(&colors);
        }
        if !ui {
            subpass = subpass.depth_stencil_attachment(&depth_ref);
        }
        let mut dependencies = vec![
            vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::ALL_COMMANDS)
                .dst_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                )
                .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                ),
        ];
        dependencies.push(
            vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(if shadow {
                    vk::PipelineStageFlags::LATE_FRAGMENT_TESTS
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                } else {
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                })
                .dst_stage_mask(if shadow {
                    vk::PipelineStageFlags::FRAGMENT_SHADER
                } else if ui {
                    vk::PipelineStageFlags::TRANSFER
                } else {
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                })
                .src_access_mask(if shadow {
                    vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE
                } else {
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                })
                .dst_access_mask(if shadow {
                    vk::AccessFlags::SHADER_READ
                } else if ui {
                    vk::AccessFlags::TRANSFER_READ
                } else {
                    vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                }),
        );
        Ok(unsafe {
            self.ctx.handle.create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&[subpass])
                    .dependencies(&dependencies),
                None,
            )
        }?)
    }
    fn pipeline(
        &self,
        compiled: &crate::shader::Compiled,
        vertex: &str,
        fragment: Option<&str>,
        pass: vk::RenderPass,
        layout: vk::PipelineLayout,
        ui: bool,
    ) -> Result<vk::Pipeline> {
        let mut modules = vec![];
        let result = (|| -> Result<vk::Pipeline> {
            let mut names = vec![CString::new(vertex)?];
            if let Some(f) = fragment {
                names.push(CString::new(f)?);
            }
            let mut stages = vec![];
            for (i, name) in names.iter().enumerate() {
                let words = &compiled.spirv[name.to_str()?];
                let module = unsafe {
                    self.ctx.handle.create_shader_module(
                        &vk::ShaderModuleCreateInfo::default().code(words),
                        None,
                    )
                }?;
                modules.push(module);
                stages.push(
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(if i == 0 {
                            vk::ShaderStageFlags::VERTEX
                        } else {
                            vk::ShaderStageFlags::FRAGMENT
                        })
                        .module(module)
                        .name(name),
                );
            }
            let mut binding = vec![vk::VertexInputBindingDescription {
                binding: 0,
                stride: 32,
                input_rate: vk::VertexInputRate::VERTEX,
            }];
            let mut attrs: Vec<_> = (if ui {
                vec![
                    (0, vk::Format::R32G32_SFLOAT),
                    (8, vk::Format::R32G32_SFLOAT),
                    (16, vk::Format::R32G32B32A32_SFLOAT),
                ]
            } else {
                vec![
                    (0, vk::Format::R32G32B32_SFLOAT),
                    (12, vk::Format::R32G32B32_SFLOAT),
                    (24, vk::Format::R32G32_SFLOAT),
                ]
            })
            .into_iter()
            .enumerate()
            .map(
                |(i, (offset, format))| vk::VertexInputAttributeDescription {
                    location: i as u32,
                    binding: 0,
                    format,
                    offset,
                },
            )
            .collect();
            if vertex.starts_with("skinned_") {
                binding.push(vk::VertexInputBindingDescription {
                    binding: 1,
                    stride: 32,
                    input_rate: vk::VertexInputRate::VERTEX,
                });
                attrs.extend([
                    vk::VertexInputAttributeDescription {
                        location: 3,
                        binding: 1,
                        format: vk::Format::R32G32B32A32_UINT,
                        offset: 0,
                    },
                    vk::VertexInputAttributeDescription {
                        location: 4,
                        binding: 1,
                        format: vk::Format::R32G32B32A32_SFLOAT,
                        offset: 16,
                    },
                ]);
            }
            let vertex = vk::PipelineVertexInputStateCreateInfo::default()
                .vertex_binding_descriptions(&binding)
                .vertex_attribute_descriptions(&attrs);
            let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
            let viewport = vk::PipelineViewportStateCreateInfo::default()
                .viewport_count(1)
                .scissor_count(1);
            let raster = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::NONE)
                .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
                .line_width(1.);
            let samples = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1);
            let depth = vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(!ui)
                .depth_write_enable(!ui)
                .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
            let color = vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(ui)
                .src_color_blend_factor(vk::BlendFactor::ONE)
                .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(vk::BlendOp::ADD)
                .src_alpha_blend_factor(vk::BlendFactor::ONE)
                .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .alpha_blend_op(vk::BlendOp::ADD);
            let colors = if fragment.is_some() {
                vec![color]
            } else {
                vec![]
            };
            let blending = vk::PipelineColorBlendStateCreateInfo::default().attachments(&colors);
            let dynamics = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
            let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamics);
            let info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex)
                .input_assembly_state(&assembly)
                .viewport_state(&viewport)
                .rasterization_state(&raster)
                .multisample_state(&samples)
                .depth_stencil_state(&depth)
                .color_blend_state(&blending)
                .dynamic_state(&dynamic)
                .layout(layout)
                .render_pass(pass)
                .subpass(0);
            match unsafe {
                self.ctx
                    .handle
                    .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
            } {
                Ok(p) => Ok(p[0]),
                Err((created, e)) => {
                    for p in created {
                        unsafe {
                            self.ctx.handle.destroy_pipeline(p, None);
                        }
                    }
                    Err(e.into())
                }
            }
        })();
        for module in modules {
            unsafe {
                self.ctx.handle.destroy_shader_module(module, None);
            }
        }
        result
    }
}
impl Drop for Pipelines {
    fn drop(&mut self) {
        unsafe {
            for p in [
                self.scene,
                self.shadow,
                self.skinned_scene,
                self.skinned_shadow,
                self.ui,
            ] {
                self.ctx.handle.destroy_pipeline(p, None);
            }
            for l in [self.scene_layout, self.ui_layout] {
                self.ctx.handle.destroy_pipeline_layout(l, None);
            }
            for l in [self.scene_set, self.ui_set] {
                self.ctx.handle.destroy_descriptor_set_layout(l, None);
            }
            for p in [self.main_pass, self.shadow_pass, self.ui_pass] {
                self.ctx.handle.destroy_render_pass(p, None);
            }
            for s in [self.sampler, self.shadow_sampler, self.ui_sampler] {
                self.ctx.handle.destroy_sampler(s, None);
            }
        }
    }
}
