//! TsubameDo desktop shell: one command, `call`, forwards everything to the session, the same
//! one the dev bridge serves to a browser.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use serde_json::Value;
use tauri::State;
use tsubamedo_session::Session;

#[tauri::command]
async fn call(session: State<'_, Arc<Session>>, method: String, args: Value) -> Result<Value, String> {
    session.inner().clone().call(&method, args).await
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::new())
        .invoke_handler(tauri::generate_handler![call])
        .run(tauri::generate_context!())
        .expect("TsubameDo failed to start");
}
