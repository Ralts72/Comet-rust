//! Rhai stays private to the runtime; callbacks see owned snapshots and publish validated commands.
mod api;
mod commands;
use crate::{
    FIXED_DELTA,
    animation::Animation,
    audio::Audio,
    hud::Hud,
    navigation::{Navigation, NavigationStatuses},
    physics::Physics,
    progress::Progress,
    session::SceneRequest,
};
use anyhow::{Context as _, Result, ensure};
use comet_core::{ActionInput, AssetId, EntityId, HudFrame, ProgressValues, WorldId};
use comet_scene::{Scene, SceneSnapshot, ScriptParameters};
use rhai::{AST, CallFnOptions, Dynamic, Engine, FLOAT, INT, Map, Scope};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

pub type ScriptLibrary = BTreeMap<AssetId, String>;
const MAX_BINDINGS: usize = 128;
const MAX_PHASE_OPERATIONS: u64 = 200_000;
const MAX_DIAGNOSTIC_BYTES: usize = 4096;
const CALLBACKS: [(&str, usize); 5] = [
    ("on_start", 1),
    ("on_fixed_update", 2),
    ("on_trigger", 3),
    ("on_animation_event", 3),
    ("on_stop", 1),
];

#[derive(Clone, Debug)]
pub struct ScriptDiagnostic {
    pub world: WorldId,
    pub sequence: u64,
    pub entity: EntityId,
    pub asset: AssetId,
    pub tick: u64,
    pub phase: &'static str,
    pub message: String,
    pub failed: bool,
}
#[derive(Default)]
struct Fuel {
    queries: Rc<Cell<usize>>,
    remaining: Cell<u64>,
    previous: Cell<u64>,
    exhausted: Cell<bool>,
}
fn engine(fuel: Rc<Fuel>) -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(20_000)
        .set_max_call_levels(32)
        .set_max_expr_depths(64, 32)
        .set_max_variables(128)
        .set_max_functions(128)
        .set_max_string_size(16_384)
        .set_max_array_size(1024)
        .set_max_map_size(256);
    engine
        .disable_symbol("eval")
        .on_print(|_| {})
        .on_debug(|_, _, _| {});
    engine.on_progress(move |operations| {
        let previous = fuel.previous.replace(operations);
        let delta = operations.saturating_sub(previous);
        let remaining = fuel.remaining.get();
        if delta > remaining {
            fuel.exhausted.set(true);
            fuel.remaining.set(0);
            Some("script phase operation budget exceeded".into())
        } else {
            fuel.remaining.set(remaining - delta);
            None
        }
    });
    api::register(&mut engine);
    engine
}

