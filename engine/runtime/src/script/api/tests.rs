use super::*;
use crate::navigation::NavigationStatus;
use comet_scene::SceneSnapshot;

#[test]
fn follow_resolves_both_world_identities_and_stop_callbacks_only_query_navigation() {
    let owner = EntityId::new();
    let target = EntityId::new();
    let world = WorldId::new();
    let mut snapshot = SceneSnapshot::default();
    for id in [owner, target] {
        let mut entity = EntityData::new("entity");
        entity.id = id;
        snapshot.entities.push(entity);
    }
    snapshot.canonicalize();
    let call = Rc::new(RefCell::new(Call {
        world,
        owner,
        snapshot: Arc::new(snapshot),
        parameters: Arc::new(Default::default()),
        input: ActionInput::default(),
        time: 0.,
        writable: true,
        commands: vec![],
        logs: vec![],
        scene_request: None,
        progress_scope: None,
        progress: Arc::new(Default::default()),
        navigation_statuses: Some(Arc::new([(owner, NavigationStatus::Idle)].into())),
        animation_states: Arc::new(Default::default()),
    }));
    let mut engine = Engine::new();
    register(&mut engine);
    for foreign_owner in [false, true] {
        let mut scope = rhai::Scope::new();
        scope.push("ctx", Context(call.clone()));
        scope.push(
            "owner",
            Entity::local(owner, if foreign_owner { WorldId::new() } else { world }),
        );
        scope.push(
            "target",
            Entity::local(target, if foreign_owner { world } else { WorldId::new() }),
        );
        let error = engine
            .eval_with_scope::<()>(&mut scope, "ctx.navigate_to(owner, target);")
            .unwrap_err();
        assert!(error.to_string().contains("different runtime world"));
        assert!(call.borrow().commands.is_empty());
    }
    let mut scope = rhai::Scope::new();
    scope.push("ctx", Context(call.clone()));
    scope.push("owner", Entity::local(owner, world));
    scope.push("target", Entity::local(target, world));
    engine
        .eval_with_scope::<()>(&mut scope, "ctx.navigate_to(owner, target);")
        .unwrap();
    assert_eq!(call.borrow().commands.len(), 1);
    call.borrow_mut().commands.clear();
    call.borrow_mut().writable = false;
    assert_eq!(
        engine
            .eval_with_scope::<ImmutableString>(&mut scope, "ctx.navigation_status(owner)")
            .unwrap(),
        "idle"
    );
    let error = engine
        .eval_with_scope::<()>(&mut scope, "ctx.navigate_to(owner, target);")
        .unwrap_err();
    assert!(error.to_string().contains("on_stop"));
    assert!(call.borrow().commands.is_empty());
}

#[test]
fn animation_controls_and_queries_enforce_world_identity_stop_permissions_and_finite_speed() {
    let mut owner = EntityData::new("animation owner");
    let id = owner.id;
    let world = WorldId::new();
    owner.transform.translation = [1., 0., 0.];
    let call = Rc::new(RefCell::new(Call {
        world,
        owner: id,
        snapshot: Arc::new(SceneSnapshot {
            entities: vec![owner],
            ..Default::default()
        }),
        parameters: Arc::new(Default::default()),
        input: ActionInput::default(),
        time: 0.,
        writable: true,
        commands: vec![],
        logs: vec![],
        scene_request: None,
        progress_scope: None,
        progress: Arc::new(Default::default()),
        navigation_statuses: None,
        animation_states: Arc::new(
            [(
                id,
                crate::AnimationState {
                    clip: AssetId::new(),
                    state: None,
                    blend: 1.,
                    parameters: Arc::new(Default::default()),
                    time: 0.25,
                    duration: 1.,
                    playing: true,
                    speed: 1.,
                },
            )]
            .into(),
        ),
    }));
    let mut engine = Engine::new();
    register(&mut engine);
    let mut scope = rhai::Scope::new();
    scope.push("ctx", Context(call.clone()));
    scope.push("foreign", Entity::local(id, WorldId::new()));
    scope.push("owner", Entity::local(id, world));
    for expression in [
        "ctx.play_animation(foreign)",
        "ctx.pause_animation(foreign)",
        "ctx.resume_animation(foreign)",
        "ctx.set_animation_speed(foreign, 1)",
        "ctx.animation_state(foreign)",
    ] {
        let error = engine
            .eval_with_scope::<Dynamic>(&mut scope, expression)
            .unwrap_err();
        assert!(error.to_string().contains("different runtime world"));
        assert!(call.borrow().commands.is_empty());
    }
    for speed in [FLOAT::NAN, FLOAT::INFINITY, FLOAT::MAX] {
        scope.set_or_push("speed", speed);
        let error = engine
            .eval_with_scope::<()>(&mut scope, "ctx.set_animation_speed(owner, speed);")
            .unwrap_err();
        assert!(error.to_string().contains("finite and fit f32"));
        assert!(call.borrow().commands.is_empty());
    }
    call.borrow_mut().writable = false;
    let state = engine
        .eval_with_scope::<rhai::Map>(&mut scope, "ctx.animation_state(owner)")
        .unwrap();
    assert_eq!(state["time"].clone_cast::<FLOAT>(), 0.25);
    for expression in [
        "ctx.play_animation(owner);",
        "ctx.pause_animation(owner);",
        "ctx.resume_animation(owner);",
        "ctx.set_animation_speed(owner, 2);",
    ] {
        let error = engine
            .eval_with_scope::<()>(&mut scope, expression)
            .unwrap_err();
        assert!(error.to_string().contains("on_stop"));
        assert!(call.borrow().commands.is_empty());
    }
}
