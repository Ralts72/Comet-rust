//! Landmass owns paths and steering; Rapier remains the authority for actual character motion.
use crate::{FIXED_DELTA, physics::Physics};
use anyhow::{Context, Result, ensure};
use comet_asset_data::NavigationData;
use comet_core::{EntityId, math::Vec3};
use comet_scene::{EntityRef, SceneSnapshot};
use landmass::{
    Agent, AgentId, AgentState, Archipelago, ArchipelagoOptions, Character, CharacterId,
    CoordinateSystem, FromAgentRadius, Island, NavigationMesh, PointSampleDistance3d,
    TargetReachedCondition,
};
use std::{collections::BTreeMap, sync::Arc};

/// Result of the most recent steering update, independent of the navigation SDK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationStatus {
    Idle,
    Moving,
    Arrived,
    Unreachable,
    OffMesh,
}
impl NavigationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Moving => "moving",
            Self::Arrived => "arrived",
            Self::Unreachable => "unreachable",
            Self::OffMesh => "off_mesh",
        }
    }
}

pub(crate) type NavigationStatuses = BTreeMap<EntityId, NavigationStatus>;

#[derive(Clone, Copy, Debug)]
pub(crate) enum NavigationCommand {
    NavigateTo(EntityId, [f32; 3]),
    Follow(EntityId, EntityId),
    Stop(EntityId),
}

#[derive(Clone, Copy)]
enum Goal {
    Point([f32; 3]),
    Entity(EntityId),
    Stopped,
}

// SDK coordinates are isolated here; persisted and public values remain engine arrays.
type ForeignPoint = <landmass::XYZ as CoordinateSystem>::Coordinate;
struct Coordinates;
impl CoordinateSystem for Coordinates {
    type Coordinate = [f32; 3];
    type SampleDistance = PointSampleDistance3d;
    const FLIP_POLYGONS: bool = true;
    fn to_landmass(point: &[f32; 3]) -> ForeignPoint {
        ForeignPoint::new(point[0], -point[2], point[1])
    }
    fn from_landmass(point: &ForeignPoint) -> [f32; 3] {
        [point.x, point.z, -point.y]
    }
}

pub(crate) struct Navigation {
    archipelago: Archipelago<Coordinates>,
    agents: BTreeMap<EntityId, AgentId>,
    characters: BTreeMap<EntityId, CharacterId>,
    velocities: BTreeMap<EntityId, [f32; 3]>,
    goals: BTreeMap<EntityId, Goal>,
    statuses: Arc<NavigationStatuses>,
}

/// Validate offline products and motor compatibility without scripts or simulation advancement.
pub fn validate_navigation(snapshot: &SceneSnapshot, data: Option<&NavigationData>) -> Result<()> {
    if data.is_none()
        && snapshot.entities.iter().all(|e| {
            e.navigation_region.is_none()
                && e.navigation_agent
                    .as_ref()
                    .is_none_or(|agent| !agent.enabled)
        })
    {
        return snapshot.validate();
    }
    let physics = Physics::new(snapshot)?;
    Navigation::new(snapshot, &physics, data).map(|_| ())
}

