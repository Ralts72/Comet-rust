#[test]
fn shaders_and_layouts_compile_for_both_backends() {
    for ui in [false, true] {
        let compiled = super::compile(ui).unwrap();
        assert!(!compiled.msl.is_empty());
        assert!(
            compiled
                .spirv
                .values()
                .all(|s| s.first() == Some(&0x07230203))
        );
    }
}
