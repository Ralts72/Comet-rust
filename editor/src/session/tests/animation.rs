use super::*;
use comet_asset_pipeline::SourceMeta;
use comet_scene::{EntityRef, PrefabResolver};
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
fn unbound_native_rigs_can_save_reopen_refresh_and_repair_but_never_start_incomplete() -> Result<()>
{
    let (_temp, mut editor, mut assets) = fixture()?;
    let prefab = crate::test_support::prepare_skin_asset(&editor)?;
    assets.refresh()?;
    let source = assets.prefab(prefab)?;
    let controller = source
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .unwrap();
    let owner = controller.id;
    let original_animator = controller.animator.clone().unwrap();
    let native = editor.create_prefab_source(Path::new("animated.prefab"))?;
    let metadata = fs::read(SourceMeta::path(&native))?;
    let native_id = SourceMeta::load(&native)?.id;
    editor.document = Document::open_prefab(native_id, source, &mut assets)?;
    editor.source = native.clone();
    editor.last_snapshot = editor.shared_snapshot(&mut assets)?;
    editor.edit_animation(owner, None, None, &mut assets)?;
    editor.commit()?;
    editor.save(&mut assets)?;
    let saved = editor.document.shared_source();
    let bytes = fs::read(&native)?;
    assert_eq!(fs::read(SourceMeta::path(&native))?, metadata);
    let error = editor
        .start(&mut assets)
        .expect_err("unbound skin cannot start");
    assert!(error.is::<comet_scene::UnboundRig>());
    assert!(editor.play.is_none() && !editor.document.is_dirty());

    // A permitted missing binding cannot conceal an unrelated malformed model during saving.
    let mut bad = EntityData::new("bad unrelated model");
    bad.model = Some(comet_scene::Model {
        mesh: AssetId::new(),
        surface: comet_scene::Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    editor.document.insert_entity(bad, &mut assets)?;
    assert!(editor.save(&mut assets).is_err());
    assert_eq!(fs::read(&native)?, bytes);
    editor.document.undo()?;
    assert!(Arc::ptr_eq(&saved, &editor.document.shared_source()));

    let root = editor.project_root().to_owned();
    let products = editor.products.clone();
    let (mut reopened, mut rebuilt) = open(root, products, "animated.prefab".into())?;
    assert_eq!(reopened.document.shared_source(), saved);
    assert_eq!(reopened.document.prefab_asset(), Some(native_id));
    reopened.refresh_resources()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while reopened.resource_build.is_running() {
        reopened.poll_resources(&mut rebuilt);
        ensure!(
            Instant::now() < deadline,
            "unbound rig resource refresh timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(reopened.resource_build.failure.is_none());
    assert_eq!(reopened.document.shared_source(), saved);
    reopened.edit_animation(owner, Some(original_animator), None, &mut rebuilt)?;
    reopened.commit()?;
    let meshes: Vec<_> = reopened
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .filter_map(|entity| entity.skin.clone().map(|skin| (entity.id, skin)))
        .collect();
    for (id, mut skin) in meshes {
        skin.animator = Some(EntityRef::Local { entity: owner });
        reopened.edit_animation(id, None, Some(skin), &mut rebuilt)?;
        reopened.commit()?;
    }
    reopened.save(&mut rebuilt)?;
    let repaired = reopened.document.shared_source();
    reopened.start(&mut rebuilt)?;
    reopened.play.as_mut().unwrap().advance(1. / 60.)?;
    assert!(
        reopened
            .play
            .as_ref()
            .unwrap()
            .animation_time(owner)
            .unwrap()
            > 0.
    );
    reopened.stop();
    assert_eq!(reopened.document.shared_source(), repaired);
    assert!(!reopened.document.is_dirty());
    assert_eq!(fs::read(SourceMeta::path(&native))?, metadata);
    Ok(())
}
