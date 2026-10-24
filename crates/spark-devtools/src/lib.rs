//! Spark devtools: the live link between a running game and Spark Editor.
//!
//! ```text
//!  spark-editor                               spark <game> --devtools 127.0.0.1:PORT
//!  ┌──────────────┐   EditorCommand (JSON)   ┌──────────────────────┐
//!  │ Server       │ ───────────────────────▶ │ Link (in the runner) │
//!  │ (listens)    │ ◀─────────────────────── │                      │
//!  └──────────────┘   GameMessage (JSON)     └──────────────────────┘
//! ```
//!
//! - Transport: TCP on localhost, one JSON object per line (easy to debug with `nc`).
//! - The editor listens, the game connects (the editor passes the address with `--devtools`).
//! - Logs are NOT sent here: the editor reads the game's stdout / stderr.
//! - Both sides do socket I/O on background threads: a slow or dead peer never stalls a frame.
//! - Versioned: [`PROTOCOL_VERSION`] travels in [`GameMessage::Hello`]; unknown messages are skipped.

mod link;
mod protocol;

pub use link::{Link, Server};
pub use protocol::*;

/// Bumped on incompatible changes of [`GameMessage`] / [`EditorCommand`].
pub const PROTOCOL_VERSION: u32 = 1;

/// The runner flag that enables devtools: `spark game --devtools 127.0.0.1:50123`.
pub const FLAG: &str = "--devtools";
