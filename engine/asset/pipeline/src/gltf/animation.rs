//! glTF-specific readers stop here; products and scene bindings use Comet identities.
use super::{ImportedProduct, named_key, product_label};
use anyhow::{Context, Result, ensure};
use comet_asset_data::{
    AnimationChannel, AnimationClipData, AnimationInterpolation, AnimationValues,
    MAX_ANIMATION_CHANNELS, MAX_ANIMATION_KEYS, MAX_SKIN_JOINTS, SkeletonData, SkeletonNode,
    SkinData, SkinJoint, product::Product,
};
use comet_core::{
    AssetId, EntityId,
    math::{Mat4, Quat},
};
use comet_scene::{
    Animator, ContentSource, EntityData, EntityRef, MAX_ANIMATOR_BINDINGS, SkinBinding,
};
use std::collections::{BTreeMap, BTreeSet};

type Attributes = (Option<Vec<[u16; 4]>>, Option<Vec<[f32; 4]>>);
pub(super) fn attributes(
    primitive: &gltf::Primitive<'_>,
    buffers: &[Vec<u8>],
) -> Result<Attributes> {
    for (semantic, _) in primitive.attributes() {
        ensure!(
            !matches!(semantic, gltf::Semantic::Joints(set) | gltf::Semantic::Weights(set) if set != 0),
            "additional joint/weight sets are unsupported"
        );
    }
    let reader = primitive.reader(|buffer| Some(buffers[buffer.index()].as_slice()));
    let joints = reader
        .read_joints(0)
        .map(|values| values.into_u16().collect::<Vec<_>>());
    let weights = reader
        .read_weights(0)
        .map(|values| values.into_f32().collect::<Vec<_>>());
    ensure!(
        joints.is_some() == weights.is_some(),
        "joint/weight attributes must be paired"
    );
    let weights = weights
        .map(|mut values| -> Result<_> {
            for weights in &mut values {
                ensure!(
                    weights
                        .iter()
                        .all(|value| value.is_finite() && *value >= 0.),
                    "invalid glTF skin weights"
                );
                let sum: f64 = weights.iter().map(|value| f64::from(*value)).sum();
                ensure!(
                    sum.is_finite() && sum > 0.,
                    "glTF skin weights have zero sum"
                );
                for weight in weights {
                    *weight = (f64::from(*weight) / sum) as f32;
                }
            }
            Ok(values)
        })
        .transpose()?;
    Ok((joints, weights))
}

