use anyhow::Result;
use comet_core::{atomic_create, digest, file_digest, read_bounded};
use std::{
    fs::{self, File},
    io::Seek,
};

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

#[test]
fn bounded_files_reject_large_metadata_before_reading_and_preserve_exact_limits() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("有界输入.bin");
    fs::write(&path, b"complete")?;
    let file = File::open(&path)?;
    let mut observer = file.try_clone()?;
    assert!(read_bounded(file, 7).is_err());
    assert_eq!(
        observer.stream_position()?,
        0,
        "oversized metadata must reject before reading"
    );
    assert_eq!(read_bounded(File::open(&path)?, 8)?, b"complete");
    assert_eq!(fs::read(&path)?, b"complete");
    assert!(read_bounded(File::open(&path)?, 0).is_err());
    drop(observer);
    fs::write(&path, b"")?;
    assert!(read_bounded(File::open(&path)?, 0)?.is_empty());
    #[cfg(unix)]
    assert!(read_bounded(File::open(temp.path())?, 1024).is_err());
    Ok(())
}

#[test]
fn file_digests_preserve_the_byte_protocol_and_source_contents() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("文件摘要.bin");
    let large: Vec<_> = (0..131_113).map(|index| (index % 251) as u8).collect();
    for bytes in [b"".as_slice(), b"abc".as_slice(), large.as_slice()] {
        fs::write(&path, bytes)?;
        assert_eq!(file_digest(&path)?, digest(bytes));
        assert_eq!(fs::read(&path)?, bytes);
    }
    fs::write(&path, b"abc")?;
    assert_eq!(
        file_digest(&path)?,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    fs::remove_file(&path)?;
    assert_eq!(
        file_digest(&path)
            .unwrap_err()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(file_digest(temp.path()).is_err());
    Ok(())
}
