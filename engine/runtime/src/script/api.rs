use super::commands::Command;
use crate::{animation::AnimationStates, navigation::NavigationStatuses};
use comet_core::{
    Action, ActionInput, AssetId, EntityId, ProgressValue, ProgressValues, WorldId,
    math::{Quat, Vec3},
    validate_progress_key,
};
use comet_scene::{
    EntityData, EntityRef, SceneSnapshot, ScriptParameters, ScriptValue,
    schema::{self, FieldValue},
};
use rhai::{Dynamic, Engine, EvalAltResult, FLOAT, INT, ImmutableString};
use std::{cell::RefCell, rc::Rc, sync::Arc};

type ScriptResult<T> = Result<T, Box<EvalAltResult>>;
const MAX_COMMANDS: usize = 256;
const MAX_LOGS_PER_CALL: usize = 32;

#[derive(Clone, Debug)]
pub(super) struct Entity {
    reference: EntityRef,
    world: WorldId,
}
impl Entity {
    pub fn local(id: EntityId, world: WorldId) -> Self {
        Self {
            reference: EntityRef::Local { entity: id },
            world,
        }
    }
    fn resolve(&self, call: &Call) -> ScriptResult<EntityId> {
        if self.world != call.world {
            return Err("entity belongs to a different runtime world".into());
        }
        let EntityRef::Local { entity } = self.reference else {
            return Err("external scene entity is not loaded in this world".into());
        };
        call.entity(entity)?;
        Ok(entity)
    }
}

