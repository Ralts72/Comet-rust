use super::*;

#[test]
fn a_destination_created_after_validation_wins_and_the_private_candidate_is_removed() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let destination = temp.path().join("project");
    let candidate = prepared_project(&destination)?;
    let private = candidate.path().to_owned();
    assert!(private.join("assets").join(DEFAULT_SCENE).is_file());
    assert!(!private.join(".assetc.lock").exists());
    assert!(!private.join("products").exists());
    assert!(!destination.exists());
    assert_eq!(
        fs::read_dir(temp.path())?.count(),
        1,
        "validation products must be cleaned before publication"
    );
    fs::create_dir(&destination)?;
    fs::write(destination.join("keep.txt"), "another project")?;
    assert!(candidate.publish().is_err());
    assert!(!private.exists());
    assert_eq!(fs::read(destination.join("keep.txt"))?, b"another project");
    assert_eq!(fs::read_dir(temp.path())?.count(), 1);
    Ok(())
}
