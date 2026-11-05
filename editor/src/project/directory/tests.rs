use super::*;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

fn wait(reader: &mut DirectoryReader, id: RequestId) -> Result<Arc<Listing>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        reader.poll();
        if let Some(result) = reader.listing(id) {
            return result.clone().map_err(anyhow::Error::msg);
        }
        ensure!(Instant::now() < deadline, "directory job did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn bounded_single_level_authors_stay_inside_assets_and_keep_native_path_names() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let assets = project.join("assets");
    fs::create_dir_all(assets.join(" nested "))?;
    let project = project.canonicalize()?;
    let assets = assets.canonicalize()?;
    for name in [
        "main.scene",
        " source.prefab",
        "imported.glb",
        "main.scene.meta",
    ] {
        fs::write(assets.join(name), [])?;
    }
    fs::write(assets.join(" nested /hidden.scene"), [])?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::write(temp.path().join("outside.scene"), [])?;
        symlink(
            temp.path().join("outside.scene"),
            assets.join("escape.scene"),
        )?;
        symlink(temp.path(), assets.join("escape-dir"))?;
    }
    let listing = scan(
        Scope::Authors {
            project: project.clone(),
        },
        None,
        MAX_ENTRIES,
    )?;
    assert_eq!(listing.assets.as_deref(), Some(assets.as_path()));
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.path == assets.join(" source.prefab"))
    );
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.directory && entry.name == " nested ")
    );
    assert!(
        !listing
            .entries
            .iter()
            .any(|entry| entry.name == "hidden.scene"
                || entry.name == "imported.glb"
                || entry.name == "main.scene.meta"
                || entry.name == "escape.scene"
                || entry.name == "escape-dir")
    );
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        fs::write(
            assets.join(OsString::from_vec(b" native-\xff.prefab".to_vec())),
            [],
        )?;
        let listing = scan(
            Scope::Authors {
                project: project.clone(),
            },
            None,
            MAX_ENTRIES,
        )?;
        assert!(listing.entries.iter().any(|entry| entry.name
            == OsString::from_vec(b" native-\xff.prefab".to_vec())
            && entry.path == assets.join(&entry.name)));
    }
    assert!(
        scan(
            Scope::Authors {
                project: project.clone()
            },
            Some(temp.path().to_owned()),
            MAX_ENTRIES
        )
        .is_err()
    );
    let limited = scan(Scope::Authors { project }, None, 2)?;
    assert!(limited.truncated);
    assert!(limited.entries.len() <= 2);
    assert!(scan(Scope::Directories, Some(assets.join("missing")), 2).is_err());
    Ok(())
}

#[test]
fn only_the_latest_queued_request_can_replace_the_visible_listing() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let old = temp.path().join("old");
    let latest = temp.path().join("latest");
    fs::create_dir(&old)?;
    fs::create_dir(&latest)?;
    let (release, blocked) = mpsc::channel();
    let job = Job::spawn("blocked-directory-test", move |_| {
        blocked.recv()?;
        scan(Scope::Directories, Some(old), MAX_ENTRIES)
    })?;
    let old_id = RequestId(1);
    let mut reader = DirectoryReader {
        next_id: 1,
        active: Some(old_id),
        running: Some(Running { id: old_id, job }),
        ..Default::default()
    };
    let middle = reader.request(Scope::Directories, Some(temp.path().join("missing")))?;
    let last = reader.request(Scope::Directories, Some(latest.clone()))?;
    reader.poll();
    assert!(reader.listing(middle).is_none());
    assert!(reader.listing(last).is_none());
    assert_eq!(reader.running.as_ref().unwrap().id, old_id);
    release.send(())?;
    assert_eq!(wait(&mut reader, last)?.directory, latest.canonicalize()?);
    assert!(reader.listing(old_id).is_none() && reader.listing(middle).is_none());
    Ok(())
}

#[test]
fn cancelling_a_pending_read_returns_without_joining_or_reviving_a_closed_picker() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().to_owned();
    let (release, blocked) = mpsc::channel();
    let job = Job::spawn("cancel-directory-test", move |_| {
        blocked.recv()?;
        scan(Scope::Directories, Some(path), MAX_ENTRIES)
    })?;
    let id = RequestId(1);
    let mut reader = DirectoryReader {
        next_id: 1,
        active: Some(id),
        running: Some(Running { id, job }),
        ..Default::default()
    };
    let (returned, result) = mpsc::channel();
    let cancel = std::thread::spawn(move || {
        reader.cancel(id);
        let _ = returned.send(reader);
    });
    let cancelled = result.recv_timeout(Duration::from_secs(1));
    release.send(())?;
    cancel.join().unwrap();
    let mut reader = cancelled.context("cancel joined the pending filesystem worker")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while reader.running.is_some() {
        reader.poll();
        ensure!(Instant::now() < deadline, "cancelled read did not retire");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(reader.listing(id).is_none());
    assert!(reader.active.is_none() && reader.result.is_none());
    Ok(())
}
