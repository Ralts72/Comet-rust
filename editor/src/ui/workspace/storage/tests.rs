use super::*;

#[test]
fn layout_roundtrip_checks_topology_and_bad_candidates_preserve_the_current_layout_and_file()
-> Result<()> {
    let mut tree = default_tree();
    let scene = tree
        .tiles
        .iter()
        .find_map(|(&id, tile)| matches!(tile, Tile::Pane(Pane::Scene)).then_some(id))
        .unwrap();
    tree.set_visible(scene, false);
    let root = tree.root.unwrap();
    let Some(Tile::Container(Container::Linear(row))) = tree.tiles.get_mut(root) else {
        unreachable!()
    };
    row.shares[row.children[0]] = 350.;
    if let Some(path) = std::env::var_os("COMET_TEST_EDITOR_LAYOUT_CHILD") {
        assert_eq!(read(std::path::Path::new(&path))?.unwrap(), tree);
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("workspace.json");
    fs::write(&path, encode(&tree)?)?;
    let child = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", "ui::workspace::storage::tests::layout_roundtrip_checks_topology_and_bad_candidates_preserve_the_current_layout_and_file"])
        .env("COMET_TEST_EDITOR_LAYOUT_CHILD", &path).output()?;
    assert!(
        child.status.success() && String::from_utf8_lossy(&child.stdout).contains("1 passed"),
        "{child:?}"
    );
    assert_eq!(read(&path)?.unwrap(), tree);
    let accepted = tree.clone();
    let bad = br#"{"version":99,"tree":{}}"#;
    fs::write(&path, bad)?;
    let mut storage = Storage::default();
    assert!(storage.load(path.clone(), &mut tree).is_err());
    storage.save(&default_tree());
    assert_eq!(tree, accepted);
    assert_eq!(fs::read(&path)?, bad);
    storage.allow_save();
    storage.save(&default_tree());
    assert!(storage.warning.is_none());
    assert_eq!(read(&path)?.unwrap(), default_tree());
    for variant in 0..5 {
        let mut tree = default_tree();
        let root = tree.root.unwrap();
        match variant {
            0 => {
                if let Some(Tile::Container(Container::Linear(row))) = tree.tiles.get_mut(root) {
                    row.children.push(root);
                }
            }
            1 => {
                let _ = tree.tiles.insert_pane(Pane::Scene);
            }
            2 => {
                tree.tiles.remove(scene);
            }
            3 => {
                tree.root = Some(TileId(u64::MAX));
            }
            _ => {
                if let Some(Tile::Container(Container::Linear(row))) = tree.tiles.get_mut(root) {
                    row.shares[row.children[0]] = f32::NAN;
                }
            }
        }
        assert!(encode(&tree).is_err(), "invalid topology {variant}");
    }
    fs::File::create(&path)?.set_len(MAX_BYTES as u64 + 1)?;
    assert!(read(&path).is_err());
    Ok(())
}
