//! Where the desktop app keeps recovery copies of unsaved changes: one file per running window.
//!
//! Each session writes `<session>.json` in the recovery folder and holds an exclusive lock on
//! `<session>.lock` for as long as it runs. The operating system drops the lock when the
//! process ends, however it ends, so a copy whose lock can be taken belongs to a session that is
//! gone and is offered for recovery; a copy whose lock is held belongs to a window still open,
//! and is left alone. A session removes only its own copy, so two windows never delete each
//! other's.
//!
//! Older versions kept one shared file; it is offered like the copy of a session that is gone.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The slot name of the single file older versions kept.
const LEGACY: &str = "legacy";

/// A new session's name: unique among the sessions running and the ones that ended.
pub fn new_session() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos:x}", std::process::id())
}

pub struct Store {
    dir: PathBuf,
    legacy: Option<PathBuf>,
    session: String,
    /// Held while the session runs; `None` if the lock could not be taken (then another
    /// session may offer this one's copy, which is the safe way to fail).
    _lock: Option<File>,
}

impl Store {
    /// The store in `dir` for the session `session`, taking the session's lock.
    pub fn open(dir: PathBuf, legacy: Option<PathBuf>, session: String) -> Self {
        let lock = fs::create_dir_all(&dir).ok().and_then(|()| {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(dir.join(format!("{session}.lock")))
                .ok()?;
            file.try_lock().ok()?;
            Some(file)
        });
        Self {
            dir,
            legacy,
            session,
            _lock: lock,
        }
    }

    fn copy_path(&self, session: &str) -> PathBuf {
        self.dir.join(format!("{session}.json"))
    }

    /// Writes this session's copy (atomically: a crash while writing never leaves a broken one).
    pub fn write(&self, copy: &str) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        craft_atomic::atomic_write(&self.copy_path(&self.session), copy.as_bytes())
    }

    /// Removes this session's copy, if it has one.
    pub fn clear(&self) -> io::Result<()> {
        not_found_is_fine(fs::remove_file(self.copy_path(&self.session)))
    }

    /// Whether the session `session` is still running: its lock is held.
    fn running(&self, session: &str) -> bool {
        let Ok(file) = OpenOptions::new()
            .write(true)
            .open(self.dir.join(format!("{session}.lock")))
        else {
            // No lock file: the session ended and its lock file was cleaned up.
            return false;
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            // Held, or not to be told: leave the copy alone.
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => true,
        }
    }

    /// The copies left by sessions that are gone (and the older versions' file): each is its
    /// slot's name and the copy, the most recently written first.
    pub fn orphans(&self) -> Vec<(String, String)> {
        let mut found: Vec<(SystemTime, String, String)> = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(slot) = path.file_stem().and_then(OsStr::to_str) else {
                    continue;
                };
                if slot == self.session {
                    continue;
                }
                match path.extension().and_then(OsStr::to_str) {
                    Some("json") if !self.running(slot) => {
                        let Ok(copy) = fs::read_to_string(&path) else {
                            continue;
                        };
                        let when = entry
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(UNIX_EPOCH);
                        found.push((when, slot.to_owned(), copy));
                    }
                    // A lock file whose copy is gone: nothing keeps it.
                    Some("lock")
                        if !path.with_extension("json").exists() && !self.running(slot) =>
                    {
                        let _ = fs::remove_file(&path);
                    }
                    _ => {}
                }
            }
        }
        if let Some(path) = &self.legacy
            && let Ok(copy) = fs::read_to_string(path)
        {
            let when = fs::metadata(path)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            found.push((when, LEGACY.to_owned(), copy));
        }
        found.sort_by_key(|(when, _, _)| std::cmp::Reverse(*when));
        found
            .into_iter()
            .map(|(_, slot, copy)| (slot, copy))
            .collect()
    }

    /// Removes the copy in `slot` (as [`Store::orphans`] named it).
    pub fn remove(&self, slot: &str) -> io::Result<()> {
        if slot == LEGACY {
            return self
                .legacy
                .as_deref()
                .map_or(Ok(()), |p| not_found_is_fine(fs::remove_file(p)));
        }
        // Only a name this store made: never a path.
        let plain = !slot.is_empty()
            && slot
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !plain || slot == self.session {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("not a recovery copy: {slot}"),
            ));
        }
        not_found_is_fine(fs::remove_file(self.copy_path(slot)))?;
        not_found_is_fine(fs::remove_file(self.dir.join(format!("{slot}.lock"))))
    }
}

fn not_found_is_fine(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used)] // A store that fails to read back fails the test.
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("stochcraft-recovery-{}-{name}", new_session()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn two_sessions_keep_their_own_copies() {
        let dir = temp("two");
        let a = Store::open(dir.clone(), None, "a-1".into());
        let b = Store::open(dir.clone(), None, "b-1".into());
        a.write("copy of a").expect("a writes");
        b.write("copy of b").expect("b writes");
        // One session finishing (saved, discarded) removes its own copy only.
        b.clear().expect("b clears");
        assert_eq!(
            fs::read_to_string(dir.join("a-1.json")).expect("a's copy"),
            "copy of a"
        );
        assert!(!dir.join("b-1.json").exists());
        // Clearing again is fine.
        b.clear().expect("nothing to clear");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_running_session_is_not_offered_and_a_gone_one_is() {
        let dir = temp("gone");
        let a = Store::open(dir.clone(), None, "a-1".into());
        let b = Store::open(dir.clone(), None, "b-1".into());
        b.write("copy of b").expect("b writes");
        assert_eq!(a.orphans(), vec![], "b's window is still open");
        // b's window ends without clearing (a crash, or closing with changes unsaved).
        drop(b);
        assert_eq!(
            a.orphans(),
            vec![("b-1".to_owned(), "copy of b".to_owned())]
        );
        // Recovered or discarded: the copy goes, and a's own is untouched.
        a.write("copy of a").expect("a writes");
        a.remove("b-1").expect("removed");
        assert_eq!(a.orphans(), vec![]);
        assert!(dir.join("a-1.json").exists());
        assert!(!dir.join("b-1.lock").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_newest_copy_comes_first_and_the_old_shared_file_is_offered() {
        let dir = temp("order");
        let legacy = dir.with_extension("json");
        fs::write(&legacy, "old shared copy").expect("legacy file");
        let a = Store::open(dir.clone(), Some(legacy.clone()), "a-1".into());
        let slots: Vec<_> = a.orphans().into_iter().map(|(s, _)| s).collect();
        assert_eq!(slots, vec![LEGACY.to_owned()]);
        a.remove(LEGACY).expect("removed");
        assert!(!legacy.exists());
        assert_eq!(a.orphans(), vec![]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_plain_slot_name_is_removed() {
        let dir = temp("names");
        let a = Store::open(dir.clone(), None, "a-1".into());
        for bad in ["", "../x", "a/b", "a\\b", "a-1", "x.json"] {
            assert!(a.remove(bad).is_err(), "{bad}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_write_is_an_error() {
        // The "folder" is a file, so nothing can be made inside it.
        let file = temp("file");
        fs::write(&file, "x").expect("a file");
        let a = Store::open(file.join("recovery"), None, "a-1".into());
        assert!(a.write("copy").is_err());
        assert!(!file.join("recovery").exists());
        let _ = fs::remove_file(file);
    }
}
