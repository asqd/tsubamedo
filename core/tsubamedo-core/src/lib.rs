//! TsubameDo core: everything the app and the CLI do with requests.
//!
//! - [`model`] — a request as the user edits it, with `{{variables}}` still in place.
//! - [`request_file`] — one request per TOML file; saving rewrites only what changed.
//! - [`curl`] — `curl …` commands in and out.
//! - [`organize`] — renaming, moving and deleting requests and folders (to the Trash).
//! - [`git`] — what changed in the collection, commit, pull and push, through the system git.
//! - [`collection`] — a folder of request files, environments and their variables.
//! - [`cookies`] — what servers set, sent back like a browser does, a jar per environment.
//! - [`digest`] — Digest authentication, answered after the server's challenge.
//! - [`oauth`] — OAuth 2.0 tokens: got in the browser or directly, kept and renewed.
//! - [`sigv4`] — AWS Signature Version 4, made right before the request goes.
//! - [`prepare`] — variables substituted, auth and body turned into bytes and headers.
//! - [`http`] — sending with a time for every phase; big bodies go to a temporary file.
//! - [`view`] — lines of a body (raw or pretty JSON) for a viewer that never loads it whole.
//! - [`compare`] — two responses side by side: status, headers, JSON by meaning, text by lines.
//! - [`codegen`] — the request as code: curl, Python, JavaScript, Go and twenty more.
//! - [`checks`] — response checks (`status == 200`, `$.items count > 0`) and captured values.
//! - [`schema`] — JSON Schema for the `schema` check.
//! - [`run`] — requests that take values from each other's responses, in the right order.
//! - [`scripts`] — JavaScript before and after a request, on QuickJS.
//! - [`import`] — collections of Insomnia and Postman, OpenAPI and HAR.

pub mod checks;
pub mod codegen;
pub mod collection;
pub mod compare;
pub mod cookies;
pub mod curl;
pub mod digest;
pub mod error;
pub mod fake;
pub mod git;
pub mod http;
pub mod import;
pub mod model;
pub mod oauth;
pub mod organize;
pub mod prepare;
pub mod request_file;
pub mod run;
pub mod schema;
pub mod scripts;
pub mod sigv4;
pub mod template;
pub mod view;

pub use error::{Error, Result};
pub use model::{ApiKeyLocation, Auth, AwsV4, Body, Check, ClientAuth, Grant, OAuth2, Pair, Part, Request, RequestOptions, Scripts};
pub use template::Vars;
