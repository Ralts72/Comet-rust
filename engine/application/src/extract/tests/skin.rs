use super::*;
use comet_asset_data::{SkeletonData, SkeletonNode, SkinData, SkinJoint};
use comet_core::math::{Mat4, Quat};
use comet_scene::{Animator, EntityRef, SkinBinding, Transform, UnboundRig};

struct Rig {
    snapshot: Arc<SceneSnapshot>,
    actor: EntityId,
    bone: EntityId,
    model: EntityId,
}

fn rig(products: &mut Products) -> Result<Rig> {
    let skeleton = AssetId::new();
    let skin = AssetId::new();
    let source_root = EntityId::new();
    let source_child = EntityId::new();
    let mut actor = EntityData::new("rig instance");
    actor.transform = Transform {
        translation: [5., 1., 0.],
        rotation: Quat::from_rotation_z(0.4).to_array(),
        scale: [-2., 3., 1.],
    };
    let mut root = EntityData::new("root joint");
    root.parent = Some(actor.id);
    root.transform.translation = [0., 1., 0.];
    let mut child = EntityData::new("child joint");
    child.parent = Some(root.id);
    child.transform.translation = [1., 0., 0.];
    let mut animator = Animator::new(skeleton);
    animator
        .bindings
        .insert(source_root, Some(EntityRef::local(root.id)));
    animator
        .bindings
        .insert(source_child, Some(EntityRef::local(child.id)));
    actor.animator = Some(animator);
    let mut model = products.model("mesh node transform is ignored", Some(actor.id));
    model.transform.translation = [1000.; 3];
    model.transform.scale = [0.; 3];
    model.skin = Some(SkinBinding {
        skin,
        animator: Some(EntityRef::local(actor.id)),
    });
    products.write(
        skeleton,
        Product::Skeleton(SkeletonData {
            version: 1,
            nodes: vec![
                SkeletonNode {
                    id: source_root,
                    name: "root".into(),
                    parent: None,
                    rest: root.transform.clone(),
                },
                SkeletonNode {
                    id: source_child,
                    name: "child".into(),
                    parent: Some(source_root),
                    rest: child.transform.clone(),
                },
            ],
        }),
    )?;
    products.write(
        skin,
        Product::Skin(SkinData {
            version: 1,
            skeleton,
            joints: vec![
                SkinJoint {
                    node: source_root,
                    inverse_bind: Mat4::from_translation(Vec3::new(0., -1., 0.)).to_cols_array_2d(),
                },
                SkinJoint {
                    node: source_child,
                    inverse_bind: Mat4::from_translation(Vec3::new(-1., -1., 0.))
                        .to_cols_array_2d(),
                },
            ],
        }),
    )?;
    let mut store = products.store()?;
    let mesh = store.load(products.mesh)?;
    let Product::Mesh(mut mesh) = mesh.payload.clone() else {
        unreachable!()
    };
    mesh.version = 2;
    mesh.joints = Some(vec![[0, 1, 0, 0]; mesh.positions.len()]);
    mesh.weights = Some(vec![[0.25, 0.75, 0., 0.]; mesh.positions.len()]);
    products.write(products.mesh, Product::Mesh(mesh))?;
    products.publish()?;
    let result = Rig {
        snapshot: snapshot(vec![actor.clone(), root, child.clone(), model.clone()]),
        actor: actor.id,
        bone: child.id,
        model: model.id,
    };
    Ok(result)
}

fn near(left: Mat4, right: Mat4) {
    assert!(
        left.to_cols_array()
            .iter()
            .zip(right.to_cols_array())
            .all(|(a, b)| (*a - b).abs() < 1e-5)
    );
}

#[test]
fn selection_bounds_follow_descendants_and_the_current_skin_pose_without_loading_unselected_products()
-> Result<()> {
    let mut products = Products::new()?;
    let rig = rig(&mut products)?;
    let mut store = products.store()?;
    let mut scene = rig.snapshot.as_ref().clone();
    let mut unrelated = products.model("unavailable unrelated model", None);
    unrelated.model.as_mut().unwrap().mesh = AssetId::new();
    let mut empty = EntityData::new("empty pivot");
    empty.transform.translation = [20., 30., 40.];
    let empty_id = empty.id;
    scene.entities.extend([unrelated, empty]);
    let actor = &scene
        .entities
        .iter()
        .find(|e| e.id == rig.actor)
        .unwrap()
        .transform;
    let matrix = Mat4::from_scale_rotation_translation(
        Vec3::from_array(actor.scale),
        Quat::from_array(actor.rotation),
        Vec3::from_array(actor.translation),
    );
    for moved in [false, true] {
        if moved {
            scene
                .entities
                .iter_mut()
                .find(|e| e.id == rig.bone)
                .unwrap()
                .transform
                .translation[0] = 2.;
        }
        let bounds =
            super::super::selection_bounds(&scene, &mut store, &[rig.actor, rig.model])?.unwrap();
        let shift = if moved { 0.75 } else { 0. };
        let points = [[-0.5, 0., 0.], [0.5, 0., 0.], [0., 1., 0.]]
            .map(|p| matrix.transform_point3(Vec3::from_array(p) + Vec3::X * shift));
        let min = points
            .into_iter()
            .fold(Vec3::splat(f32::INFINITY), Vec3::min);
        let max = points
            .into_iter()
            .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
        assert!(bounds[0].distance(min) < 1e-5 && bounds[1].distance(max) < 1e-5);
    }
    assert_eq!(
        super::super::selection_bounds(&scene, &mut store, &[empty_id])?,
        Some([Vec3::new(20., 30., 40.); 2])
    );
    assert!(super::super::selection_bounds(&scene, &mut store, &[])?.is_none());
    assert!(extract(&scene, &mut store, &[]).is_err());
    scene
        .entities
        .iter_mut()
        .find(|e| e.id == rig.actor)
        .unwrap()
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .clear();
    assert!(super::super::selection_bounds(&scene, &mut store, &[rig.actor]).is_err());
    Ok(())
}

