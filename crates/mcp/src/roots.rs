//! Folders an agent may read from and write to.
//!
//! An agent driving StochCraft through the MCP server can be kept to the folders it was given:
//! every file a command reads must lie inside the read root, every file it writes inside the
//! write root. [`Roots::check`] decides, before the file is touched:
//!
//! - The path is made absolute against the working folder, then its links are followed: a file
//!   (or folder) link that leads out of a root is outside it. For a file that does not exist yet,
//!   its nearest existing folder's links are followed and the rest must be plain names, so `..`
//!   below a folder that does not exist is refused, as is a link that leads nowhere (writing
//!   through it would create its target).
//! - The result is compared with the root by whole folder names: `/work2/x` is not inside
//!   `/work`.
//! - Read and write access are separate; a root left out grants none of its access. Without
//!   either root ([`Roots::default`]) nothing is confined.
//!
//! A path is checked, then opened: another program that swaps a link in between can still win
//! that race, so the roots keep an agent to the files it was given rather than guarding against
//! other software on the computer.
//!
//! Adapted from FieldCraft's `crates/io/src/roots.rs` (MIT OR Apache-2.0), see ATTRIBUTION.md.

use std::path::{Component, Path, PathBuf};

/// What a command does with a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

impl Access {
    fn name(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

/// The folders file access is confined to; the default confines nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    /// `None` while unconfined; otherwise the folder each of read and write access is granted
    /// in (`None`: not granted).
    confined: Option<[Option<PathBuf>; 2]>,
}

fn resolve_root(p: &Path, access: Access) -> Result<PathBuf, String> {
    let shown = p.display();
    let dir = std::fs::canonicalize(p)
        .map_err(|e| format!("the {} root `{shown}`: {e}", access.name()))?;
    if !dir.is_dir() {
        return Err(format!(
            "the {} root `{shown}` is not a folder",
            access.name()
        ));
    }
    Ok(dir)
}

impl Roots {
    /// Confine reads to `read` and writes to `write`. A root left out grants none of its access.
    /// Each root must be an existing folder; it is resolved (links followed) once, here.
    pub fn new(read: Option<&Path>, write: Option<&Path>) -> Result<Roots, String> {
        let read = read.map(|p| resolve_root(p, Access::Read)).transpose()?;
        let write = write.map(|p| resolve_root(p, Access::Write)).transpose()?;
        Ok(Roots {
            confined: Some([read, write]),
        })
    }

    /// Whether any access is confined.
    pub fn is_confined(&self) -> bool {
        self.confined.is_some()
    }

