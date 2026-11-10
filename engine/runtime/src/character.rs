//! Capsule motor: gameplay intent to a collision-constrained kinematic pose.
use crate::FIXED_DELTA;
use comet_core::{Action, ActionInput, EntityId};
use comet_scene::Character;
use rapier3d::{
    control::{CharacterAutostep, CharacterLength, KinematicCharacterController},
    prelude::*,
};
pub(super) const COLLISION_OFFSET: f32 = 0.01;

#[derive(Clone, Debug)]
pub struct CharacterState {
    pub entity: EntityId,
    pub position: [f32; 3],
    pub grounded: bool,
    pub vertical_velocity: f32,
    pub crouched: bool,
    /// Collision-constrained horizontal movement, in world metres per second.
    pub horizontal_velocity: [f32; 2],
}
pub(super) struct Motor {
    pub config: Character,
    pub state: CharacterState,
    standing: SharedShape,
    crouching: SharedShape,
    height_change: f32,
}
impl Motor {
    pub fn new(
        config: Character,
        entity: EntityId,
        position: [f32; 3],
        half_height: f32,
        radius: f32,
    ) -> Self {
        let crouch_half_height =
            ((half_height + radius) * config.crouch_height_ratio - radius).max(0.);
        Self {
            config,
            state: CharacterState {
                entity,
                position,
                grounded: false,
                vertical_velocity: 0.,
                crouched: false,
                horizontal_velocity: [0.; 2],
            },
            standing: SharedShape::capsule_y(half_height, radius),
            crouching: SharedShape::capsule_y(crouch_half_height, radius),
            height_change: half_height - crouch_half_height,
        }
    }

    pub fn update_posture(
        &mut self,
        world: &mut PhysicsWorld,
        handle: RigidBodyHandle,
        collider_handle: ColliderHandle,
        input: &ActionInput,
    ) -> bool {
        let crouched =
            self.config.player_controlled && input.enabled && input.held.contains(Action::Crouch);
        if crouched == self.state.crouched {
            return false;
        }
        let shape = if crouched {
            &self.crouching
        } else {
            &self.standing
        };
        let mut next = *world.bodies[handle].position();
        // Resize around the feet rather than moving the bottom into the floor.
        next.translation.y += if crouched {
            -self.height_change
        } else {
            self.height_change
        };
        let queries = world.query_pipeline_with_filter(
            QueryFilter::default()
                .exclude_sensors()
                .exclude_rigid_body(handle)
                .groups(world.colliders[collider_handle].collision_groups()),
        );
        if !crouched
            && queries
                .intersect_shape(next, shape.as_ref())
                .next()
                .is_some()
        {
            return false;
        }
        world.colliders[collider_handle].set_shape(shape.clone());
        world.bodies[handle].set_position(next, true);
        world.bodies[handle].set_next_kinematic_position(next);
        self.state.crouched = crouched;
        true
    }

    pub fn next_pose(
        &mut self,
        world: &PhysicsWorld,
        handle: RigidBodyHandle,
        collider_handle: ColliderHandle,
        input: &ActionInput,
        navigation_velocity: Option<[f32; 3]>,
    ) -> Pose {
        let dt = FIXED_DELTA as f32;
        let c = &self.config;
        let controlled = c.player_controlled && input.enabled;
        let jump = controlled
            && input
                .events
                .iter()
                .any(|e| e.action == Action::Jump && e.pressed);
        if self.state.grounded && self.state.vertical_velocity < 0. {
            self.state.vertical_velocity = 0.;
        }
        if jump && self.state.grounded {
            self.state.vertical_velocity = c.jump_speed;
        }
        self.state.vertical_velocity -= c.gravity * dt;
        let axes = if controlled { input.movement } else { [0.; 2] };
        let mut direction = Vector::new(axes[0], 0., -axes[1]);
        if direction.length_squared() > 1. {
            direction = direction.normalize();
        }
        let speed = if self.state.crouched {
            c.crouch_speed
        } else if controlled && input.held.contains(Action::Run) {
            c.move_speed * c.run_multiplier
        } else {
            c.move_speed
        };
        let horizontal = if c.player_controlled {
            direction * speed
        } else {
            Vector::from_array(navigation_velocity.unwrap_or([0.; 3]))
        };
        let desired = horizontal * dt + Vector::Y * self.state.vertical_velocity * dt;
        let controller = KinematicCharacterController {
            offset: CharacterLength::Absolute(COLLISION_OFFSET),
            autostep: (c.step_height > 0.).then_some(CharacterAutostep {
                max_height: CharacterLength::Absolute(c.step_height),
                // Require a landing width matching the capsule diameter.
                min_width: CharacterLength::Relative(1.0),
                include_dynamic_bodies: false,
            }),
            max_slope_climb_angle: c.max_slope_degrees.to_radians(),
            min_slope_slide_angle: c.max_slope_degrees.to_radians(),
            snap_to_ground: (c.snap_distance > 0.)
                .then_some(CharacterLength::Absolute(c.snap_distance)),
            // Leave clearance for repeated f32 floor casts, including shortened capsules.
            normal_nudge_factor: 0.005,
            ..Default::default()
        };
        let collider = &world.colliders[collider_handle];
        let current = *world.bodies[handle].position();
        let queries = world.query_pipeline_with_filter(
            QueryFilter::default()
                .exclude_sensors()
                .exclude_rigid_body(handle)
                .groups(collider.collision_groups()),
        );
        let movement =
            controller.move_shape(dt, &queries, collider.shape(), &current, desired, |_| {});
        self.state.horizontal_velocity = [movement.translation.x / dt, movement.translation.z / dt];
        self.state.grounded = movement.grounded;
        if (movement.grounded && self.state.vertical_velocity < 0.)
            || (self.state.vertical_velocity > 0. && movement.translation.y + 1e-4 < desired.y)
        {
            self.state.vertical_velocity = 0.;
        }
        let mut next = current;
        next.translation += movement.translation;
        next
    }
}
