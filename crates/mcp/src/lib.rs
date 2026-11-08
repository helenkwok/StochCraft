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
pub mod tools;

pub use backend::{Backend, Headless, Remote, catalogue};
pub use roots::{Access, Roots};
pub use server::{
    MAX_BATCH_MESSAGES, MAX_LINE, PROTOCOL_VERSION, SUPPORTED_VERSIONS, Server, read_line,
};

/// How the server is started (`stochcraft-cli mcp`).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The control channel of a running app (`127.0.0.1:7070`); `None` runs headless.
    pub connect: Option<String>,
    /// Folders file access is confined to; unconfined by default.
    pub roots: Roots,
}

/// The backend `options` ask for, and a note for stderr when the server falls back to headless
/// because the app cannot be reached. Confined roots with `connect` are refused: the app would
/// read and write files the roots do not cover.
pub fn backend(options: Options) -> Result<(Box<dyn Backend>, Option<String>), String> {
    match options.connect {
        Some(addr) if options.roots.is_confined() => Err(format!(
            "--connect {addr} cannot be combined with --read-root or --write-root: the app is not \
             confined by them"
        )),
        Some(addr) => match Remote::connect(&addr) {
            Ok(r) => Ok((Box::new(r), None)),
            Err(e) => Ok((
                Box::new(Headless::new(options.roots)),
                Some(format!(
                    "StochCraft at {addr} cannot be reached ({e}); running headless"
                )),
            )),
        },
        None => Ok((Box::new(Headless::new(options.roots)), None)),
    }
}