#[test]
fn world_palettes_include_the_actor_once_and_reuse_idle_and_selection_batches() -> Result<()> {
    let mut products = Products::new()?;
    let rig = rig(&mut products)?;
    let mut store = products.store()?;
    let mut extractor = SceneExtractor::default();
    let first = extractor.extract(&rig.snapshot, &mut store, &[])?;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].entity, rig.model);
    assert_eq!(first[0].transform.model(), Mat4::IDENTITY);
    let actor = &rig
        .snapshot
        .entities
        .iter()
        .find(|e| e.id == rig.actor)
        .unwrap()
        .transform;
    let actor = Mat4::from_scale_rotation_translation(
        Vec3::from_array(actor.scale),
        Quat::from_array(actor.rotation),
        Vec3::from_array(actor.translation),
    );
    let palette = first[0].skin.as_ref().unwrap();
    for matrix in palette.matrices() {
        near(*matrix, actor);
    }
    for id in products.index.products.keys() {
        store.evict(*id);
    }
    assert!(Arc::ptr_eq(
        &first,
        &extractor.extract(&rig.snapshot, &mut store, &[])?
    ));
    let highlighted = extractor.extract(&rig.snapshot, &mut store, &[rig.actor])?;
    assert!(Arc::ptr_eq(palette, highlighted[0].skin.as_ref().unwrap()));
    assert!(highlighted[0].selected && !first[0].selected);
    assert_eq!(store.resident_count(), 0);
    let mut moved = rig.snapshot.as_ref().clone();
    moved
        .entities
        .iter_mut()
        .find(|e| e.id == rig.bone)
        .unwrap()
        .transform
        .rotation = Quat::from_rotation_z(1.).to_array();
    let changed = extractor.extract(&Arc::new(moved), &mut store, &[])?;
    assert_ne!(
        changed[0].skin.as_ref().unwrap().matrices()[1],
        palette.matrices()[1]
    );
    near(palette.matrices()[1], actor);
    Ok(())
}

#[test]
fn missing_bindings_defer_only_the_typed_error_and_keep_the_previous_batch() -> Result<()> {
    let mut products = Products::new()?;
    let rig = rig(&mut products)?;
    let mut store = products.store()?;
    let mut extractor = SceneExtractor::default();
    let accepted = extractor.extract(&rig.snapshot, &mut store, &[])?;
    let mut broken = rig.snapshot.as_ref().clone();
    broken
        .entities
        .iter_mut()
        .find(|e| e.id == rig.actor)
        .unwrap()
        .animator
        .as_mut()
        .unwrap()
        .bindings
        .values_mut()
        .for_each(|v| *v = None);
    let error = extract(&broken, &mut store, &[])
        .err()
        .expect("unbound skin");
    assert_eq!(
        error.downcast_ref::<UnboundRig>().unwrap().entities,
        vec![rig.model]
    );
    assert!(
        extractor
            .extract(&Arc::new(broken.clone()), &mut store, &[])
            .is_err()
    );
    assert!(Arc::ptr_eq(
        &accepted,
        &extractor.extract(&rig.snapshot, &mut store, &[])?
    ));
    let mut invalid = products.model("later missing product", None);
    invalid.model.as_mut().unwrap().mesh = AssetId::new();
    broken.entities.push(invalid);
    let error = extract(&broken, &mut store, &[])
        .err()
        .expect("later asset error must not be hidden");
    assert!(error.downcast_ref::<UnboundRig>().is_none());
    broken.entities.pop();
    let model = broken
        .entities
        .iter_mut()
        .find(|e| e.id == rig.model)
        .unwrap();
    model.skin.as_mut().unwrap().animator = Some(EntityRef::External {
        scene: AssetId::new(),
        entity: rig.actor,
    });
    assert!(
        extract(&broken, &mut store, &[])
            .err()
            .unwrap()
            .downcast_ref::<UnboundRig>()
            .is_none()
    );
    Ok(())
}
