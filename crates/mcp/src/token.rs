//! The control channel's token (docs/mcp.md).
//!
//! Listening on the loopback address limits where a connection comes from, not who makes it, so
//! the app makes a random token when it opens the channel and writes it, with the port, to a
//! file only its user can read. A client presents the token as its first line and the channel
//! closes any connection that does not. This module makes the token, keeps the file for as long
//! as the channel is open, and finds the token on the client's side.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The token's length in bytes: 256 bits, shown as 64 hexadecimal digits.
pub const TOKEN_BYTES: usize = 32;

/// A new random token from the operating system's random source, in hexadecimal.
pub fn generate() -> io::Result<String> {
    let bytes = random_bytes()?;
    let mut hex = String::with_capacity(TOKEN_BYTES * 2);
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
}

#[cfg(not(target_arch = "wasm32"))]
fn random_bytes() -> io::Result<[u8; TOKEN_BYTES]> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|e| io::Error::other(format!("no random numbers from the system: {e}")))?;
    Ok(bytes)
}

/// The browser build has no control channel to open, and no system random source here.
#[cfg(target_arch = "wasm32")]
fn random_bytes() -> io::Result<[u8; TOKEN_BYTES]> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the control channel is not available in the browser",
    ))
}

/// Whether two byte strings are equal, in a time that depends on their length only and not on
/// where they first differ (the length is not secret: tokens are all 64 digits).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let difference = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(difference) == 0
}

/// Where the app writes the token file and the client looks for it: on Linux and other Unix
/// systems `$XDG_RUNTIME_DIR/stochcraft/control.json` (a folder only the user can open), else the
/// user's config folder; on Windows `%LOCALAPPDATA%\StochCraft\control.json`, which the profile's
/// default permissions already keep to the user.
pub fn default_path() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::data_local_dir().map(|d| d.join("StochCraft").join("control.json"))
    } else {
        dirs::runtime_dir()
            .or_else(dirs::config_dir)
            .map(|d| d.join("stochcraft").join("control.json"))
    }
}

/// The port and token in a token file.
pub fn read(path: &Path) -> Result<(u16, String), String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("the token file {} cannot be read: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("the token file {} is not JSON: {e}", path.display()))?;
    let port = value["port"]
        .as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .ok_or_else(|| format!("the token file {} has no `port`", path.display()))?;
    let token = value["token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("the token file {} has no `token`", path.display()))?;
    Ok((port, token.to_owned()))
}

/// The token for `--connect`, from the first of these that is given: `--token`, the environment
/// variable `STOCHCRAFT_TOKEN`, the token file the app wrote. A failure names what was missing.
pub fn resolve(
    explicit: Option<&str>,
    environment: Option<&str>,
    file: Option<&Path>,
) -> Result<String, String> {
    for given in [explicit, environment].into_iter().flatten() {
        let given = given.trim();
        if !given.is_empty() {
            return Ok(given.to_owned());
        }
    }
    let Some(file) = file else {
        return Err(
            "no token for --connect, and this system has no place for the app's token file: pass \
             --token or set STOCHCRAFT_TOKEN"
                .into(),
        );
    };
    if !file.exists() {
        return Err(format!(
            "no token for --connect: the app's token file {} does not exist (is the app running \
             with --control?); pass --token or set STOCHCRAFT_TOKEN to give one by hand",
            file.display()
        ));
    }
    read(file).map(|(_, token)| token)
}

/// The token file while the channel is open: removed when this is dropped, if it is still the
/// one written here.
pub struct TokenFile {
    path: PathBuf,
    token: String,
}

impl TokenFile {
    /// Writes `{"port", "token"}` to `path`: first to a file of its own beside it that only the
    /// user can read (mode 0600 on Unix), then renamed over `path`, so a reader never sees half
    /// of it. If `path` names another app's port that still answers, that app owns the file and
    /// this fails with `AddrInUse` rather than hide its token. Looking at the file and replacing
    /// it happen under a lock file beside it (`control.json.lock`), so two apps starting at once
    /// cannot both decide the file is free; a lock held for more than [`LOCK_STALE`] is taken to
    /// be left by a crash and removed, and one that stays held for [`LOCK_WAIT`] is an error.
    pub fn publish(path: &Path, port: u16, token: &str) -> io::Result<Self> {
        let folder = path
            .parent()
            .ok_or_else(|| io::Error::other("the token file has no folder"))?;
        make_folder(folder)?;
        let _lock = Lock::take(&folder.join("control.json.lock"))?;
        if let Ok((other, _)) = read(path)
            && other != port
            && answers(other)
        {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!(
                    "another StochCraft app is running its control channel on port {other} and \
                     owns {}; close it first",
                    path.display()
                ),
            ));
        }
        let temp = folder.join(format!("control.json.{}.tmp", std::process::id()));
        let _ = fs::remove_file(&temp);
        let written = write_private(&temp, &json!({ "port": port, "token": token }).to_string())
            .and_then(|()| fs::rename(&temp, path));
        if let Err(e) = written {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        Ok(Self {
            path: path.to_owned(),
            token: token.to_owned(),
        })
    }
}

