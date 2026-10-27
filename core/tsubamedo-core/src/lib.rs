//! TsubameDo core: everything the app and the CLI do with requests.
//!
//! - [`model`] — a request as the user edits it, with `{{variables}}` still in place.
//! - [`request_file`] — one request per TOML file; saving rewrites only what changed.
//! - [`curl`] — `curl …` commands in and out.
//! - [`organize`] — renaming, moving and deleting requests and folders (to the Trash).
//! - [`collection`] — a folder of request files, environments and their variables.
//! - [`prepare`] — variables substituted, auth and body turned into bytes and headers.
//! - [`http`] — sending with a time for every phase; big bodies go to a temporary file.
//! - [`view`] — lines of a body (raw or pretty JSON) for a viewer that never loads it whole.
//! - [`checks`] — response checks (`status == 200`, `$.items count > 0`) and captured values.
//! - [`run`] — requests that take values from each other's responses, in the right order.

pub mod checks;
pub mod collection;
pub mod curl;
pub mod error;
pub mod http;
pub mod model;
pub mod organize;
pub mod prepare;
pub mod request_file;
pub mod run;
pub mod template;
pub mod view;

pub use error::{Error, Result};
pub use model::{ApiKeyLocation, Auth, Body, Check, Pair, Part, Request, RequestOptions};
pub use template::Vars;