    /// The path to use for `access` to `path`: unchanged while unconfined, else the resolved path
    /// (absolute, links followed), which lies inside the root. Refused with a message naming the
    /// root and the path.
    pub fn check(&self, path: &Path, access: Access) -> Result<PathBuf, String> {
        let Some(roots) = &self.confined else {
            return Ok(path.to_path_buf());
        };
        let shown = path.display();
        let Some(root) = roots.get(access as usize).and_then(Option::as_ref) else {
            return Err(format!(
                "file access is not granted: no {} root was given: {shown}",
                access.name()
            ));
        };
        let resolved = resolve(path).map_err(|why| format!("path refused: {why}: {shown}"))?;
        if !resolved.starts_with(root) {
            return Err(format!(
                "path refused: outside the {} root {}: {shown}",
                access.name(),
                root.display()
            ));
        }
        Ok(resolved)
    }
}

/// `path` absolute, its links followed: wholly when it exists, else up to its nearest existing
/// folder, below which only plain names may follow.
fn resolve(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    #[cfg(windows)]
    if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
        const DEVICES: [&str; 22] = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        if DEVICES
            .iter()
            .any(|d| d.eq_ignore_ascii_case(name.trim_end()))
        {
            return Err("a device name".into());
        }
    }
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    if let Ok(p) = std::fs::canonicalize(&absolute) {
        return Ok(p);
    }
    // A link that leads nowhere: writing through it would create its target.
    if std::fs::symlink_metadata(&absolute).is_ok() {
        return Err("a link that leads nowhere".into());
    }
    let mut rest = Vec::new();
    let mut base = absolute.as_path();
    loop {
        let Some(parent) = base.parent() else {
            return Err("no existing folder above it".into());
        };
        let Some(name) = base.file_name() else {
            return Err("not a plain file name".into());
        };
        rest.push(name.to_os_string());
        base = parent;
        if let Ok(dir) = std::fs::canonicalize(base) {
            if !dir.is_dir() {
                return Err(format!("{} is not a folder", base.display()));
            }
            let mut out = dir;
            for name in rest.iter().rev() {
                out.push(name);
            }
            return Ok(out);
        }
        if std::fs::symlink_metadata(base).is_ok() {
            return Err("a link that leads nowhere".into());
        }
        // `..` or `.` below a folder that does not exist cannot be resolved.
        if !matches!(base.components().next_back(), Some(Component::Normal(_))) {
            return Err("`..` below a folder that does not exist".into());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("stochcraft-roots-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(std::fs::canonicalize(&d).unwrap())
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn unconfined_changes_nothing() {
        let r = Roots::default();
        assert!(!r.is_confined());
        assert_eq!(
            r.check(Path::new("/etc/passwd"), Access::Write).unwrap(),
            Path::new("/etc/passwd")
        );
    }

    #[test]
    fn inside_outside_and_access_kinds() {
        let d = Dir::new("kinds");
        let (read, write) = (d.0.join("in"), d.0.join("out"));
        std::fs::create_dir_all(&read).unwrap();
        std::fs::create_dir_all(&write).unwrap();
        std::fs::write(read.join("m.bpmn"), "x").unwrap();
        let r = Roots::new(Some(&read), Some(&write)).unwrap();
        assert_eq!(
            r.check(&read.join("m.bpmn"), Access::Read).unwrap(),
            read.join("m.bpmn")
        );
        // A new file in the write root, but not in the read root.
        assert!(r.check(&write.join("new.bpmn"), Access::Write).is_ok());
        assert!(r.check(&write.join("new.bpmn"), Access::Read).is_err());
        assert!(r.check(&read.join("m.bpmn"), Access::Write).is_err());
        // `..` out of the root, a sibling whose name starts the same, and the empty path.
        assert!(r.check(&read.join("../out/x"), Access::Read).is_err());
        let sibling = d.0.join("in2");
        std::fs::create_dir_all(&sibling).unwrap();
        assert!(r.check(&sibling.join("x"), Access::Read).is_err());
        assert!(r.check(Path::new(""), Access::Read).is_err());
        // `..` below a folder that does not exist: refused on Unix, where the system would look
        // for the folder. Windows removes `..` from the path's text before opening it (as
        // `std::path::absolute` does), so there it is the file `x` in the root, which is what
        // opening it would reach, even through a link (#77).
        let up = r.check(&write.join("nope/../x"), Access::Write);
        if cfg!(windows) {
            assert_eq!(up, r.check(&write.join("x"), Access::Write));
            assert!(up.is_ok(), "{up:?}");
        } else {
            assert!(up.is_err(), "{up:?}");
        }
        // Only a read root: nothing can be written.
        let only_read = Roots::new(Some(&read), None).unwrap();
        let e = only_read
            .check(&read.join("m.bpmn"), Access::Write)
            .unwrap_err();
        assert!(e.contains("no write root"), "{e}");
        // A root must exist.
        assert!(Roots::new(Some(&d.0.join("missing")), None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn links_out_of_a_root_are_outside_it() {
        let d = Dir::new("links");
        let root = d.0.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(d.0.join("secret"), "x").unwrap();
        std::os::unix::fs::symlink(d.0.join("secret"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(d.0.join("nowhere"), root.join("dangling")).unwrap();
        let r = Roots::new(Some(&root), Some(&root)).unwrap();
        assert!(r.check(&root.join("link"), Access::Read).is_err());
        assert!(r.check(&root.join("dangling"), Access::Write).is_err());
    }
}