pub(super) fn import(
    doc: &gltf::Document,
    buffers: &[Vec<u8>],
    id: AssetId,
    visited: &BTreeSet<usize>,
    source: &mut ContentSource,
    products: &mut Vec<ImportedProduct>,
) -> Result<()> {
    if doc.skins().next().is_none() && doc.animations().next().is_none() {
        return Ok(());
    }
    ensure!(
        doc.scenes().len() == 1,
        "animated glTF import requires one scene; export the selected scene explicitly"
    );
    ensure!(
        visited.len() <= MAX_ANIMATOR_BINDINGS,
        "animation source node budget exceeded"
    );
    let node_ids: BTreeMap<_, _> = doc
        .nodes()
        .filter(|node| visited.contains(&node.index()))
        .map(|node| {
            let key = named_key(
                "node",
                node.index(),
                node.name(),
                doc.nodes().filter(|n| n.name() == node.name()).count(),
            );
            (node.index(), EntityId::in_asset(id, &key))
        })
        .collect();
    let skeleton_id = id.child("skeleton:scene");
    let entities: BTreeMap<_, _> = source
        .base
        .entities
        .iter()
        .map(|entity| (entity.id, entity))
        .collect();
    let skeleton = SkeletonData {
        version: 1,
        nodes: node_ids
            .iter()
            .map(|(index, node)| {
                let entity = entities[node];
                let source_node = doc.nodes().nth(*index).expect("known imported node");
                SkeletonNode {
                    id: *node,
                    name: product_label(
                        "Node",
                        *index,
                        source_node.name(),
                        doc.nodes()
                            .filter(|n| n.name() == source_node.name())
                            .count(),
                    ),
                    parent: entity.parent,
                    rest: entity.transform.clone(),
                }
            })
            .collect(),
    };
    skeleton.validate()?;
    let parents: BTreeMap<_, _> = skeleton
        .nodes
        .iter()
        .map(|node| (node.id, node.parent))
        .collect();
    let mut skins = BTreeMap::new();
    let used: BTreeSet<_> = doc
        .nodes()
        .filter(|node| visited.contains(&node.index()))
        .filter_map(|node| node.skin().map(|skin| skin.index()))
        .collect();
    for skin in doc.skins().filter(|skin| used.contains(&skin.index())) {
        ensure!(
            skin.joints().len() <= MAX_SKIN_JOINTS,
            "glTF skin joint budget exceeded"
        );
        let reader = skin.reader(|buffer| Some(buffers[buffer.index()].as_slice()));
        let inverse = reader
            .read_inverse_bind_matrices()
            .map(|values| values.take(skin.joints().len()).collect::<Vec<_>>());
        ensure!(
            inverse
                .as_ref()
                .is_none_or(|values| values.len() >= skin.joints().len()),
            "truncated inverse bind matrices"
        );
        let joints = skin
            .joints()
            .enumerate()
            .map(|(index, node)| -> Result<_> {
                Ok(SkinJoint {
                    node: *node_ids
                        .get(&node.index())
                        .context("skin joint is outside the imported scene")?,
                    inverse_bind: inverse
                        .as_ref()
                        .map_or(Mat4::IDENTITY.to_cols_array_2d(), |values| values[index]),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let data = SkinData {
            version: 1,
            skeleton: skeleton_id,
            joints,
        };
        data.validate_for(&skeleton)?;
        let mut common: Option<BTreeSet<_>> = None;
        for joint in &data.joints {
            let mut ancestors = BTreeSet::new();
            let mut next = Some(joint.node);
            while let Some(node) = next {
                ancestors.insert(node);
                next = parents[&node];
            }
            common = Some(common.map_or_else(
                || ancestors.clone(),
                |old| old.intersection(&ancestors).copied().collect(),
            ));
        }
        let common = common.context("empty skin")?;
        ensure!(
            !common.is_empty(),
            "skin joints have no common source ancestor"
        );
        if let Some(root) = skin.skeleton() {
            let root = node_ids
                .get(&root.index())
                .context("skin skeleton root is outside the imported scene")?;
            ensure!(
                common.contains(root),
                "skin skeleton node is not an ancestor of all joints"
            );
        }
        let skin_id = id.child(&named_key(
            "skin",
            skin.index(),
            skin.name(),
            doc.skins().filter(|s| s.name() == skin.name()).count(),
        ));
        skins.insert(skin.index(), skin_id);
        products.push(ImportedProduct {
            id: skin_id,
            payload: Product::Skin(data),
            label: product_label(
                "Skin",
                skin.index(),
                skin.name(),
                doc.skins().filter(|s| s.name() == skin.name()).count(),
            ),
        });
    }
    let mut first_clip = None;
    for animation in doc.animations() {
        ensure!(
            animation.channels().count() <= MAX_ANIMATION_CHANNELS,
            "animation channel budget exceeded"
        );
        let mut total = 0usize;
        let mut channels = Vec::new();
        let mut duration = 0f32;
        for channel in animation.channels() {
            let sampler = channel.sampler();
            total = total
                .checked_add(sampler.input().count())
                .context("animation key budget overflow")?;
            ensure!(total <= MAX_ANIMATION_KEYS, "animation key budget exceeded");
            ensure!(
                sampler.output().count() <= MAX_ANIMATION_KEYS * 3,
                "animation output key budget exceeded"
            );
            let interpolation = match sampler.interpolation() {
                gltf::animation::Interpolation::Step => AnimationInterpolation::Step,
                gltf::animation::Interpolation::Linear => AnimationInterpolation::Linear,
                gltf::animation::Interpolation::CubicSpline => AnimationInterpolation::CubicSpline,
            };
            ensure!(
                sampler.output().count()
                    == sampler.input().count() * interpolation.values_per_key(),
                "animation input/output count mismatch"
            );
            let target = channel.target().node();
            ensure!(
                matches!(
                    target.transform(),
                    gltf::scene::Transform::Decomposed { .. }
                ),
                "animated node must use TRS, not matrix"
            );
            let node = *node_ids.get(&target.index()).context("animation target is outside the imported scene; export the selected scene explicitly")?;
            let reader = channel.reader(|buffer| Some(buffers[buffer.index()].as_slice()));
            let times: Vec<_> = reader
                .read_inputs()
                .context("missing animation input")?
                .collect();
            let values = match reader.read_outputs().context("missing animation output")? {
                gltf::animation::util::ReadOutputs::Translations(values) => {
                    AnimationValues::Translation(values.collect())
                }
                gltf::animation::util::ReadOutputs::Scales(values) => {
                    AnimationValues::Scale(values.collect())
                }
                gltf::animation::util::ReadOutputs::Rotations(values) => {
                    let values = values
                        .into_f32()
                        .enumerate()
                        .map(|(index, value)| -> Result<_> {
                            if interpolation == AnimationInterpolation::CubicSpline
                                && index % 3 != 1
                            {
                                ensure!(
                                    value.iter().all(|v| v.is_finite()),
                                    "non-finite glTF animation rotation tangent"
                                );
                                return Ok(value);
                            }
                            let rotation = Quat::from_array(value);
                            ensure!(
                                rotation.is_finite()
                                    && (rotation.length_squared() - 1.).abs() <= 1e-3,
                                "invalid glTF animation quaternion"
                            );
                            Ok(rotation.normalize().to_array())
                        })
                        .collect::<Result<Vec<_>>>()?;
                    AnimationValues::Rotation(values)
                }
                gltf::animation::util::ReadOutputs::MorphTargetWeights(_) => {
                    anyhow::bail!("morph animation is unsupported")
                }
            };
            if let Some(last) = times.last() {
                duration = duration.max(*last);
            }
            channels.push(AnimationChannel {
                node,
                interpolation,
                times,
                values,
            });
        }
        let events = animation
            .extras()
            .as_ref()
            .map(|extras| -> Result<_> {
                let value: serde_json::Value = serde_json::from_str(extras.get())?;
                value.get("comet_events").map_or_else(
                    || Ok(Vec::new()),
                    |events| {
                        serde_json::from_value::<Vec<comet_asset_data::AnimationEvent>>(
                            events.clone(),
                        )
                        .context("invalid glTF animation extras.comet_events")
                    },
                )
            })
            .transpose()?
            .unwrap_or_default();
        let data = AnimationClipData {
            version: if !events.is_empty() {
                comet_asset_data::ANIMATION_CLIP_VERSION
            } else if channels
                .iter()
                .any(|channel| channel.interpolation == AnimationInterpolation::CubicSpline)
            {
                2
            } else {
                1
            },
            skeleton: skeleton_id,
            duration,
            channels,
            events,
        };
        data.validate_for(&skeleton)?;
        let clip = id.child(&named_key(
            "animation",
            animation.index(),
            animation.name(),
            doc.animations()
                .filter(|a| a.name() == animation.name())
                .count(),
        ));
        first_clip.get_or_insert(clip);
        products.push(ImportedProduct {
            id: clip,
            payload: Product::AnimationClip(data),
            label: product_label(
                "Animation",
                animation.index(),
                animation.name(),
                doc.animations()
                    .filter(|a| a.name() == animation.name())
                    .count(),
            ),
        });
    }
    let controller = EntityId::in_asset(id, "animation:controller");
    for node in doc.nodes().filter(|node| visited.contains(&node.index())) {
        if let Some(mesh) = node.mesh() {
            let key = named_key(
                "node",
                node.index(),
                node.name(),
                doc.nodes().filter(|n| n.name() == node.name()).count(),
            );
            for primitive in mesh.primitives() {
                let entity = source
                    .base
                    .entities
                    .iter_mut()
                    .find(|entity| {
                        entity.id
                            == EntityId::in_asset(
                                id,
                                &format!("{key}/primitive:{}", primitive.index()),
                            )
                    })
                    .context("imported primitive entity")?;
                if let Some(skin) = node.skin() {
                    let skin_id = skins[&skin.index()];
                    let data = products
                        .iter()
                        .find_map(|p| (p.id == skin_id).then_some(&p.payload))
                        .context("imported skin")?;
                    let Product::Skin(data) = data else {
                        unreachable!()
                    };
                    let mesh_id = entity.model.as_ref().unwrap().mesh;
                    let mesh = products
                        .iter()
                        .find_map(|p| (p.id == mesh_id).then_some(&p.payload))
                        .context("imported mesh")?;
                    let Product::Mesh(mesh) = mesh else {
                        unreachable!()
                    };
                    data.validate_mesh(mesh)?;
                    entity.skin = Some(SkinBinding {
                        skin: skin_id,
                        animator: Some(EntityRef::local(controller)),
                    });
                }
            }
        }
    }
    let mut owner = EntityData::new("蒙皮模型控制器");
    owner.id = controller;
    owner.animator = Some(Animator {
        controller: None,
        skeleton: skeleton_id,
        clip: first_clip,
        bindings: node_ids
            .values()
            .map(|node| (*node, Some(EntityRef::local(*node))))
            .collect(),
        looping: true,
        speed: 1.,
    });
    for entity in &mut source.base.entities {
        if entity.parent.is_none() {
            entity.parent = Some(controller);
        }
    }
    source.base.entities.push(owner);
    source.base.version = 9;
    products.push(ImportedProduct {
        id: skeleton_id,
        payload: Product::Skeleton(skeleton),
        label: "Skeleton".into(),
    });
    Ok(())
}