impl Drop for TokenFile {
    fn drop(&mut self) {
        // Not another app's file, written after this one's.
        if read(&self.path).is_ok_and(|(_, token)| token == self.token) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// A lock held for longer than this was left by an app that stopped without removing it.
const LOCK_STALE: Duration = Duration::from_secs(10);
/// How long to wait for another app's lock before giving up.
const LOCK_WAIT: Duration = Duration::from_secs(2);

/// A lock between processes: a file created with `create_new`, which only one can do, removed
/// when this is dropped.
struct Lock(PathBuf);

impl Lock {
    fn take(path: &Path) -> io::Result<Self> {
        let started = Instant::now();
        loop {
            match write_private(path, &std::process::id().to_string()) {
                Ok(()) => return Ok(Lock(path.to_owned())),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = fs::remove_file(path);
                    } else if started.elapsed() > LOCK_WAIT {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "another StochCraft app is setting up the token file ({} is held)",
                                path.display()
                            ),
                        ));
                    } else {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Whether something answers on `127.0.0.1:port`.
fn answers(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// The file's folder, made if need be. On Unix it is made with mode 0700, and one that already
/// exists with another mode is changed to 0700; if that cannot be done (it is not the user's),
/// this fails rather than leave the token in a folder others can open.
fn make_folder(folder: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(folder)?;
        let mode = fs::metadata(folder)?.permissions().mode() & 0o777;
        if mode != 0o700 {
            fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "the folder {} for the token file has mode {mode:o}, not 700, and cannot \
                         be made private: {e}",
                        folder.display()
                    ),
                )
            })?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        builder.create(folder)
    }
}