impl Navigation {
    pub fn new(
        snapshot: &SceneSnapshot,
        physics: &Physics,
        data: Option<&NavigationData>,
    ) -> Result<Option<Self>> {
        let region = snapshot
            .entities
            .iter()
            .find_map(|e| e.navigation_region.as_ref());
        let Some(region) = region else {
            ensure!(data.is_none(), "navigation data has no region");
            ensure!(
                snapshot
                    .entities
                    .iter()
                    .all(|e| e.navigation_agent.as_ref().is_none_or(|a| !a.enabled)),
                "enabled navigation agent requires a region"
            );
            return Ok(None);
        };
        ensure!(region.data.is_some(), "navigation region is not baked");
        let data = data.context("navigation product is missing; bake the region before Play")?;
        data.validate_for(snapshot)?;
        let mesh = NavigationMesh::<Coordinates> {
            vertices: data.vertices.clone(),
            polygons: data
                .polygons
                .iter()
                .map(|p| p.iter().map(|&i| i as usize).collect())
                .collect(),
            polygon_type_indices: vec![0; data.polygons.len()],
            height_mesh: None,
        }
        .validate()
        .context("invalid Landmass navigation mesh")?;
        let mut archipelago =
            Archipelago::new(ArchipelagoOptions::from_agent_radius(region.agent_radius));
        archipelago.add_island(Island::new(
            landmass::Transform {
                translation: [0.; 3],
                rotation: 0.,
            },
            Arc::new(mesh),
        ));
        let mut agents = BTreeMap::new();
        let mut characters = BTreeMap::new();
        let mut entities: Vec<_> = snapshot.entities.iter().collect();
        entities.sort_by_key(|e| e.id);
        for entity in entities {
            let Some(character) = &entity.character else {
                continue;
            };
            let (position, velocity, radius, height) = physics
                .character_motion(entity.id)
                .context("navigation character has no motor")?;
            if let Some(config) = entity.navigation_agent.as_ref().filter(|a| a.enabled) {
                ensure!(
                    !matches!(
                        config.target,
                        Some(EntityRef::External { .. } | EntityRef::InInstance { .. })
                    ),
                    "navigation target is not loaded in this world"
                );
                ensure!(
                    region.agent_radius as f64 + 1e-6
                        >= radius as f64 + crate::character::COLLISION_OFFSET as f64,
                    "navigation bake radius does not cover capsule clearance"
                );
                ensure!(
                    region.agent_height as f64 + 1e-5 >= height as f64,
                    "navigation bake height does not cover capsule"
                );
                ensure!(
                    region.max_climb <= character.step_height
                        && region.max_slope_degrees <= character.max_slope_degrees,
                    "navigation profile exceeds character movement capabilities"
                );
                let mut agent = Agent::create(
                    position,
                    velocity,
                    radius,
                    character.move_speed,
                    character.move_speed,
                );
                agent.target_reached_condition =
                    TargetReachedCondition::StraightPathDistance(Some(config.arrival_distance));
                agents.insert(entity.id, archipelago.add_agent(agent));
            } else {
                characters.insert(
                    entity.id,
                    archipelago.add_character(Character {
                        position,
                        velocity,
                        radius,
                    }),
                );
            }
        }
        let statuses = Arc::new(
            agents
                .keys()
                .map(|&id| (id, NavigationStatus::Idle))
                .collect(),
        );
        Ok(Some(Self {
            archipelago,
            agents,
            characters,
            velocities: BTreeMap::new(),
            goals: BTreeMap::new(),
            statuses,
        }))
    }

    /// The owner supplies the UUID-ordered snapshot published by Scene::shared_snapshot.
    pub fn validate_commands(
        &self,
        snapshot: &SceneSnapshot,
        commands: &[NavigationCommand],
    ) -> Result<()> {
        for command in commands {
            let id = match *command {
                NavigationCommand::NavigateTo(id, point) => {
                    ensure!(
                        point.into_iter().all(f32::is_finite),
                        "navigation target must be finite"
                    );
                    id
                }
                NavigationCommand::Follow(id, target) => {
                    ensure!(
                        snapshot
                            .entities
                            .binary_search_by_key(&target, |entity| entity.id)
                            .is_ok(),
                        "stale navigation target"
                    );
                    id
                }
                NavigationCommand::Stop(id) => id,
            };
            ensure!(
                self.agents.contains_key(&id),
                "entity is not an enabled navigation agent"
            );
        }
        Ok(())
    }

    /// Called only after the complete world-command batch has passed validation and publication.
    pub fn apply_commands(&mut self, commands: &[NavigationCommand]) {
        for command in commands {
            let (id, goal) = match *command {
                NavigationCommand::NavigateTo(id, point) => (id, Goal::Point(point)),
                NavigationCommand::Follow(id, target) => (id, Goal::Entity(target)),
                NavigationCommand::Stop(id) => (id, Goal::Stopped),
            };
            // A stopped override must keep suppressing the author's default target.
            self.goals.insert(id, goal);
        }
    }

