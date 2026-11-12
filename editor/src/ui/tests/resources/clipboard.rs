use super::*;

fn shortcut(key: egui::Key) -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        })
        .into()
}

#[test]
fn clipboard_menus_and_native_events_keep_links_selection_and_text_input_isolated() -> Result<()> {
    let mut c = Case::new(true)?;
    c.frame(vec![]);
    c.press(c.resource, resources());
    c.press("添加到场景", resources());
    let before = c.editor.document.shared_source();
    let original = before.instances[0].id;
    let roots = c.roots(original);
    c.editor.selection.replace(roots);
    c.context_menu("落体乙");
    c.press("复制到剪贴板", Rect::EVERYTHING);
    assert!(c.editor.clipboard.is_some());
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    assert_eq!(c.editor.selection.active(), Some(roots[1]));
    assert_eq!(c.editor.document.history_len(), 1);
    c.key(egui::Key::Delete, egui::Modifiers::NONE);
    assert!(c.editor.document.shared_source().instances.is_empty());
    let physical = c.editor.project_root().join("assets/rigid.prefab");
    let bytes = fs::read(&physical)?;
    let mut changed = ContentSource::from_json(&bytes)?;
    let old = changed.base.entities[0].id;
    let replacement = EntityId::new();
    for entity in &mut changed.base.entities {
        if entity.id == old {
            entity.id = replacement;
        }
        comet_scene::schema::remap(entity, &[(old, replacement)].into())?;
    }
    fs::write(&physical, changed.to_json()?)?;
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    c.assets.refresh()?;
    c.frame(vec![egui::Event::Paste(String::new())]);
    assert!(
        c.editor
            .error
            .as_deref()
            .is_some_and(|error| error.contains("资源根已变化"))
    );
    assert!(c.editor.document.shared_source().instances.is_empty());
    assert_eq!(c.editor.document.history_len(), 2);
    assert!(c.editor.clipboard.is_some());
    fs::write(&physical, bytes)?;
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    c.assets.refresh()?;
    c.editor.error = None;
    c.frame(shortcut(egui::Key::Paste));
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let pasted = c.editor.document.shared_source();
    assert_eq!(pasted.instances.len(), 1);
    assert_ne!(pasted.instances[0].id, original);
    assert_eq!(pasted.instances[0].prefab, before.instances[0].prefab);
    assert_eq!(c.editor.selection.len(), 2);
    assert_eq!(
        c.editor.selection.active(),
        Some(c.roots(pasted.instances[0].id)[1])
    );
    assert_eq!(c.editor.document.history_len(), 3);
    let mut events = shortcut(egui::Key::V);
    events.extend(shortcut(egui::Key::Paste));
    events.push(egui::Event::Paste(String::new()));
    c.frame(events);
    assert_eq!(c.editor.document.shared_source().instances.len(), 2);
    assert_eq!(
        c.editor.document.history_len(),
        4,
        "native and key events paste once"
    );
    c.context_menu("场景根节点");
    c.press("粘贴", Rect::EVERYTHING);
    assert_eq!(c.editor.document.shared_source().instances.len(), 3);
    assert_eq!(c.editor.document.history_len(), 5);

    let script = c.editor.project_root().join("assets/clipboard.rhai");
    let script_bytes = b"fn on_start(ctx) {}";
    fs::write(&script, script_bytes)?;
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    c.assets.refresh()?;
    let script_id = comet_asset_pipeline::SourceMeta::load(&script)?.id;
    let mut parent = EntityData::new("粘贴父级");
    parent.script = Some(comet_scene::Script::new(script_id));
    let parent_id = parent.id;
    c.editor.document.insert_entity(parent, &mut c.assets)?;
    c.editor.selection.replace([parent_id]);
    let before_failure = c.editor.document.shared_source();
    let history = c.editor.document.history_len();
    c.context_menu("粘贴父级");
    c.press("粘贴", Rect::EVERYTHING);
    assert!(
        c.editor.error.is_some(),
        "physical roots cannot acquire an author parent"
    );
    assert!(Arc::ptr_eq(
        &before_failure,
        &c.editor.document.shared_source()
    ));
    assert_eq!(c.editor.document.history_len(), history);
    assert_eq!(c.editor.selection.ids(), [parent_id]);
    assert!(c.editor.clipboard.is_some());
    c.editor.error = None;

    // Replace the fragment with a self-contained author subtree through native Copy.
    let mut child = EntityData::new("子实体");
    child.parent = Some(parent_id);
    child.links.target = Some(comet_scene::EntityRef::local(parent_id));
    c.editor.document.insert_entity(child, &mut c.assets)?;
    c.frame(vec![egui::Event::Copy]);
    c.context_menu("粘贴父级");
    c.press("粘贴", Rect::EVERYTHING);
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let copied = c.editor.selection.active().unwrap();
    let snapshot = c.editor.snapshot(&mut c.assets)?;
    let author = snapshot.entities.iter().find(|e| e.id == copied).unwrap();
    assert_eq!(author.parent, Some(parent_id));
    assert!(snapshot.entities.iter().any(|e| e.parent == Some(copied)
        && e.links.target == Some(comet_scene::EntityRef::local(copied))));
    let before_text = c.editor.document.shared_source();
    let ctx = c.ctx.clone();
    let mut text = String::new();
    for events in [
        vec![],
        vec![egui::Event::Paste("文本内容".into())],
        shortcut(egui::Key::Paste),
    ] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
                crate::ui::show(&mut c.editor, ui, &mut c.assets, true).unwrap();
            },
        );
        output.textures_delta.clear();
    }
    assert_eq!(text, "文本内容");
    assert!(Arc::ptr_eq(
        &before_text,
        &c.editor.document.shared_source()
    ));
    ctx.memory_mut(|memory| memory.surrender_focus(memory.focused().unwrap()));
    c.editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    c.frame(shortcut(egui::Key::V));
    assert!(Arc::ptr_eq(
        &before_text,
        &c.editor.document.shared_source()
    ));
    c.editor.documents = Default::default();

    // Switching author documents within the same project retains self-contained fragments.
    let path = c
        .editor
        .create_scene_source(std::path::Path::new("clipboard.scene"))?;
    c.editor.switch_document(path, &mut c.assets)?;
    assert!(c.editor.clipboard.is_some());
    fs::remove_file(&script)?;
    c.editor.refresh_resources()?;
    c.wait_resources()?;
    assert!(
        c.editor.resource_build.failure.is_none(),
        "{:?}",
        c.editor.resource_build.failure
    );
    let before_missing_asset = c.editor.document.shared_source();
    assert!(c.editor.paste_objects(None, &mut c.assets).is_err());
    assert!(Arc::ptr_eq(
        &before_missing_asset,
        &c.editor.document.shared_source()
    ));
    assert_eq!(c.editor.document.history_len(), 0);
    assert!(c.editor.selection.is_empty());
    assert!(c.editor.clipboard.is_some());
    fs::write(&script, script_bytes)?;
    c.editor.refresh_resources()?;
    c.wait_resources()?;
    assert!(
        c.editor.resource_build.failure.is_none(),
        "{:?}",
        c.editor.resource_build.failure
    );
    c.frame(shortcut(egui::Key::V));
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    assert_eq!(c.editor.document.shared_source().base.entities.len(), 2);
    assert_eq!(c.editor.document.history_len(), 1);
    c.editor.start(&mut c.assets)?;
    let authoring = c.editor.document.shared_source();
    c.frame(vec![egui::Event::Copy, egui::Event::Paste(String::new())]);
    assert!(Arc::ptr_eq(&authoring, &c.editor.document.shared_source()));
    c.editor.stop();
    for (path, bytes) in &c.sources {
        assert_eq!(fs::read(path)?, *bytes);
    }
    Ok(())
}
