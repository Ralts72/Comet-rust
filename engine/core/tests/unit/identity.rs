use super::*;
#[test]
fn stable_instance_identity_and_nil_rejection() {
    let entity = EntityId::new();
    let instance = InstanceId::new();
    assert_eq!(entity.in_instance(instance), entity.in_instance(instance));
    assert_ne!(
        entity.in_instance(instance),
        entity.in_instance(InstanceId::new())
    );
    assert!(
        "00000000-0000-0000-0000-000000000000"
            .parse::<AssetId>()
            .is_err()
    );
}
