//! Structural component commands only target authored entities; asset validation stays in the editor.
use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::{AssetId, EntityId};
use comet_editor_core::{ComponentEdit, ComponentKind};
use comet_scene::{
    Animator, AudioSource, BodyMotion, Character, CharacterAnimation, ColliderShape, EntityData,
    Model, NavigationAgent, NavigationRegion, PhysicsBody, Script, SkinBinding, Surface,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Default)]
pub(crate) struct ComponentResources {
    pub asset: Option<AssetId>,
    pub material: Option<AssetId>,
}
impl Editor {
    fn component_targets(&self, ids: &[EntityId]) -> Result<Vec<EntityData>> {
        self.ensure_authoring_editable()?;
        let source = self.document.shared_source();
        let mut seen = BTreeSet::new();
        ids.iter()
            .map(|id| {
                ensure!(seen.insert(*id), "组件目标重复：{id}");
                source
                    .base
                    .entities
                    .binary_search_by_key(id, |entity| entity.id)
                    .ok()
                    .map(|index| source.base.entities[index].clone())
                    .with_context(|| {
                        format!("组件结构只能在作者实体上修改，请在源 Prefab 中编辑：{id}")
                    })
            })
            .collect()
    }
    pub(crate) fn remove_components(
        &mut self,
        ids: &[EntityId],
        kind: ComponentKind,
        assets: &mut AssetStore,
    ) -> Result<()> {
        let edits: Vec<_> = self
            .component_targets(ids)?
            .iter()
            .filter(|entity| kind.is_present(entity))
            .map(|entity| (entity.id, kind.removal(entity)))
            .collect();
        if edits.is_empty() {
            return Ok(());
        }
        self.preview_document(assets, |document, assets| {
            document.preview_components(edits, assets)
        })
    }
    pub(crate) fn add_components(
        &mut self,
        ids: &[EntityId],
        kind: ComponentKind,
        resources: ComponentResources,
        assets: &mut AssetStore,
    ) -> Result<()> {
        let targets = self
            .component_targets(ids)?
            .into_iter()
            .filter(|entity| !kind.is_present(entity))
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return Ok(());
        }
        let mut edits = Vec::new();
        let mut snapshot = self.document.expand(assets)?;
        let mut transforms = None;
        for entity in &targets {
            let asset = || resources.asset.context("请先选择组件资源");
            let edit = (|| -> Result<ComponentEdit> {
                Ok(match kind {
                    ComponentKind::Model => {
                        let mesh = asset()?;
                        let material = resources.material.context("请先选择材质资源")?;
                        let product = assets.load(material)?;
                        let Product::Material(surface) = &product.payload else {
                            anyhow::bail!("请选择材质产品");
                        };
                        let model = Model {
                            mesh,
                            surface: Surface {
                                material,
                                roughness: surface.roughness,
                            },
                        };
                        crate::models::validate_products(&model, assets)?;
                        if transforms.is_none() {
                            transforms = Some(comet_scene::world_transforms(&snapshot)?);
                        }
                        comet_render::DrawTransform::prepare(
                            transforms.as_ref().unwrap()[&entity.id],
                        )?;
                        ComponentEdit::Model(Some(model))
                    }
                    ComponentKind::Physics => ComponentEdit::Physics {
                        body: Some(PhysicsBody::default()),
                        character: entity.character.clone(),
                    },
                    ComponentKind::Character => ComponentEdit::Physics {
                        body: Some(entity.physics.clone().unwrap_or(PhysicsBody {
                            motion: BodyMotion::Kinematic,
                            shape: ColliderShape::Capsule {
                                half_height: 0.5,
                                radius: 0.3,
                            },
                            ..Default::default()
                        })),
                        character: Some(Character {
                            player_controlled: ids.len() == 1
                                && !snapshot.entities.iter().any(|e| {
                                    e.character.as_ref().is_some_and(|c| c.player_controlled)
                                }),
                            ..Default::default()
                        }),
                    },
                    ComponentKind::Script => {
                        let script = Script::new(asset()?);
                        crate::scripts::validate_asset(script.asset, assets)?;
                        ComponentEdit::Script(Some(script))
                    }
                    ComponentKind::Audio => {
                        let audio = AudioSource::new(asset()?);
                        crate::audio::validate_source(&audio, assets)?;
                        ComponentEdit::Audio(Some(audio))
                    }
                    ComponentKind::NavigationRegion => ComponentEdit::Navigation {
                        region: Some(NavigationRegion::default()),
                        agent: entity.navigation_agent.clone(),
                    },
                    ComponentKind::NavigationAgent => ComponentEdit::Navigation {
                        region: entity.navigation_region.clone(),
                        agent: Some(NavigationAgent::default()),
                    },
                    ComponentKind::Animation | ComponentKind::Skin => {
                        let index = snapshot
                            .entities
                            .binary_search_by_key(&entity.id, |e| e.id)
                            .map_err(|_| anyhow::anyhow!("组件目标已失效"))?;
                        let mut candidate = snapshot.entities[index].clone();
                        if kind == ComponentKind::Animation {
                            candidate.animator = Some(Animator::new(asset()?));
                        } else {
                            candidate.skin = Some(SkinBinding {
                                skin: asset()?,
                                animator: None,
                            });
                        }
                        snapshot.entities[index] = candidate.clone();
                        // Validate expanded references, but retain the owner's lexical declarations.
                        ComponentEdit::Animation {
                            animator: if kind == ComponentKind::Animation {
                                candidate.animator
                            } else {
                                entity.animator.clone()
                            },
                            skin: if kind == ComponentKind::Skin {
                                candidate.skin
                            } else {
                                entity.skin.clone()
                            },
                        }
                    }
                    ComponentKind::CharacterAnimation => {
                        ComponentEdit::CharacterAnimation(Some(CharacterAnimation {
                            character: None,
                            animator: None,
                        }))
                    }
                    ComponentKind::PointJoint
                    | ComponentKind::HingeJoint
                    | ComponentKind::SliderJoint => {
                        let joint = match kind {
                            ComponentKind::PointJoint => comet_runtime::JointKind::Point,
                            ComponentKind::HingeJoint => comet_runtime::JointKind::Hinge,
                            _ => comet_runtime::JointKind::Slider,
                        };
                        ComponentEdit::Joint(crate::joints::initial_joint(joint, entity))
                    }
                })
            })()
            .with_context(|| format!("无法为实体 {} ({}) 添加组件", entity.name, entity.id))?;
            edits.push((entity.id, edit));
        }
        if edits.is_empty() {
            return Ok(());
        }
        if matches!(kind, ComponentKind::Animation | ComponentKind::Skin) {
            crate::animation::validate_rig_changes(
                &snapshot,
                &ids.iter().copied().collect(),
                assets,
            )?;
        }
        self.preview_document(assets, |document, assets| {
            document.preview_components(edits, assets)
        })
    }
}
