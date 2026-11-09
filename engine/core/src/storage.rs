use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use uuid::Uuid;

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn validate_digest(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid SHA-256 digest"
    );
    Ok(())
}

/// Read a newly opened regular file under a caller-owned byte budget.
/// The open handle keeps metadata and content on the same file; path policy belongs to callers.
pub fn read_bounded(file: File, limit: usize) -> Result<Vec<u8>> {
    let metadata = file.metadata().context("inspect bounded file")?;
    ensure!(metadata.is_file(), "bounded input is not a regular file");
    ensure!(
        metadata.len() <= limit as u64,
        "file exceeds {limit}-byte budget"
    );
    read_contents(file, limit)
}

fn read_contents(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let sentinel = limit.checked_add(1).context("file byte budget overflow")?;
    let mut bytes = Vec::new();
    reader.take(sentinel as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "file grew beyond {limit}-byte budget");
    Ok(bytes)
}

/// Publish a complete file by same-directory rename; failure leaves the old file intact.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temporary = parent.join(format!(".comet-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("publish {}", path.display()))
}

/// Publish a complete new file without replacing a destination created by another writer.
pub fn atomic_create(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("new file requires a parent directory")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".comet-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        crate::publication::rename_exclusive(&temporary, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result.with_context(|| format!("create new file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    #[test]
    fn growth_after_metadata_cannot_read_past_the_budget_sentinel() {
        let mut input = Cursor::new([7; 64]);
        assert!(read_contents(&mut input, 4).is_err());
        assert_eq!(input.position(), 5);
        assert!(read_contents(Cursor::new([]), 0).unwrap().is_empty());
    }

    #[test]
    fn read_errors_and_overflow_limits_never_return_partial_bytes() {
        struct Failed;
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected read failure"))
            }
        }
        let error = read_contents(Cursor::new(b"prefix").chain(Failed), 16).unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::Other
        );
        let mut input = Cursor::new(b"never consumed");
        assert!(read_contents(&mut input, usize::MAX).is_err());
        assert_eq!(input.position(), 0);
    }
}
