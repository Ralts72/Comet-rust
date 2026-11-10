//! Rapier objects never escape the independent Play world.
use crate::FIXED_DELTA;
use crate::character::{CharacterState, Motor};
use anyhow::{Context, Result, ensure};
use comet_core::{ActionInput, EntityId};
use comet_scene::{BodyMotion, ColliderShape, PhysicsBody, PoseUpdate, Scene, SceneSnapshot};
use rapier3d::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

// Collect sensor transitions at each solver interval, including short crossings.
// Gameplay callbacks and character intent still run once per fixed tick.
const PHYSICS_SUBSTEPS: usize = 4;
mod joints;
pub(crate) mod queries;
pub use joints::{JointKind, JointStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerEvent {
    pub trigger: EntityId,
    pub other: EntityId,
    pub entered: bool,
    pub tick: u64,
}
pub(crate) enum PhysicsCommand {
    Teleport(PoseUpdate),
    Velocity(EntityId, [f32; 3]),
    Impulse(EntityId, [f32; 3]),
    CharacterMotion(EntityId, bool),
    CharacterFacing(EntityId, [f32; 2]),
}
struct Body {
    handle: RigidBodyHandle,
    collider: ColliderHandle,
    motion: BodyMotion,
    motor: Option<Motor>,
}
pub(crate) struct Physics {
    world: PhysicsWorld,
    bodies: BTreeMap<EntityId, Body>,
    collider_ids: HashMap<ColliderHandle, EntityId>,
    overlaps: BTreeSet<(EntityId, EntityId)>,
    events: Vec<TriggerEvent>,
    joints: joints::Joints,
    navigation_static: bool,
    query_snapshot: Option<std::sync::Arc<queries::Queries>>,
}
fn groups(body: &PhysicsBody) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_retain(body.layer),
        Group::from_bits_retain(body.mask),
        InteractionTestMode::And,
    )
}
fn pose(translation: [f32; 3], rotation: [f32; 4]) -> Pose {
    Pose::from_parts(
        Vector::from_array(translation),
        Rotation::from_array(rotation).normalize(),
    )
}
impl Physics {
    pub(crate) fn validate_commands(&self, commands: &[PhysicsCommand]) -> Result<()> {
        let mut candidates = BTreeMap::new();
        for command in commands {
            if let PhysicsCommand::CharacterMotion(id, _) | PhysicsCommand::CharacterFacing(id, _) =
                command
            {
                ensure!(
                    self.bodies.get(id).is_some_and(|b| b.motor.is_some()),
                    "character command requires a character"
                );
                if let PhysicsCommand::CharacterFacing(_, v) = command {
                    ensure!(
                        v.iter().all(|v| v.is_finite()) && (v[0] != 0. || v[1] != 0.),
                        "character facing must be finite and nonzero"
                    );
                }
                continue;
            }
            let (id, value) = match command {
                PhysicsCommand::Teleport(pose) => (pose.entity, pose.translation),
                PhysicsCommand::Velocity(id, value) | PhysicsCommand::Impulse(id, value) => {
                    (*id, *value)
                }
                _ => unreachable!(),
            };
            ensure!(
                value.into_iter().all(f32::is_finite),
                "physical script command must be finite"
            );
            let body = self.bodies.get(&id).context("entity has no physics body")?;
            let candidate = candidates
                .entry(id)
                .or_insert_with(|| self.world.bodies[body.handle].clone());
            match command {
                PhysicsCommand::Teleport(update) => {
                    ensure!(
                        !self.navigation_static || body.motion != BodyMotion::Fixed,
                        "navigation freezes fixed body poses"
                    );
                    candidate.set_position(pose(update.translation, update.rotation), true);
                    candidate.set_linvel(Vector::ZERO, true);
                    candidate.set_angvel(Vector::ZERO, true);
                }
                PhysicsCommand::Velocity(..) => {
                    ensure!(
                        body.motion != BodyMotion::Fixed && body.motor.is_none(),
                        "velocity requires a non-character moving body"
                    );
                    candidate.set_linvel(Vector::from_array(value), true);
                }
                PhysicsCommand::Impulse(..) => {
                    ensure!(
                        body.motion == BodyMotion::Dynamic,
                        "impulse requires a dynamic body"
                    );
                    candidate.apply_impulse(Vector::from_array(value), true);
                }
                _ => unreachable!(),
            }
            ensure!(
                candidate.linvel().is_finite(),
                "physical script command overflows velocity"
            );
        }
        Ok(())
    }
    pub fn new(snapshot: &SceneSnapshot) -> Result<Self> {
        snapshot.validate()?;
        let mut world = PhysicsWorld::new();
        world.integration_parameters.dt = FIXED_DELTA as f32 / PHYSICS_SUBSTEPS as f32;
        world.integration_parameters.num_solver_iterations = 1;
        let mut bodies = BTreeMap::new();
        let mut collider_ids = HashMap::new();
        // Stable insertion order also makes source entity ordering irrelevant to simulation.
        let mut entities: Vec<_> = snapshot.entities.iter().collect();
        entities.sort_by_key(|e| e.id);
        for e in entities {
            let Some(config) = &e.physics else {
                continue;
            };
            let builder = match config.motion {
                BodyMotion::Fixed => RigidBodyBuilder::fixed(),
                BodyMotion::Dynamic => RigidBodyBuilder::dynamic().ccd_enabled(true),
                BodyMotion::Kinematic if e.character.is_some() => {
                    RigidBodyBuilder::kinematic_position_based()
                }
                BodyMotion::Kinematic => RigidBodyBuilder::kinematic_velocity_based(),
            }
            .pose(pose(e.transform.translation, e.transform.rotation));
            let s = e.transform.scale;
            let shape = match config.shape {
                ColliderShape::Box { half_extents: h } => {
                    ColliderBuilder::cuboid(h[0] * s[0], h[1] * s[1], h[2] * s[2])
                }
                ColliderShape::Sphere { radius } => ColliderBuilder::ball(radius * s[0]),
                ColliderShape::Capsule {
                    half_height,
                    radius,
                } => ColliderBuilder::capsule_y(half_height * s[0], radius * s[0]),
            }
            .mass(config.mass)
            .friction(config.friction)
            .restitution(config.restitution)
            .sensor(config.sensor)
            .collision_groups(groups(config))
            .active_collision_types(ActiveCollisionTypes::all());
            let (handle, collider) = world.insert(builder, shape);
            let motor = if let Some(config) = &e.character {
                let capsule = world.colliders[collider]
                    .shape()
                    .as_capsule()
                    .context("character requires a capsule")?;
                Some(Motor::new(
                    config.clone(),
                    e.id,
                    e.transform.translation,
                    capsule.half_height(),
                    capsule.radius,
                ))
            } else {
                None
            };
            collider_ids.insert(collider, e.id);
            bodies.insert(
                e.id,
                Body {
                    handle,
                    collider,
                    motion: config.motion,
                    motor,
                },
            );
        }
        let joints = joints::Joints::new(&mut world, &bodies, snapshot)?;
        refresh_queries(&mut world);
        Ok(Self {
            query_snapshot: None,
            world,
            bodies,
            collider_ids,
            overlaps: BTreeSet::new(),
            events: vec![],
            joints,
            navigation_static: snapshot
                .entities
                .iter()
                .any(|e| e.navigation_region.is_some()),
        })
    }
    pub fn begin_frame(&mut self) {
        self.events.clear();
    }
    pub fn events(&self) -> &[TriggerEvent] {
        &self.events
    }
    pub fn joint_status(&self, entity: EntityId, kind: JointKind) -> Option<JointStatus> {
        self.joints.status(entity, kind)
    }
    pub fn player(&self) -> Option<&CharacterState> {
        self.bodies
            .values()
            .filter_map(|b| b.motor.as_ref())
            .find(|m| m.config.player_controlled)
            .map(|m| &m.state)
    }
    pub fn character(&self, id: EntityId) -> Option<&CharacterState> {
        self.bodies.get(&id)?.motor.as_ref().map(|m| &m.state)
    }
    /// Actual feet and velocity, not the previous navigation command.
    pub(crate) fn character_motion(&self, id: EntityId) -> Option<([f32; 3], [f32; 3], f32, f32)> {
        let body = self.bodies.get(&id)?;
        body.motor.as_ref()?;
        let capsule = self.world.colliders[body.collider].shape().as_capsule()?;
        let height = 2. * (capsule.half_height() + capsule.radius);
        let rigid = &self.world.bodies[body.handle];
        let mut feet = rigid.position().translation.to_array();
        feet[1] -= height * 0.5;
        Some((feet, rigid.linvel().to_array(), capsule.radius, height))
    }
    pub fn contains(&self, id: EntityId) -> bool {
        self.bodies.contains_key(&id)
    }
    pub(crate) fn queries(&mut self) -> std::sync::Arc<queries::Queries> {
        if self.query_snapshot.is_none() {
            self.query_snapshot = Some(std::sync::Arc::new(queries::Queries::capture(self)));
        }
        self.query_snapshot.as_ref().unwrap().clone()
    }
    pub(crate) fn set_character_motion_enabled(&mut self, id: EntityId, enabled: bool) {
        self.bodies
            .get_mut(&id)
            .unwrap()
            .motor
            .as_mut()
            .unwrap()
            .motion_enabled = enabled;
    }
    pub(crate) fn face_character(&mut self, id: EntityId, v: [f32; 2]) {
        let length = f64::from(v[0]).hypot(f64::from(v[1]));
        self.bodies
            .get_mut(&id)
            .unwrap()
            .motor
            .as_mut()
            .unwrap()
            .state
            .facing = Some(v.map(|x| (f64::from(x) / length) as f32));
    }
    pub fn step(
        &mut self,
        input: &ActionInput,
        tick: u64,
        scene: &mut Scene,
        navigation: Option<&BTreeMap<EntityId, [f32; 3]>>,
    ) -> Result<()> {
        self.query_snapshot = None;
        for body in self.bodies.values_mut() {
            if let Some(motor) = &mut body.motor
                && motor.update_posture(&mut self.world, body.handle, body.collider, input)
            {
                refresh_queries(&mut self.world);
            }
        }
        let mut targets = vec![];
        for (id, body) in &mut self.bodies {
            if let Some(motor) = &mut body.motor {
                let start = *self.world.bodies[body.handle].position();
                let next = motor.next_pose(
                    &self.world,
                    body.handle,
                    body.collider,
                    input,
                    navigation
                        .and_then(|velocities| velocities.get(id))
                        .copied(),
                );
                targets.push((body.handle, start, next));
            }
        }
        for substep in 1..=PHYSICS_SUBSTEPS {
            for &(handle, start, next) in &targets {
                let target = if substep == PHYSICS_SUBSTEPS {
                    next
                } else {
                    start.lerp(&next, substep as f32 / PHYSICS_SUBSTEPS as f32)
                };
                self.world.bodies[handle].set_next_kinematic_position(target);
            }
            self.world.step();
            ensure!(
                self.world.quarantine().is_empty(),
                "physics quarantined invalid simulation state"
            );
            self.collect_events(tick);
        }
        let mut updates = vec![];
        for (id, body) in &mut self.bodies {
            let position = self.world.bodies[body.handle].position();
            ensure!(
                position.translation.is_finite() && position.rotation.is_finite(),
                "physics produced a non-finite pose for {id}"
            );
            if let Some(motor) = &mut body.motor {
                motor.state.position = position.translation.to_array();
            }
            updates.push(PoseUpdate {
                entity: *id,
                translation: position.translation.to_array(),
                rotation: position.rotation.to_array(),
            });
        }
        scene.update_poses(&updates)?;
        Ok(())
    }
    fn collect_events(&mut self, tick: u64) {
        let mut overlaps = BTreeSet::new();
        for (a, b, intersecting) in self.world.narrow_phase.intersection_pairs() {
            if !intersecting {
                continue;
            }
            for (trigger, other) in [(a, b), (b, a)] {
                if self.world.colliders[trigger].is_sensor() {
                    overlaps.insert((self.collider_ids[&trigger], self.collider_ids[&other]));
                }
            }
        }
        for &(trigger, other) in overlaps.difference(&self.overlaps) {
            self.events.push(TriggerEvent {
                trigger,
                other,
                entered: true,
                tick,
            });
        }
        for &(trigger, other) in self.overlaps.difference(&overlaps) {
            self.events.push(TriggerEvent {
                trigger,
                other,
                entered: false,
                tick,
            });
        }
        self.overlaps = overlaps;
    }
    pub fn set_velocity(&mut self, id: EntityId, velocity: [f32; 3]) -> Result<()> {
        ensure!(
            velocity.into_iter().all(f32::is_finite),
            "velocity must be finite"
        );
        let body = self.bodies.get(&id).context("entity has no physics body")?;
        ensure!(
            body.motion != BodyMotion::Fixed && body.motor.is_none(),
            "velocity requires a dynamic body or a non-character kinematic body"
        );
        self.world.bodies[body.handle].set_linvel(Vector::from_array(velocity), true);
        Ok(())
    }
    pub fn apply_impulse(&mut self, id: EntityId, impulse: [f32; 3]) -> Result<()> {
        ensure!(
            impulse.into_iter().all(f32::is_finite),
            "impulse must be finite"
        );
        let body = self.bodies.get(&id).context("entity has no physics body")?;
        ensure!(
            body.motion == BodyMotion::Dynamic,
            "impulse requires a dynamic body"
        );
        self.world.bodies[body.handle].apply_impulse(Vector::from_array(impulse), true);
        Ok(())
    }
    pub fn teleport(&mut self, update: &PoseUpdate) -> Result<()> {
        self.query_snapshot = None;
        let body = self
            .bodies
            .get_mut(&update.entity)
            .context("entity has no physics body")?;
        let rb = &mut self.world.bodies[body.handle];
        ensure!(
            !self.navigation_static || body.motion != BodyMotion::Fixed,
            "navigation freezes fixed body poses"
        );
        let position = pose(update.translation, update.rotation);
        rb.set_position(position, true);
        rb.set_next_kinematic_position(position);
        rb.set_linvel(Vector::ZERO, true);
        rb.set_angvel(Vector::ZERO, true);
        if let Some(motor) = &mut body.motor {
            motor.state.position = update.translation;
            motor.state.grounded = false;
            motor.state.vertical_velocity = 0.;
            motor.state.horizontal_velocity = [0.; 2];
            motor.state.facing = None;
        }
        refresh_queries(&mut self.world);
        Ok(())
    }
}

fn refresh_queries(world: &mut PhysicsWorld) {
    // Populate queries without consuming the dynamics pipeline's creation/change flags.
    world
        .bodies
        .propagate_modified_body_positions_to_colliders(&mut world.colliders);
    for (handle, collider) in world.colliders.iter() {
        world.broad_phase.set_aabb(
            &world.integration_parameters,
            handle,
            collider.compute_aabb(),
        );
    }
}
