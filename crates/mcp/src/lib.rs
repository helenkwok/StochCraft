//! StochCraft's MCP server (docs/mcp.md).
//!
//! JSON-RPC 2.0 over stdio, one message per line, implementing the Model Context Protocol's
//! lifecycle, tools and resources. Every tool runs engine commands (the same ones the app,
//! scripts and the command line run) and never opens a dialog. A tool call is served by a
//! [`Backend`]: an engine in this process ([`Headless`]) or the running app reached through its
//! control channel ([`Remote`]), so a person can watch the agent work.
//!
//! The server treats its input as hostile: a message line is capped at [`MAX_LINE`] bytes and
//! answered with an error past it, a JSON-RPC batch at [`MAX_BATCH_MESSAGES`] messages, a
//! `batch` tool call at [`tools::MAX_STEPS`] commands, and tool arguments are checked against
//! the tool's schema before anything runs. Read and write [`Roots`] confine the files a headless
//! session opens and saves.
//!
//! The design follows FieldCraft's `crates/mcp` (MIT OR Apache-2.0), see ATTRIBUTION.md.

mod backend;
pub mod control;
pub mod roots;
mod server;
pub mod token;
pub mod tools;

pub use backend::{Backend, ConnectError, Headless, Remote, catalogue};
pub use roots::{Access, Roots};
pub use server::{
    MAX_BATCH_MESSAGES, MAX_LINE, PROTOCOL_VERSION, SUPPORTED_VERSIONS, Server, read_line,
};

/// How the server is started (`stochcraft-cli mcp`).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The control channel of a running app (`127.0.0.1:7070`); `None` runs headless.
    pub connect: Option<String>,
    /// The control channel's token (`--token`); else the environment variable `STOCHCRAFT_TOKEN`,
    /// else the file the app wrote ([`token::default_path`]).
    pub token: Option<String>,
    /// Where to look for the token file instead of [`token::default_path`].
    pub token_file: Option<std::path::PathBuf>,
    /// Folders file access is confined to; unconfined by default.
    pub roots: Roots,
}

/// The backend `options` ask for, and a note for stderr when the server falls back to headless
/// because the app cannot be reached (or is not running: no token and no token file); an app that
/// answers the token with an error, such as a wrong token or too many clients, is an error
/// instead, never a different engine. Confined roots with `connect` are refused: the app would
/// read and write files the roots do not cover. The token may come from `STOCHCRAFT_TOKEN`.
pub fn backend(options: Options) -> Result<(Box<dyn Backend>, Option<String>), String> {
    backend_with_environment(options, std::env::var("STOCHCRAFT_TOKEN").ok())
}

/// [`backend`], with the value of `STOCHCRAFT_TOKEN` given instead of read, so that a test need
/// not depend on or change the process's environment.
#[doc(hidden)]
pub fn backend_with_environment(
    options: Options,
    environment: Option<String>,
) -> Result<(Box<dyn Backend>, Option<String>), String> {
    match options.connect {
        Some(addr) if options.roots.is_confined() => Err(format!(
            "--connect {addr} cannot be combined with --read-root or --write-root: the app is not \
             confined by them"
        )),
        Some(addr) => {
            let file = options.token_file.clone().or_else(token::default_path);
            // The app writes its token file whenever its channel is open, so with no token given
            // and no file, the app is not running: run headless, as when it cannot be reached.
            let given = [options.token.as_deref(), environment.as_deref()]
                .into_iter()
                .flatten()
                .any(|t| !t.trim().is_empty());
            if let Some(f) = file.as_deref().filter(|f| !given && !f.exists()) {
                return Ok((
                    Box::new(Headless::new(options.roots)),
                    Some(format!(
                        "StochCraft at {addr} is not running (no token file at {}); running headless",
                        f.display()
                    )),
                ));
            }
            let token = token::resolve(
                options.token.as_deref(),
                environment.as_deref(),
                file.as_deref(),
            )?;
            match Remote::connect(&addr, &token) {
                Ok(r) => Ok((Box::new(r), None)),
                // The app is there but did not take the connection (a wrong token, all places
                // taken, a broken handshake): an error, not a reason to drive a different engine,
                // since the agent would think it was working in the window.
                Err(ConnectError::Refused(e)) => {
                    Err(format!("the app at {addr} refused the connection: {e}"))
                }
                Err(ConnectError::Broken(e)) => Err(format!(
                    "the connection to the app at {addr} was made but could not be set up: {e}"
                )),
                Err(ConnectError::Unreachable(e)) => Ok((
                    Box::new(Headless::new(options.roots)),
                    Some(format!(
                        "StochCraft at {addr} cannot be reached ({e}); running headless"
                    )),
                )),
            }
        }
        None => Ok((Box::new(Headless::new(options.roots)), None)),
    }
}
