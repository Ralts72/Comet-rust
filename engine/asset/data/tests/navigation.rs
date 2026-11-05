use anyhow::Result;
use comet_asset_data::navigation::{NAVIGATION_VERSION, NavigationData, NavigationInput};
use comet_core::{AssetId, EntityId, digest};
use comet_scene::{
    BodyMotion, Character, ColliderShape, EntityData, EntityRef, Model, NavigationAgent,
    NavigationRegion, PhysicsBody, SceneSnapshot, Surface,
};

fn scene() -> SceneSnapshot {
    let mut region = EntityData::new("导航区域");
    region.navigation_region = Some(NavigationRegion {
        half_extents: [6., 3., 6.],
        ..Default::default()
    });
    let mut floor = EntityData::new("碰撞地板");
    floor.transform.translation[1] = -0.25;
    floor.physics = Some(PhysicsBody {
        shape: ColliderShape::Box {
            half_extents: [6., 0.25, 6.],
        },
        ..Default::default()
    });
    SceneSnapshot {
        entities: vec![region, floor],
        ..Default::default()
    }
}

#[test]
fn input_digest_tracks_collision_geometry_and_profile_without_authoring_identity_or_gameplay()
-> Result<()> {
    let mut scene = scene();
    let input = NavigationInput::capture(&scene)?;
    let before = input.digest().to_owned();
    let target = EntityData::new("目标");
    let mut npc = EntityData::new("NPC");
    npc.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            radius: 0.3,
            half_height: 0.5,
        },
        ..Default::default()
    });
    npc.character = Some(Character {
        player_controlled: false,
        ..Default::default()
    });
    npc.navigation_agent = Some(NavigationAgent {
        target: Some(EntityRef::local(target.id)),
        ..Default::default()
    });
    scene.entities.extend([npc, target]);
    scene.entities[0].id = EntityId::new();
    scene.entities[0].navigation_region.as_mut().unwrap().data = Some(AssetId::new());
    scene.entities[1].id = EntityId::new();
    scene.entities[1].name = "只改可视内容".into();
    scene.entities[1].transform.translation[0] = -0.;
    scene.entities[1].transform.rotation[3] = -1.;
    scene.entities[1].model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.1,
        },
    });
    scene.entities.reverse();
    assert_eq!(NavigationInput::capture(&scene)?.digest(), before);
    let floor = scene
        .entities
        .iter()
        .position(|entity| entity.name == "只改可视内容")
        .unwrap();
    scene.entities[floor].transform.translation[0] = 0.25;
    assert_ne!(NavigationInput::capture(&scene)?.digest(), before);
    scene.entities[floor].transform.translation[0] = 0.;
    scene
        .entities
        .iter_mut()
        .find_map(|entity| entity.navigation_region.as_mut())
        .unwrap()
        .max_climb = 0.4;
    assert_ne!(NavigationInput::capture(&scene)?.digest(), before);
    Ok(())
}

#[test]
fn capture_clips_boxes_and_rejects_only_relevant_unsupported_colliders_and_excessive_grids()
-> Result<()> {
    let mut scene = scene();
    scene.entities[1].physics.as_mut().unwrap().shape = ColliderShape::Box {
        half_extents: [100., 0.25, 100.],
    };
    let input = NavigationInput::capture(&scene)?;
    let bounds = input.bounds();
    assert!(
        input
            .triangles()
            .iter()
            .flatten()
            .all(|vertex| (0..3).all(|i| vertex[i] >= bounds[0][i] && vertex[i] <= bounds[1][i]))
    );
    let mut sphere = EntityData::new("不支持的静态球体");
    sphere.transform.translation[0] = 100.;
    sphere.physics = Some(PhysicsBody {
        shape: ColliderShape::Sphere { radius: 0.5 },
        ..Default::default()
    });
    scene.entities.push(sphere);
    NavigationInput::capture(&scene)?;
    scene.entities.last_mut().unwrap().transform.translation[0] = 0.;
    assert!(NavigationInput::capture(&scene).is_err());
    scene
        .entities
        .last_mut()
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .sensor = true;
    NavigationInput::capture(&scene)?;
    scene
        .entities
        .last_mut()
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .sensor = false;
    scene
        .entities
        .last_mut()
        .unwrap()
        .physics
        .as_mut()
        .unwrap()
        .motion = BodyMotion::Dynamic;
    NavigationInput::capture(&scene)?;
    scene.entities.pop();
    scene.entities[0]
        .navigation_region
        .as_mut()
        .unwrap()
        .half_extents = [100_000., 3., 100_000.];
    assert!(NavigationInput::capture(&scene).is_err());
    scene.entities[0]
        .navigation_region
        .as_mut()
        .unwrap()
        .half_extents = [6., 3., 6.];
    scene.entities[0].transform.translation[0] = 1e20;
    assert!(NavigationInput::capture(&scene).is_err());
    scene.entities[0].transform.translation[0] = 0.;
    scene.entities[1].physics.as_mut().unwrap().shape = ColliderShape::Box {
        half_extents: [1e30, 0.25, 1e30],
    };
    assert!(NavigationInput::capture(&scene).is_err());
    Ok(())
}

fn data(input_digest: String) -> NavigationData {
    NavigationData {
        version: NAVIGATION_VERSION,
        input_digest,
        vertices: vec![[0., 0., 0.], [2., 0., 0.], [2., 0., 2.], [0., 0., 2.]],
        polygons: vec![vec![0, 1, 2, 3]],
    }
}

#[test]
fn topology_rejects_bad_winding_indices_connectivity_and_nonfinite_or_empty_products() -> Result<()>
{
    let valid = data(digest(b"navigation fixture"));
    valid.validate()?;
    for failure in 0..8 {
        let mut invalid = valid.clone();
        match failure {
            0 => invalid.polygons[0].reverse(),
            1 => invalid.polygons[0][1] = 99,
            2 => invalid.polygons[0][1] = 0,
            3 => invalid.vertices[1][0] = f32::NAN,
            4 => invalid.vertices.clear(),
            5 => invalid.polygons.push(invalid.polygons[0].clone()),
            6 => invalid.vertices[2] = [0.5, 0., 0.5],
            _ => invalid.version += 1,
        }
        assert!(
            invalid.validate().is_err(),
            "accepted invalid topology case {failure}"
        );
    }
    Ok(())
}

#[test]
fn freshness_rejects_geometry_profile_changes_and_meshes_outside_the_captured_region() -> Result<()>
{
    let mut scene = scene();
    let data = data(NavigationInput::capture(&scene)?.digest().into());
    data.validate_for(&scene)?;
    scene.entities[1].transform.translation[0] += 0.5;
    assert!(data.validate_for(&scene).is_err());
    scene.entities[1].transform.translation[0] -= 0.5;
    scene.entities[0]
        .navigation_region
        .as_mut()
        .unwrap()
        .cell_size = 0.25;
    assert!(data.validate_for(&scene).is_err());
    scene.entities[0]
        .navigation_region
        .as_mut()
        .unwrap()
        .cell_size = 0.2;
    let mut outside = data.clone();
    for vertex in &mut outside.vertices {
        vertex[0] += 100.;
    }
    outside.validate()?;
    assert!(outside.validate_for(&scene).is_err());
    Ok(())
}