/// Creates `path` (which must not exist) readable by the user alone, and writes `text` and a
/// newline to it.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stochcraft-token-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tokens_are_256_bits_of_hex_and_differ() {
        let (a, b) = (generate().unwrap(), generate().unwrap());
        assert_eq!(a.len(), TOKEN_BYTES * 2);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn the_comparison_is_equal_only_for_equal_bytes() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"abc123", b"abc123"));
        assert!(!constant_time_eq(b"abc123", b"abc124"));
        assert!(!constant_time_eq(b"abc123", b"bbc123"));
        assert!(!constant_time_eq(b"abc123", b"abc12"));
        assert!(!constant_time_eq(b"abc12", b"abc123"));
        assert!(!constant_time_eq(b"", b"a"));
    }

    #[test]
    fn the_file_holds_the_port_and_token_and_goes_when_dropped() {
        let dir = scratch("file");
        let path = dir.join("stochcraft").join("control.json");
        let file = TokenFile::publish(&path, 7070, "abc").unwrap();
        assert_eq!(read(&path).unwrap(), (7070, "abc".to_owned()));
        // No temporary file is left beside it.
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        drop(file);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_another_app_wrote_since_is_left_alone() {
        let dir = scratch("other");
        let path = dir.join("control.json");
        // A port nothing answers on: the first app is gone, its file is replaced.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let first = TokenFile::publish(&path, closed, "first").unwrap();
        let second = TokenFile::publish(&path, 7071, "second").unwrap();
        drop(first);
        assert_eq!(read(&path).unwrap(), (7071, "second".to_owned()));
        drop(second);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_running_apps_file_is_not_taken() {
        let dir = scratch("live");
        let path = dir.join("control.json");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap().port();
        let first = TokenFile::publish(&path, live, "first").unwrap();
        let refused = TokenFile::publish(&path, 1, "second");
        assert_eq!(refused.err().unwrap().kind(), io::ErrorKind::AddrInUse);
        assert_eq!(read(&path).unwrap(), (live, "first".to_owned()));
        drop(first);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_apps_publishing_at_once_leave_one_winner() {
        for round in 0..20 {
            let dir = scratch(&format!("race{round}"));
            let path = dir.join("control.json");
            // Both ports answer (they are listening), as running apps' do.
            let listeners: Vec<_> = (0..2)
                .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
                .collect();
            let ports: Vec<u16> = listeners
                .iter()
                .map(|l| l.local_addr().unwrap().port())
                .collect();
            let barrier = std::sync::Barrier::new(2);
            let results: Vec<io::Result<TokenFile>> = std::thread::scope(|scope| {
                let handles: Vec<_> = ports
                    .iter()
                    .enumerate()
                    .map(|(i, port)| {
                        let (path, barrier) = (&path, &barrier);
                        scope.spawn(move || {
                            barrier.wait();
                            TokenFile::publish(path, *port, &format!("token{i}"))
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let winners: Vec<usize> = (0..2).filter(|i| results[*i].is_ok()).collect();
            assert_eq!(winners.len(), 1, "round {round}");
            let loser = results.iter().find_map(|r| r.as_ref().err()).unwrap();
            assert_eq!(loser.kind(), io::ErrorKind::AddrInUse, "{loser}");
            // The file is the winner's, and the lock is gone.
            let (port, token) = read(&path).unwrap();
            assert_eq!(
                (port, token),
                (ports[winners[0]], format!("token{}", winners[0]))
            );
            assert!(!dir.join("control.json.lock").exists());
            drop(results);
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_lock_left_by_a_crash_is_removed_and_a_fresh_one_is_waited_for() {
        let dir = scratch("lock");
        let path = dir.join("control.json");
        let lock = dir.join("control.json.lock");
        // Left 20 seconds ago: stale.
        fs::write(&lock, "1").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(20);
        fs::OpenOptions::new()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let file = TokenFile::publish(&path, 7070, "abc").unwrap();
        assert!(!lock.exists());
        drop(file);
        // Held just now by someone else: an error after the wait, and the lock is not theirs to
        // remove.
        fs::write(&lock, "1").unwrap();
        let held = TokenFile::publish(&path, 7070, "abc");
        assert_eq!(held.err().unwrap().kind(), io::ErrorKind::TimedOut);
        assert!(lock.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_exists_with_wider_permissions_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("widemode");
        let folder = dir.join("stochcraft");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        let _file = TokenFile::publish(&folder.join("control.json"), 7070, "abc").unwrap();
        assert_eq!(
            fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn only_the_user_can_read_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        let path = dir.join("stochcraft").join("control.json");
        let _file = TokenFile::publish(&path, 7070, "abc").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_token_comes_from_the_argument_then_the_environment_then_the_file() {
        let dir = scratch("resolve");
        let path = dir.join("control.json");
        assert!(
            resolve(None, None, Some(&path))
                .unwrap_err()
                .contains("control.json")
        );
        assert!(resolve(None, None, None).is_err());
        let _file = TokenFile::publish(&path, 7070, "fromfile").unwrap();
        assert_eq!(resolve(None, None, Some(&path)).unwrap(), "fromfile");
        assert_eq!(
            resolve(None, Some("fromenv"), Some(&path)).unwrap(),
            "fromenv"
        );
        assert_eq!(
            resolve(Some(" given "), Some("fromenv"), Some(&path)).unwrap(),
            "given"
        );
        // An empty one counts as not given.
        assert_eq!(
            resolve(Some(""), Some(""), Some(&path)).unwrap(),
            "fromfile"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
