use anyhow::{Result, ensure};
use std::collections::BTreeMap;
#[repr(C)]
pub(crate) struct Globals {
    pub view_projection: [[f32; 4]; 4],
    pub light_projection: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub light: [f32; 4],
}
#[repr(C)]
pub(crate) struct Object {
    pub model: [[f32; 4]; 4],
    pub normal: [[f32; 4]; 4],
    pub color: [f32; 4],
    pub parameters: [f32; 4],
}
impl Globals {
    pub fn for_frame(frame: &crate::FramePacket) -> Self {
        Self {
            view_projection: frame
                .camera
                .view_projection(frame.viewport[2] as f32 / frame.viewport[3].max(1) as f32)
                .to_cols_array_2d(),
            light_projection: crate::Camera::light_projection().to_cols_array_2d(),
            eye: frame.camera.eye.extend(1.).to_array(),
            light: glam::Vec3::new(6., 10., 5.)
                .normalize()
                .extend(0.)
                .to_array(),
        }
    }
}
impl Object {
    pub fn for_draw(draw: &crate::Draw, shadows: bool) -> Result<Self> {
        let comet_asset_data::product::Product::Material(material) = &draw.material.payload else {
            anyhow::bail!("draw uses a non-material product")
        };
        Ok(Self {
            model: draw.transform.to_cols_array_2d(),
            normal: draw.transform.inverse().transpose().to_cols_array_2d(),
            color: material.base_color,
            parameters: [
                draw.roughness,
                material.metallic,
                if draw.selected { 1. } else { 0. },
                if shadows { 1. } else { 0. },
            ],
        })
    }
}
pub(crate) fn vertices(mesh: &comet_asset_data::product::MeshData) -> Vec<[f32; 8]> {
    mesh.positions
        .iter()
        .zip(&mesh.normals)
        .zip(&mesh.uvs)
        .map(|((p, n), uv)| [p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1]])
        .collect()
}
pub(crate) struct Compiled {
    pub msl: String,
    pub names: BTreeMap<String, String>,
    pub spirv: BTreeMap<String, Vec<u32>>,
}
pub(crate) fn compile(ui: bool) -> Result<Compiled> {
    let source = if ui {
        include_str!("../shaders/ui.wgsl")
    } else {
        include_str!("../shaders/scene.wgsl")
    };
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| anyhow::anyhow!("{}", e.emit_to_string(source)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)?;
    for (_, ty) in module.types.iter() {
        if let naga::TypeInner::Struct { span, .. } = ty.inner {
            if ty.name.as_deref() == Some("Globals") {
                ensure!(
                    span as usize == std::mem::size_of::<Globals>(),
                    "Globals CPU/GPU layout mismatch"
                );
            }
            if ty.name.as_deref() == Some("Object") {
                ensure!(
                    span as usize == std::mem::size_of::<Object>(),
                    "Object CPU/GPU layout mismatch"
                );
            }
        }
    }
    let mut options = naga::back::msl::Options {
        lang_version: (2, 4),
        fake_missing_bindings: false,
        ..Default::default()
    };
    for ep in &module.entry_points {
        let mut resources = BTreeMap::new();
        for (_, g) in module.global_variables.iter() {
            if let Some(binding) = g.binding {
                let target = if ui {
                    match binding.binding {
                        0 => naga::back::msl::BindTarget {
                            buffer: Some(1),
                            ..Default::default()
                        },
                        1 => naga::back::msl::BindTarget {
                            texture: Some(0),
                            ..Default::default()
                        },
                        _ => naga::back::msl::BindTarget {
                            sampler: Some(naga::back::msl::BindSamplerTarget::Resource(0)),
                            ..Default::default()
                        },
                    }
                } else {
                    match binding.binding {
                        0 | 1 => naga::back::msl::BindTarget {
                            buffer: Some(binding.binding as u8 + 1),
                            ..Default::default()
                        },
                        2 | 4 => naga::back::msl::BindTarget {
                            texture: Some(if binding.binding == 2 { 0 } else { 1 }),
                            ..Default::default()
                        },
                        _ => naga::back::msl::BindTarget {
                            sampler: Some(naga::back::msl::BindSamplerTarget::Resource(
                                if binding.binding == 3 { 0 } else { 1 },
                            )),
                            ..Default::default()
                        },
                    }
                };
                resources.insert(binding, target);
            }
        }
        options.per_entry_point_map.insert(
            ep.name.clone(),
            naga::back::msl::EntryPointResources {
                resources,
                sizes_buffer: None,
                immediates_buffer: None,
            },
        );
    }
    let (msl, translation) = naga::back::msl::write_string(
        &module,
        &info,
        &options,
        &naga::back::msl::PipelineOptions {
            vertex_pulling_transform: false,
            ..Default::default()
        },
    )?;
    let mut names = BTreeMap::new();
    let mut spirv = BTreeMap::new();
    for (ep, name) in module
        .entry_points
        .iter()
        .zip(translation.entry_point_names)
    {
        names.insert(
            ep.name.clone(),
            name.map_err(|e| anyhow::anyhow!("MSL entry: {e:?}"))?,
        );
        let mut spv_options = naga::back::spv::Options::default();
        spv_options
            .flags
            .remove(naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE);
        let words = naga::back::spv::write_vec(
            &module,
            &info,
            &spv_options,
            Some(&naga::back::spv::PipelineOptions {
                shader_stage: ep.stage,
                entry_point: ep.name.clone(),
            }),
        )?;
        spirv.insert(ep.name.clone(), words);
    }
    let result = Compiled { msl, names, spirv };
    ensure!(
        !result.msl.is_empty()
            && !result.names.is_empty()
            && result
                .spirv
                .values()
                .all(|s| s.first() == Some(&0x07230203)),
        "invalid backend shader output"
    );
    Ok(result)
}
#[cfg(test)]
mod tests {
    #[test]
    fn shaders_and_layouts_compile_for_both_backends() {
        for ui in [false, true] {
            let compiled = super::compile(ui).unwrap();
            assert!(!compiled.msl.is_empty());
            assert!(
                compiled
                    .spirv
                    .values()
                    .all(|s| s.first() == Some(&0x07230203))
            );
        }
    }
}