pub(super) struct Call {
    pub queries: Arc<crate::physics::queries::Queries>,
    pub query_budget: Rc<std::cell::Cell<usize>>,
    pub world_matrices:
        std::cell::OnceCell<std::collections::BTreeMap<EntityId, comet_core::math::Mat4>>,
    pub world: WorldId,
    pub owner: EntityId,
    pub snapshot: Arc<SceneSnapshot>,
    pub parameters: Arc<ScriptParameters>,
    pub input: ActionInput,
    pub time: f64,
    pub writable: bool,
    pub commands: Vec<Command>,
    pub logs: Vec<String>,
    pub scene_request: Option<AssetId>,
    pub navigation_statuses: Option<Arc<NavigationStatuses>>,
    pub animation_states: Arc<AnimationStates>,
    pub progress_scope: Option<AssetId>,
    pub progress: Arc<ProgressValues>,
}
impl Call {
    fn entity(&self, id: EntityId) -> ScriptResult<&EntityData> {
        // Scene publishes snapshots in stable UUID order, including after field/pose updates.
        self.snapshot
            .entities
            .binary_search_by_key(&id, |entity| entity.id)
            .map(|index| &self.snapshot.entities[index])
            .map_err(|_| "stale script entity".into())
    }
}
#[derive(Clone)]
pub(super) struct Context(pub Rc<RefCell<Call>>);
impl Context {
    fn progress(&mut self, key: &str, default: Dynamic) -> ScriptResult<Dynamic> {
        let owner = self.0.borrow().owner;
        self.progress_for(owner, key, default)
    }
    fn progress_of(&mut self, owner: Entity, key: &str, default: Dynamic) -> ScriptResult<Dynamic> {
        let owner = owner.resolve(&self.0.borrow())?;
        self.progress_for(owner, key, default)
    }
    fn progress_for(&self, owner: EntityId, key: &str, default: Dynamic) -> ScriptResult<Dynamic> {
        let call = self.0.borrow();
        let scene = call
            .progress_scope
            .ok_or("progress requires a scene asset scope")?;
        validate_progress_key(key).map_err(|error| error.to_string())?;
        let default = progress_value(default)?;
        let value = call.progress.get(scene, owner, key).unwrap_or(&default);
        if !value.same_type(&default) {
            return Err("progress default type does not match its existing value".into());
        }
        Ok(match value {
            ProgressValue::Bool(value) => Dynamic::from(*value),
            ProgressValue::Integer(value) => Dynamic::from(*value),
            ProgressValue::Number(value) => Dynamic::from(*value),
            ProgressValue::Text(value) => Dynamic::from(value.clone()),
        })
    }
    fn set_progress(&mut self, key: &str, value: Dynamic) -> ScriptResult<()> {
        self.progress_scope()?;
        validate_progress_key(key).map_err(|error| error.to_string())?;
        let value = progress_value(value)?;
        self.enqueue_owner(|owner| Command::SetProgress(owner, key.into(), value))
    }
    fn remove_progress(&mut self, key: &str) -> ScriptResult<()> {
        self.progress_scope()?;
        validate_progress_key(key).map_err(|error| error.to_string())?;
        self.enqueue_owner(|owner| Command::RemoveProgress(owner, key.into()))
    }
    fn progress_scope(&self) -> ScriptResult<()> {
        self.0
            .borrow()
            .progress_scope
            .ok_or_else(|| "progress requires a scene asset scope".into())
            .map(|_| ())
    }
    fn request_scene(&mut self, target: &str) -> ScriptResult<bool> {
        let mut call = self.0.borrow_mut();
        if !call.writable {
            return Err("scene commands are unavailable during on_stop".into());
        }
        let target = target
            .parse::<AssetId>()
            .map_err(|error| Box::<EvalAltResult>::from(format!("invalid scene UUID: {error}")))?;
        if let Some(pending) = call.scene_request {
            return Ok(pending == target);
        }
        call.scene_request = Some(target);
        Ok(true)
    }
    fn enqueue(
        &mut self,
        entity: Entity,
        build: impl FnOnce(EntityId) -> Command,
    ) -> ScriptResult<()> {
        let mut call = self.0.borrow_mut();
        if !call.writable {
            return Err("scene commands are unavailable during on_stop".into());
        }
        let id = entity.resolve(&call)?;
        if call.commands.len() >= MAX_COMMANDS {
            return Err("script command budget exceeded".into());
        }
        call.commands.push(build(id));
        Ok(())
    }
    fn parameter(&mut self, name: &str) -> ScriptResult<Dynamic> {
        let call = self.0.borrow();
        let value = call.parameters.get(name).ok_or_else(|| {
            Box::<EvalAltResult>::from(format!("unknown script parameter {name}"))
        })?;
        Ok(match value {
            ScriptValue::Bool(value) => (*value).into(),
            ScriptValue::Integer(value) => (*value as INT).into(),
            ScriptValue::Number(value) => (*value as FLOAT).into(),
            ScriptValue::Text(value) => value.clone().into(),
            ScriptValue::Vec3(value) => numbers(*value).into(),
            ScriptValue::Entity(None) => Dynamic::UNIT,
            ScriptValue::Entity(Some(reference)) => Dynamic::from(Entity {
                reference: reference.clone(),
                world: call.world,
            }),
        })
    }
    fn enqueue_owner(&mut self, build: impl FnOnce(EntityId) -> Command) -> ScriptResult<()> {
        let owner = {
            let call = self.0.borrow();
            Entity::local(call.owner, call.world)
        };
        self.enqueue(owner, build)
    }
}
fn progress_value(value: Dynamic) -> ScriptResult<ProgressValue> {
    let value = if value.is::<bool>() {
        ProgressValue::Bool(value.cast())
    } else if value.is::<INT>() {
        ProgressValue::Integer(value.cast())
    } else if value.is::<FLOAT>() {
        ProgressValue::Number(value.cast())
    } else if value.is::<ImmutableString>() {
        ProgressValue::Text(value.cast::<ImmutableString>().to_string())
    } else {
        return Err("progress supports only bool, integer, finite number or text".into());
    };
    value.validate().map_err(|error| error.to_string())?;
    Ok(value)
}
fn number(value: Dynamic) -> ScriptResult<f32> {
    let value = if value.is::<FLOAT>() {
        value.cast::<FLOAT>()
    } else if value.is::<INT>() {
        value.cast::<INT>() as FLOAT
    } else {
        return Err("expected a number".into());
    };
    let result = value as f32;
    if !value.is_finite() || !result.is_finite() {
        return Err("script command number must be finite and fit f32".into());
    }
    Ok(result)
}
fn vector(x: Dynamic, y: Dynamic, z: Dynamic) -> ScriptResult<[f32; 3]> {
    Ok([number(x)?, number(y)?, number(z)?])
}
fn numbers<const N: usize>(values: [f32; N]) -> rhai::Array {
    values
        .into_iter()
        .map(|value| (value as FLOAT).into())
        .collect()
}
fn rotation(x: Dynamic, y: Dynamic, z: Dynamic, w: Dynamic) -> ScriptResult<[f32; 4]> {
    let value = [number(x)?, number(y)?, number(z)?, number(w)?];
    schema::descriptor(schema::ROTATION)
        .and_then(|field| field.validate_value(&FieldValue::Quat(value)))
        .map_err(|error| Box::<EvalAltResult>::from(error.to_string()))?;
    Ok(Quat::from_array(value).normalize().to_array())
}
fn action(name: &str) -> ScriptResult<Action> {
    match name {
        "jump" => Ok(Action::Jump),
        "run" => Ok(Action::Run),
        "crouch" => Ok(Action::Crouch),
        "interact" => Ok(Action::Interact),
        "primary" => Ok(Action::Primary),
        _ => Err(format!("unknown gameplay action {name}").into()),
    }
}