    pub fn statuses(&self) -> Arc<NavigationStatuses> {
        self.statuses.clone()
    }

    pub fn status(&self, entity: EntityId) -> Option<NavigationStatus> {
        self.statuses.get(&entity).copied()
    }

    pub fn velocities(
        &mut self,
        snapshot: &SceneSnapshot,
        physics: &Physics,
    ) -> Result<&BTreeMap<EntityId, [f32; 3]>> {
        let transforms = comet_scene::world_transforms(snapshot)?;
        // Scene::shared_snapshot publishes entities in persistent UUID order.
        for (&id, &agent_id) in &self.agents {
            let (position, velocity, _, _) = physics
                .character_motion(id)
                .context("navigation motor disappeared")?;
            let index = snapshot
                .entities
                .binary_search_by_key(&id, |entity| entity.id)
                .ok()
                .context("navigation entity disappeared")?;
            let entity = &snapshot.entities[index];
            let config = entity
                .navigation_agent
                .as_ref()
                .context("navigation component disappeared")?;
            let goal = match self.goals.get(&id) {
                Some(goal) => *goal,
                None => match &config.target {
                    None => Goal::Stopped,
                    Some(EntityRef::Local { entity }) => Goal::Entity(*entity),
                    Some(EntityRef::External { .. } | EntityRef::InInstance { .. }) => {
                        anyhow::bail!("navigation target is not loaded in this world")
                    }
                },
            };
            let target = match goal {
                Goal::Stopped => None,
                Goal::Point(point) => Some(point),
                Goal::Entity(entity) => Some(
                    physics.character_motion(entity).map(|m| m.0).unwrap_or(
                        transforms
                            .get(&entity)
                            .context("navigation target disappeared")?
                            .transform_point3(Vec3::ZERO)
                            .to_array(),
                    ),
                ),
            };
            let agent = self.archipelago.get_agent_mut(agent_id).unwrap();
            agent.position = position;
            agent.velocity = velocity;
            agent.current_target = target;
            agent.paused = target.is_none();
        }
        for (&id, &character_id) in &self.characters {
            let (position, velocity, _, _) = physics
                .character_motion(id)
                .context("navigation obstacle motor disappeared")?;
            let character = self.archipelago.get_character_mut(character_id).unwrap();
            character.position = position;
            character.velocity = velocity;
        }
        self.archipelago.update(FIXED_DELTA as f32);
        self.velocities.clear();
        let mut statuses = BTreeMap::new();
        for (&id, &agent_id) in &self.agents {
            let agent = self.archipelago.get_agent(agent_id).unwrap();
            let status = match agent.state() {
                AgentState::Idle | AgentState::Paused => NavigationStatus::Idle,
                AgentState::ReachedTarget => NavigationStatus::Arrived,
                AgentState::Moving => NavigationStatus::Moving,
                AgentState::AgentNotOnNavMesh => NavigationStatus::OffMesh,
                AgentState::TargetNotOnNavMesh | AgentState::NoPath => {
                    NavigationStatus::Unreachable
                }
                // This runtime does not install animation links or consume their motion.
                AgentState::ReachedAnimationLink | AgentState::UsingAnimationLink => {
                    NavigationStatus::Unreachable
                }
            };
            let velocity = if !agent.paused && status == NavigationStatus::Moving {
                *agent.get_desired_velocity()
            } else {
                [0.; 3]
            };
            ensure!(
                velocity.into_iter().all(f32::is_finite),
                "navigation produced a non-finite velocity"
            );
            self.velocities.insert(id, [velocity[0], 0., velocity[2]]);
            statuses.insert(id, status);
        }
        self.statuses = Arc::new(statuses);
        Ok(&self.velocities)
    }
}