pub struct ScriptValidator {
    engine: Engine,
    fuel: Rc<Fuel>,
    programs: BTreeMap<AssetId, (String, Rc<AST>)>,
}
impl Default for ScriptValidator {
    fn default() -> Self {
        let fuel = Rc::new(Fuel::default());
        Self {
            engine: engine(fuel.clone()),
            fuel,
            programs: BTreeMap::new(),
        }
    }
}
impl ScriptValidator {
    /// Compile and check callback contracts without evaluating author code.
    pub fn validate(&mut self, snapshot: &SceneSnapshot, sources: &ScriptLibrary) -> Result<()> {
        snapshot.validate()?;
        let bound: Vec<_> = snapshot
            .entities
            .iter()
            .filter_map(|entity| entity.script.as_ref())
            .filter(|script| script.enabled)
            .collect();
        ensure!(
            bound.len() <= MAX_BINDINGS,
            "too many enabled script bindings (maximum {MAX_BINDINGS})"
        );
        for script in bound {
            let source = sources
                .get(&script.asset)
                .with_context(|| format!("missing script product {}", script.asset))?;
            ensure!(
                !source.trim().is_empty() && !source.contains('\0') && source.len() <= 65_536,
                "invalid script source {}",
                script.asset
            );
            let digest = comet_core::digest(source.as_bytes());
            if self
                .programs
                .get(&script.asset)
                .is_some_and(|(previous, _)| *previous == digest)
            {
                continue;
            }
            let mut ast = self
                .engine
                .compile(source)
                .map_err(|error| anyhow::anyhow!("script {}: {error}", script.asset))?;
            let mut found = false;
            for function in ast.iter_functions() {
                if let Some((_, arity)) = CALLBACKS.iter().find(|(name, _)| *name == function.name)
                {
                    ensure!(
                        function.params.len() == *arity,
                        "script {}: {} expects {} parameters",
                        script.asset,
                        function.name,
                        arity
                    );
                    found = true;
                }
            }
            ensure!(
                found,
                "script {} defines no lifecycle callback",
                script.asset
            );
            ast.set_source(script.asset.to_string());
            self.programs.insert(script.asset, (digest, Rc::new(ast)));
        }
        Ok(())
    }
}
pub fn validate_scripts(snapshot: &SceneSnapshot, sources: &ScriptLibrary) -> Result<()> {
    ScriptValidator::default().validate(snapshot, sources)
}
struct Binding {
    entity: EntityId,
    asset: AssetId,
    parameters: std::sync::Arc<ScriptParameters>,
    ast: Rc<AST>,
    scope: Scope<'static>,
    state: Dynamic,
    disabled: bool,
}
enum CallbackEvent<'a> {
    Trigger(EntityId, bool),
    Animation(&'a crate::animation::AnimationEventOccurrence),
}
pub(crate) struct Scripts {
    world: WorldId,
    engine: Engine,
    fuel: Rc<Fuel>,
    bindings: Vec<Binding>,
    diagnostics: VecDeque<ScriptDiagnostic>,
    sequence: u64,
    stopped: bool,
    scene_request: Option<SceneRequest>,
    navigation_statuses: Option<Arc<NavigationStatuses>>,
    hud: Hud,
    progress: Progress,
}
impl Scripts {
    pub fn new(
        snapshot: &SceneSnapshot,
        sources: &ScriptLibrary,
        world: WorldId,
        scope: Option<AssetId>,
        progress: Arc<ProgressValues>,
    ) -> Result<Self> {
        let mut validator = ScriptValidator::default();
        validator.validate(snapshot, sources)?;
        let mut bindings: Vec<_> = snapshot
            .entities
            .iter()
            .filter_map(|entity| {
                let script = entity.script.as_ref().filter(|script| script.enabled)?;
                Some(Binding {
                    entity: entity.id,
                    asset: script.asset,
                    parameters: std::sync::Arc::new(script.parameters.clone()),
                    ast: validator.programs[&script.asset].1.clone(),
                    scope: Scope::new(),
                    state: Map::new().into(),
                    disabled: false,
                })
            })
            .collect();
        bindings.sort_by_key(|binding| binding.entity);
        Ok(Self {
            world,
            engine: validator.engine,
            fuel: validator.fuel,
            bindings,
            diagnostics: VecDeque::new(),
            sequence: 0,
            stopped: false,
            scene_request: None,
            navigation_statuses: None,
            hud: Hud::new(world),
            progress: Progress::new(scope, progress)?,
        })
    }
    pub fn progress_snapshot(&self) -> Arc<ProgressValues> {
        self.progress.snapshot()
    }
    pub fn diagnostics(&self) -> &VecDeque<ScriptDiagnostic> {
        &self.diagnostics
    }
    pub fn hud_frame(&self) -> Option<HudFrame> {
        if self.stopped { None } else { self.hud.frame() }
    }
    pub fn inherit_diagnostics(&mut self, previous: &Self) {
        let current = std::mem::take(&mut self.diagnostics);
        self.diagnostics = previous.diagnostics.clone();
        self.sequence = previous.sequence;
        for entry in current {
            self.push_diagnostic(entry);
        }
    }
    pub fn take_scene_request(&mut self) -> Option<SceneRequest> {
        if self.stopped {
            return None;
        }
        self.scene_request.take()
    }
    pub fn reject_scene_request(&mut self, request: SceneRequest, reason: String) -> bool {
        if self.stopped || request.world != self.world {
            return false;
        }
        self.push_diagnostic(ScriptDiagnostic {
            world: request.world,
            sequence: 0,
            entity: request.entity,
            asset: request.script,
            tick: request.tick,
            phase: request.phase,
            message: format!("请求切换场景 {} 失败：{reason}", request.target),
            failed: true,
        });
        true
    }
    fn phase(&self) {
        self.fuel.queries.set(128);
        self.fuel.remaining.set(MAX_PHASE_OPERATIONS);
        self.fuel.exhausted.set(false);
    }
    fn record(
        &mut self,
        index: usize,
        tick: u64,
        phase: &'static str,
        message: String,
        failed: bool,
    ) {
        self.push_diagnostic(ScriptDiagnostic {
            world: self.world,
            sequence: 0,
            entity: self.bindings[index].entity,
            asset: self.bindings[index].asset,
            tick,
            phase,
            message,
            failed,
        });
    }
    fn push_diagnostic(&mut self, mut entry: ScriptDiagnostic) {
        if entry.message.len() > MAX_DIAGNOSTIC_BYTES {
            // Drop the original allocation too, so host errors cannot retain an oversized buffer.
            entry.message =
                entry.message[..entry.message.floor_char_boundary(MAX_DIAGNOSTIC_BYTES)].to_owned();
        }
        self.sequence = self.sequence.saturating_add(1);
        entry.sequence = self.sequence;
        if self.diagnostics.len() == 256 {
            self.diagnostics.pop_front();
        }
        self.diagnostics.push_back(entry);
    }
    pub fn start(
        &mut self,
        scene: &mut Scene,
        physics: &mut Physics,
        mut navigation: Option<&mut Navigation>,
        audio: &mut Audio,
        animation: &mut Animation,
    ) -> Result<()> {
        self.navigation_statuses = navigation.as_ref().map(|navigation| navigation.statuses());
        self.phase();
        for index in 0..self.bindings.len() {
            self.call(
                index,
                "on_start",
                0,
                &ActionInput::default(),
                scene,
                physics,
                navigation.as_deref_mut(),
                audio,
                animation,
                None,
                true,
            );
        }
        ensure!(
            !self.fuel.exhausted.get(),
            "script initialization exceeds the total phase operation budget"
        );
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn fixed(
        &mut self,
        tick: u64,
        input: &ActionInput,
        scene: &mut Scene,
        physics: &mut Physics,
        mut navigation: Option<&mut Navigation>,
        audio: &mut Audio,
        animation: &mut Animation,
    ) {
        // Fixed callbacks and the following trigger batch share the last steering result.
        self.navigation_statuses = navigation.as_ref().map(|navigation| navigation.statuses());
        self.phase();
        for index in 0..self.bindings.len() {
            self.call(
                index,
                "on_fixed_update",
                tick,
                input,
                scene,
                physics,
                navigation.as_deref_mut(),
                audio,
                animation,
                None,
                false,
            );
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn triggers(
        &mut self,
        tick: u64,
        input: &ActionInput,
        scene: &mut Scene,
        physics: &mut Physics,
        mut navigation: Option<&mut Navigation>,
        audio: &mut Audio,
        animation: &mut Animation,
    ) {
        self.phase();
        let events: Vec<_> = physics
            .events()
            .iter()
            .filter(|event| event.tick == tick)
            .copied()
            .collect();
        for event in events {
            if let Ok(index) = self
                .bindings
                .binary_search_by_key(&event.trigger, |binding| binding.entity)
            {
                self.call(
                    index,
                    "on_trigger",
                    tick,
                    input,
                    scene,
                    physics,
                    navigation.as_deref_mut(),
                    audio,
                    animation,
                    Some(CallbackEvent::Trigger(event.other, event.entered)),
                    false,
                );
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn animation_events(
        &mut self,
        events: &[crate::animation::AnimationEventOccurrence],
        tick: u64,
        input: &ActionInput,
        scene: &mut Scene,
        physics: &mut Physics,
        mut navigation: Option<&mut Navigation>,
        audio: &mut Audio,
        animation: &mut Animation,
    ) {
        self.phase();
        // A frozen post-sample batch: callbacks cannot recursively create events in this tick.
        for event in events {
            for index in 0..self.bindings.len() {
                self.call(
                    index,
                    "on_animation_event",
                    tick,
                    input,
                    scene,
                    physics,
                    navigation.as_deref_mut(),
                    audio,
                    animation,
                    Some(CallbackEvent::Animation(event)),
                    false,
                );
            }
        }
    }
    pub fn stop(
        &mut self,
        tick: u64,
        scene: &mut Scene,
        physics: &mut Physics,
        mut navigation: Option<&mut Navigation>,
        audio: &mut Audio,
        animation: &mut Animation,
    ) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.navigation_statuses = navigation.as_ref().map(|navigation| navigation.statuses());
        self.scene_request = None;
        self.phase();
        for index in 0..self.bindings.len() {
            self.call(
                index,
                "on_stop",
                tick,
                &ActionInput::default(),
                scene,
                physics,
                navigation.as_deref_mut(),
                audio,
                animation,
                None,
                false,
            );
        }
        self.navigation_statuses = None;
    }
    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        index: usize,
        name: &'static str,
        tick: u64,
        input: &ActionInput,
        scene: &mut Scene,
        physics: &mut Physics,
        navigation: Option<&mut Navigation>,
        audio: &mut Audio,
        animation: &mut Animation,
        event: Option<CallbackEvent<'_>>,
        initialize: bool,
    ) {
        if self.bindings[index].disabled || self.fuel.exhausted.get() {
            return;
        }
        let binding = &mut self.bindings[index];
        let has_callback = binding
            .ast
            .iter_functions()
            .any(|function| function.name == name);
        if !initialize && !has_callback {
            return;
        }
        let call = Rc::new(RefCell::new(api::Call {
            world: scene.world_id(),
            owner: binding.entity,
            snapshot: scene.shared_snapshot(),
            queries: physics.queries(),
            query_budget: self.fuel.queries.clone(),
            world_matrices: Default::default(),
            parameters: binding.parameters.clone(),
            input: input.clone(),
            time: tick as f64 * FIXED_DELTA,
            writable: name != "on_stop",
            commands: vec![],
            logs: vec![],
            scene_request: self.scene_request.as_ref().map(|request| request.target),
            navigation_statuses: self.navigation_statuses.clone(),
            animation_states: animation.states(),
            progress_scope: self.progress.scope(),
            progress: self.progress.snapshot(),
        }));
        let result = (|| -> Result<()> {
            self.fuel.previous.set(0);
            if initialize {
                self.engine
                    .run_ast_with_scope(&mut binding.scope, &binding.ast)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                self.fuel.previous.set(0);
            }
            if has_callback {
                let options = CallFnOptions::new()
                    .eval_ast(false)
                    .bind_this_ptr(&mut binding.state);
                let context = api::Context(call.clone());
                let result = match event {
                    Some(CallbackEvent::Trigger(other, entered)) => {
                        self.engine.call_fn_with_options::<Dynamic>(
                            options,
                            &mut binding.scope,
                            &binding.ast,
                            name,
                            (
                                context,
                                api::Entity::local(other, scene.world_id()),
                                entered,
                            ),
                        )
                    }
                    Some(CallbackEvent::Animation(event)) => {
                        self.engine.call_fn_with_options::<Dynamic>(
                            options,
                            &mut binding.scope,
                            &binding.ast,
                            name,
                            (
                                context,
                                api::Entity::local(event.animator, scene.world_id()),
                                event.name.clone(),
                            ),
                        )
                    }
                    None if name == "on_fixed_update" => {
                        self.engine.call_fn_with_options::<Dynamic>(
                            options,
                            &mut binding.scope,
                            &binding.ast,
                            name,
                            (context, FIXED_DELTA as FLOAT),
                        )
                    }
                    None => self.engine.call_fn_with_options::<Dynamic>(
                        options,
                        &mut binding.scope,
                        &binding.ast,
                        name,
                        (context,),
                    ),
                };
                let _ = result.map_err(|error| anyhow::anyhow!("{error}"))?;
            }
            let mut budget = StateBudget {
                nodes: 4096,
                bytes: 128 * 1024,
            };
            validate_state(&binding.state, 0, &mut budget)?;
            for (name, _, value) in binding.scope.iter_raw() {
                budget.charge(name.len())?;
                validate_state(value, 0, &mut budget)?;
            }
            if name != "on_stop" {
                commands::apply(
                    &call.borrow().commands,
                    scene,
                    physics,
                    navigation,
                    audio,
                    &mut self.hud,
                    animation,
                    &mut self.progress,
                )?;
                // Publication cannot fail after world commands commit on this owner thread.
                if self.scene_request.is_none()
                    && let Some(target) = call.borrow().scene_request
                {
                    self.scene_request = Some(SceneRequest {
                        target,
                        world: self.world,
                        entity: binding.entity,
                        script: binding.asset,
                        tick,
                        phase: name,
                    });
                }
            }
            Ok(())
        })();
        if result.is_ok() {
            for message in call.borrow_mut().logs.drain(..) {
                self.record(index, tick, name, message, false);
            }
        }
        if let Err(error) = result {
            self.bindings[index].disabled = true;
            self.bindings[index].state = Map::new().into();
            self.bindings[index].scope.clear();
            let message = if self.fuel.exhausted.get() {
                "script phase operation budget exceeded".to_owned()
            } else {
                format!("{error:#}")
            };
            self.record(index, tick, name, message, true);
        }
    }
}
struct StateBudget {
    nodes: usize,
    bytes: usize,
}
impl StateBudget {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .context("script state byte budget exceeded")?;
        Ok(())
    }
}
fn validate_state(value: &Dynamic, depth: usize, budget: &mut StateBudget) -> Result<()> {
    ensure!(
        depth <= 16 && budget.nodes > 0,
        "script state budget exceeded"
    );
    budget.nodes -= 1;
    budget.charge(std::mem::size_of::<Dynamic>())?;
    if value.is_unit() || value.is::<bool>() || value.is::<INT>() || value.is::<api::Entity>() {
        return Ok(());
    }
    if value.is::<FLOAT>() {
        ensure!(
            value.clone_cast::<FLOAT>().is_finite(),
            "script state number must be finite"
        );
        return Ok(());
    }
    if value.is::<rhai::ImmutableString>() {
        budget.charge(value.clone_cast::<rhai::ImmutableString>().len())?;
        return Ok(());
    }
    if let Some(values) = value.read_lock::<rhai::Array>() {
        for value in values.iter() {
            validate_state(value, depth + 1, budget)?;
        }
        return Ok(());
    }
    if let Some(values) = value.read_lock::<Map>() {
        for (key, value) in values.iter() {
            budget.charge(key.len())?;
            validate_state(value, depth + 1, budget)?;
        }
        return Ok(());
    }
    anyhow::bail!("runtime Context and native objects cannot be retained in script state")
}
