//! Rapier objects never escape the independent Play world.
use crate::FIXED_DELTA;
use crate::character::{CharacterState, Motor};
use anyhow::{Context, Result, ensure};
use comet_core::{ActionInput, EntityId};
use comet_scene::{BodyMotion, ColliderShape, PhysicsBody, PoseUpdate, Scene, SceneSnapshot};
use rapier3d::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerEvent {
    pub trigger: EntityId,
    pub other: EntityId,
    pub entered: bool,
    pub tick: u64,
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
    pub fn new(snapshot: &SceneSnapshot) -> Result<Self> {
        snapshot.validate()?;
        let mut world = PhysicsWorld::new();
        world.integration_parameters.dt = FIXED_DELTA as f32;
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
            collider_ids.insert(collider, e.id);
            bodies.insert(
                e.id,
                Body {
                    handle,
                    collider,
                    motion: config.motion,
                    motor: e.character.as_ref().map(|c| Motor {
                        config: c.clone(),
                        state: CharacterState {
                            entity: e.id,
                            position: e.transform.translation,
                            grounded: false,
                            vertical_velocity: 0.,
                        },
                    }),
                },
            );
        }
        refresh_queries(&mut world);
        Ok(Self {
            world,
            bodies,
            collider_ids,
            overlaps: BTreeSet::new(),
            events: vec![],
        })
    }
    pub fn begin_frame(&mut self) {
        self.events.clear();
    }
    pub fn events(&self) -> &[TriggerEvent] {
        &self.events
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
    pub fn contains(&self, id: EntityId) -> bool {
        self.bodies.contains_key(&id)
    }
    pub fn step(&mut self, input: &ActionInput, tick: u64, scene: &mut Scene) -> Result<()> {
        for body in self.bodies.values_mut() {
            if let Some(motor) = &mut body.motor {
                let next = motor.next_pose(&self.world, body.handle, body.collider, input);
                self.world.bodies[body.handle].set_next_kinematic_position(next);
            }
        }
        self.world.step();
        ensure!(
            self.world.quarantine().is_empty(),
            "physics quarantined invalid simulation state"
        );
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
        self.collect_events(tick);
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
        let body = self
            .bodies
            .get_mut(&update.entity)
            .context("entity has no physics body")?;
        let rb = &mut self.world.bodies[body.handle];
        let position = pose(update.translation, update.rotation);
        rb.set_position(position, true);
        rb.set_next_kinematic_position(position);
        rb.set_linvel(Vector::ZERO, true);
        rb.set_angvel(Vector::ZERO, true);
        if let Some(motor) = &mut body.motor {
            motor.state.position = update.translation;
            motor.state.grounded = false;
            motor.state.vertical_velocity = 0.;
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
