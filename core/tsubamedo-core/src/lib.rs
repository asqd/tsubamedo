//! TsubameDo core: everything the app and the CLI do with requests.
//!
//! - [`model`] — a request as the user edits it, with `{{variables}}` still in place.
//! - [`request_file`] — one request per TOML file; saving rewrites only what changed.
//! - [`curl`] — `curl …` commands in and out.
//! - [`collection`] — a folder of request files, environments and their variables.
//! - [`prepare`] — variables substituted, auth and body turned into bytes and headers.
//! - [`http`] — sending with a time for every phase; big bodies go to a temporary file.
//! - [`view`] — lines of a body (raw or pretty JSON) for a viewer that never loads it whole.

pub mod collection;
pub mod curl;
pub mod error;
pub mod http;
pub mod model;
pub mod prepare;
pub mod request_file;
pub mod template;
pub mod view;

pub use error::{Error, Result};
pub use model::{ApiKeyLocation, Auth, Body, Pair, Part, Request, RequestOptions};
pub use template::Vars;
