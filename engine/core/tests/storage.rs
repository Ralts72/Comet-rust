use anyhow::Result;
use comet_core::atomic_create;
use std::fs;

#[test]
fn exclusive_files_publish_complete_bytes_without_replacing_an_existing_destination() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("场景.scene");
    atomic_create(&path, b"complete scene")?;
    assert!(atomic_create(&path, b"replacement").is_err());
    assert_eq!(fs::read(&path)?, b"complete scene");
    let directory = temp.path().join("directory");
    fs::create_dir(&directory)?;
    assert!(atomic_create(&directory, b"file").is_err());
    assert!(directory.is_dir());
    assert!(fs::read_dir(temp.path())?.all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".comet-")
    }));
    Ok(())
}
