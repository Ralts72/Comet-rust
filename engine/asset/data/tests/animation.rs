use anyhow::Result;
use comet_asset_data::{
    AnimationChannel, AnimationClipData, AnimationInterpolation, AnimationValues, SkeletonData,
    SkeletonNode, SkinData, SkinJoint, product::MeshData,
};
use comet_core::{AssetId, EntityId, math::Mat4};
use comet_scene::Transform;

fn skeleton() -> SkeletonData {
    let root = EntityId::new();
    SkeletonData {
        version: 1,
        nodes: vec![
            SkeletonNode {
                id: root,
                name: "root".into(),
                parent: None,
                rest: Transform::default(),
            },
            SkeletonNode {
                id: EntityId::new(),
                name: "tip".into(),
                parent: Some(root),
                rest: Transform::default(),
            },
        ],
    }
}
fn mesh() -> MeshData {
    MeshData {
        version: 2,
        positions: vec![[0.; 3]; 3],
        normals: vec![[0., 1., 0.]; 3],
        uvs: vec![[0.; 2]; 3],
        indices: vec![0, 1, 2],
        joints: Some(vec![[0, 1, 0, 0]; 3]),
        weights: Some(vec![[0.5, 0.5, 0., 0.]; 3]),
    }
}

#[test]
fn skin_local_order_and_finite_affine_inverse_bind_matrices_are_checked_against_the_source_rig()
-> Result<()> {
    let skeleton = skeleton();
    let skin = SkinData {
        version: 1,
        skeleton: AssetId::new(),
        joints: skeleton
            .nodes
            .iter()
            .rev()
            .map(|node| SkinJoint {
                node: node.id,
                inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
            })
            .collect(),
    };
    skin.validate_for(&skeleton)?;
    skin.validate_mesh(&mesh())?;
    assert_eq!(skin.joints[0].node, skeleton.nodes[1].id);
    let mut bad = skin.clone();
    bad.joints[1].node = bad.joints[0].node;
    assert!(bad.validate().is_err());
    let mut bad = skin.clone();
    bad.joints[0].node = EntityId::new();
    assert!(bad.validate_for(&skeleton).is_err());
    let mut bad = skin.clone();
    bad.joints[0].inverse_bind[0][3] = 1.;
    assert!(bad.validate().is_err());
    let mut bad = mesh();
    bad.joints.as_mut().unwrap()[0][3] = 2;
    assert!(
        skin.validate_mesh(&bad).is_err(),
        "zero-weight slots still require valid skin-local indices"
    );
    let mut bad = skeleton.clone();
    bad.nodes[0].parent = Some(bad.nodes[1].id);
    assert!(bad.validate().is_err());
    let mut bad = skeleton;
    bad.nodes[0].name.push('\0');
    assert!(bad.validate().is_err());
    Ok(())
}

#[test]
fn clip_channels_keep_absolute_times_and_reject_duplicate_targets_and_invalid_quaternions()
-> Result<()> {
    let skeleton = skeleton();
    let clip = AnimationClipData {
        version: 1,
        skeleton: AssetId::new(),
        duration: 4.,
        channels: vec![AnimationChannel {
            node: skeleton.nodes[1].id,
            interpolation: AnimationInterpolation::Linear,
            times: vec![2., 4.],
            values: AnimationValues::Rotation(vec![[0., 0., 0., 1.], [0., 0., 1., 0.]]),
        }],
    };
    clip.validate_for(&skeleton)?;
    assert_eq!(
        serde_json::from_slice::<AnimationClipData>(&serde_json::to_vec(&clip)?)?,
        clip
    );
    let mut bad = clip.clone();
    bad.channels.push(bad.channels[0].clone());
    assert!(bad.validate().is_err());
    let mut bad = clip.clone();
    bad.channels[0].times[1] = 2.;
    assert!(bad.validate().is_err());
    let mut bad = clip.clone();
    bad.channels[0].values = AnimationValues::Rotation(vec![[0.; 4]; 2]);
    assert!(bad.validate().is_err());
    let mut bad = clip.clone();
    bad.channels[0].node = EntityId::new();
    assert!(bad.validate_for(&skeleton).is_err());
    let mut constant = clip;
    constant.duration = 0.;
    constant.channels[0].times = vec![0.];
    constant.channels[0].values = AnimationValues::Scale(vec![[1.; 3]]);
    constant.validate()?;
    Ok(())
}

#[test]
fn static_meshes_omit_skin_fields_and_skin_attributes_require_an_explicit_version_and_normalized_weights()
-> Result<()> {
    let mut value = mesh();
    value.validate()?;
    value.version = 1;
    assert!(value.validate().is_err());
    value.joints = None;
    value.weights = None;
    let mut json = serde_json::to_value(&value)?;
    assert!(json.get("joints").is_none() && json.get("weights").is_none());
    json["joints"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<MeshData>(json).is_err());
    let mut bad = mesh();
    bad.weights.as_mut().unwrap()[0] = [0.; 4];
    assert!(bad.validate().is_err());
    let mut bad = mesh();
    bad.weights = None;
    assert!(bad.validate().is_err());
    let mut bad = mesh();
    bad.joints.as_mut().unwrap()[0] = [0; 4];
    assert!(bad.validate().is_err());
    Ok(())
}