mod spatial;
#[cfg(test)]
mod tests;

pub(super) fn register(engine: &mut Engine) {
    spatial::register(engine);
    engine.register_fn("destroy_entity", |context: &mut Context, entity: Entity| {
        context.enqueue(entity, Command::Destroy)
    });
    engine.register_fn("exists", |context: &mut Context, entity: Entity| -> ScriptResult<bool> {
        let call = context.0.borrow();
        if entity.world != call.world {
            return Err("entity belongs to a different runtime world".into());
        }
        Ok(matches!(entity.reference, EntityRef::Local { entity } if call.entity(entity).is_ok()))
    });
    engine.register_fn(
        "set_character_motion_enabled",
        |context: &mut Context, entity: Entity, enabled: bool| -> ScriptResult<()> {
            context.enqueue(entity, |id| Command::CharacterMotion(id, enabled))
        },
    );
    engine.register_fn(
        "face_character",
        |context: &mut Context, entity: Entity, x: Dynamic, z: Dynamic| -> ScriptResult<()> {
            let direction = [number(x)?, number(z)?];
            context.enqueue(entity, |id| Command::CharacterFacing(id, direction))
        },
    );
    engine
        .register_type_with_name::<Context>("Context")
        .register_type_with_name::<Entity>("Entity");
    engine.register_fn("entity", |context: &mut Context| {
        let call = context.0.borrow();
        Entity::local(call.owner, call.world)
    });
    engine.register_fn("id", |entity: &mut Entity| -> ScriptResult<String> {
        match entity.reference {
            EntityRef::Local { entity } | EntityRef::External { entity, .. } => {
                Ok(entity.to_string())
            }
            EntityRef::InInstance { .. } => {
                Err("unresolved prefab instance entity reference".into())
            }
        }
    });
    engine.register_fn("param", Context::parameter);
    engine.register_fn("request_scene", Context::request_scene);
    engine.register_fn("progress", Context::progress);
    engine.register_fn("progress_of", Context::progress_of);
    engine.register_fn("set_progress", Context::set_progress);
    engine.register_fn("remove_progress", Context::remove_progress);
    engine.register_fn("set_hud", |context: &mut Context, text: ImmutableString| {
        context.enqueue_owner(|owner| Command::SetHud(owner, Arc::from(text.as_str())))
    });
    engine.register_fn("clear_hud", |context: &mut Context| {
        context.enqueue_owner(Command::ClearHud)
    });
    engine.register_fn("has_param", |context: &mut Context, name: &str| {
        context.0.borrow().parameters.contains_key(name)
    });
    engine.register_fn("time", |context: &mut Context| {
        context.0.borrow().time as FLOAT
    });
    engine.register_fn(
        "held",
        |context: &mut Context, name: &str| -> ScriptResult<bool> {
            Ok(context.0.borrow().input.held.contains(action(name)?))
        },
    );
    engine.register_fn(
        "pressed",
        |context: &mut Context, name: &str| -> ScriptResult<bool> {
            let action = action(name)?;
            Ok(context
                .0
                .borrow()
                .input
                .events
                .iter()
                .any(|event| event.action == action && event.pressed))
        },
    );
    engine.register_fn(
        "released",
        |context: &mut Context, name: &str| -> ScriptResult<bool> {
            let action = action(name)?;
            Ok(context
                .0
                .borrow()
                .input
                .events
                .iter()
                .any(|event| event.action == action && !event.pressed))
        },
    );
    engine.register_fn("move_x", |context: &mut Context| {
        context.0.borrow().input.movement[0] as FLOAT
    });
    engine.register_fn("move_y", |context: &mut Context| {
        context.0.borrow().input.movement[1] as FLOAT
    });
    engine.register_fn(
        "position",
        |context: &mut Context, entity: Entity| -> ScriptResult<rhai::Array> {
            let call = context.0.borrow();
            Ok(numbers(
                call.entity(entity.resolve(&call)?)?.transform.translation,
            ))
        },
    );
    engine.register_fn(
        "rotation",
        |context: &mut Context, entity: Entity| -> ScriptResult<rhai::Array> {
            let call = context.0.borrow();
            Ok(numbers(
                call.entity(entity.resolve(&call)?)?.transform.rotation,
            ))
        },
    );
    engine.register_fn(
        "scale",
        |context: &mut Context, entity: Entity| -> ScriptResult<rhai::Array> {
            let call = context.0.borrow();
            Ok(numbers(
                call.entity(entity.resolve(&call)?)?.transform.scale,
            ))
        },
    );
    for (name, build) in [
        (
            "set_position",
            Command::Position as fn(EntityId, [f32; 3]) -> Command,
        ),
        ("translate_local", Command::Translate),
        ("set_scale", Command::Scale),
        ("teleport", Command::Teleport),
        ("set_velocity", Command::Velocity),
        ("impulse", Command::Impulse),
        ("navigate_to", Command::NavigateTo),
    ] {
        engine.register_fn(
            name,
            move |context: &mut Context,
                  entity: Entity,
                  x: Dynamic,
                  y: Dynamic,
                  z: Dynamic|
                  -> ScriptResult<()> {
                let value = vector(x, y, z)?;
                context.enqueue(entity, |id| build(id, value))
            },
        );
    }
    engine.register_fn(
        "navigate_to",
        |context: &mut Context, owner: Entity, target: Entity| -> ScriptResult<()> {
            let target = {
                let call = context.0.borrow();
                target.resolve(&call)?
            };
            context.enqueue(owner, |owner| Command::FollowNavigation(owner, target))
        },
    );
    engine.register_fn(
        "stop_navigation",
        |context: &mut Context, entity: Entity| -> ScriptResult<()> {
            context.enqueue(entity, Command::StopNavigation)
        },
    );
    engine.register_fn(
        "navigation_status",
        |context: &mut Context, entity: Entity| -> ScriptResult<ImmutableString> {
            let call = context.0.borrow();
            let id = entity.resolve(&call)?;
            let states = call.navigation_statuses.as_ref().ok_or_else(|| {
                Box::<EvalAltResult>::from("navigation is not available in this runtime world")
            })?;
            let state = states.get(&id).ok_or_else(|| {
                Box::<EvalAltResult>::from("entity has no enabled navigation agent")
            })?;
            Ok(state.as_str().into())
        },
    );
    engine.register_fn(
        "play_sound",
        |context: &mut Context, entity: Entity| -> ScriptResult<()> {
            context.enqueue(entity, Command::PlaySound)
        },
    );
    for (name, command) in [
        (
            "play_animation",
            Command::PlayAnimation as fn(EntityId) -> Command,
        ),
        ("pause_animation", Command::PauseAnimation),
        ("resume_animation", Command::ResumeAnimation),
    ] {
        engine.register_fn(
            name,
            move |context: &mut Context, entity: Entity| -> ScriptResult<()> {
                context.enqueue(entity, command)
            },
        );
    }
    engine.register_fn(
        "set_animation_speed",
        |context: &mut Context, entity: Entity, speed: Dynamic| -> ScriptResult<()> {
            let speed = number(speed)?;
            context.enqueue(entity, |id| Command::AnimationSpeed(id, speed))
        },
    );
    engine.register_fn(
        "set_animation_parameter",
        |context: &mut Context, entity: Entity, name: &str, value: Dynamic| -> ScriptResult<()> {
            let value = if value.is::<bool>() {
                comet_asset_data::AnimationParameter::Bool(value.cast())
            } else {
                comet_asset_data::AnimationParameter::Number(number(value)?)
            };
            context.enqueue(entity, |id| {
                Command::AnimationParameter(id, name.into(), value)
            })
        },
    );
    engine.register_fn(
        "animation_parameter",
        |context: &mut Context, entity: Entity, name: &str| -> ScriptResult<Dynamic> {
            let call = context.0.borrow();
            let id = entity.resolve(&call)?;
            let value = call
                .animation_states
                .get(&id)
                .and_then(|state| state.parameters.get(name))
                .ok_or("unknown animation controller parameter")?;
            Ok(match value {
                comet_asset_data::AnimationParameter::Bool(value) => (*value).into(),
                comet_asset_data::AnimationParameter::Number(value) => (*value as FLOAT).into(),
            })
        },
    );
    engine.register_fn(
        "animation_state",
        |context: &mut Context, entity: Entity| -> ScriptResult<rhai::Map> {
            let call = context.0.borrow();
            let id = entity.resolve(&call)?;
            let state = call
                .animation_states
                .get(&id)
                .ok_or_else(|| Box::<EvalAltResult>::from("entity has no selected animation"))?;
            Ok([
                ("time".into(), Dynamic::from(state.time as FLOAT)),
                ("duration".into(), Dynamic::from(state.duration as FLOAT)),
                ("playing".into(), Dynamic::from(state.playing)),
                ("speed".into(), Dynamic::from(state.speed as FLOAT)),
                (
                    "state".into(),
                    Dynamic::from(state.state.as_deref().unwrap_or("").to_owned()),
                ),
                ("blend".into(), Dynamic::from(state.blend as FLOAT)),
                ("clip".into(), Dynamic::from(state.clip.to_string())),
            ]
            .into())
        },
    );
    engine.register_fn(
        "stop_sound",
        |context: &mut Context, entity: Entity| -> ScriptResult<()> {
            context.enqueue(entity, Command::StopSound)
        },
    );
    engine.register_fn(
        "set_sound_volume",
        |context: &mut Context, entity: Entity, value: Dynamic| -> ScriptResult<()> {
            let gain = number(value)?;
            context.enqueue(entity, |id| Command::SoundVolume(id, gain))
        },
    );
    engine.register_fn(
        "set_rotation",
        |context: &mut Context,
         entity: Entity,
         x: Dynamic,
         y: Dynamic,
         z: Dynamic,
         w: Dynamic|
         -> ScriptResult<()> {
            let value = rotation(x, y, z, w)?;
            context.enqueue(entity, |id| Command::Rotation(id, value))
        },
    );
    engine.register_fn(
        "rotate_local",
        |context: &mut Context,
         entity: Entity,
         x: Dynamic,
         y: Dynamic,
         z: Dynamic,
         radians: Dynamic|
         -> ScriptResult<()> {
            let axis = Vec3::from_array(vector(x, y, z)?);
            if !axis.length_squared().is_finite() || (axis.length_squared() - 1.).abs() > 1e-4 {
                return Err("rotation axis must be a finite unit local direction".into());
            }
            let axis = axis.normalize().to_array();
            let radians = number(radians)?;
            context.enqueue(entity, |id| Command::RotateLocal(id, axis, radians))
        },
    );
    engine.register_fn(
        "set_roughness",
        |context: &mut Context, entity: Entity, value: Dynamic| -> ScriptResult<()> {
            let value = number(value)?;
            context.enqueue(entity, |id| Command::Roughness(id, value))
        },
    );
    engine.register_fn(
        "log",
        |context: &mut Context, message: ImmutableString| -> ScriptResult<()> {
            let mut call = context.0.borrow_mut();
            if call.logs.len() >= MAX_LOGS_PER_CALL || message.len() > 4096 {
                return Err("script log budget exceeded".into());
            }
            call.logs.push(message.into());
            Ok(())
        },
    );
}
