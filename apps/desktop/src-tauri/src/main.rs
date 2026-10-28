//! TsubameDo desktop shell: one command, `call`, forwards everything to the session, the same
//! one the dev bridge serves to a browser.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use serde_json::Value;
use tauri::{Manager, State};
use tsubamedo_session::Session;

#[tauri::command]
async fn call(app: tauri::AppHandle, session: State<'_, Arc<Session>>, method: String, args: Value) -> Result<Value, String> {
    let reply = session.inner().clone().call(&method, args).await;
    // The menu speaks the window's language: it is told at every start and on every change.
    #[cfg(target_os = "macos")]
    if method == "setLanguage" && reply.is_ok() {
        app.set_menu(menu(&app).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    reply
}

/// The macOS menu without "Close Window": ⌘W closes a tab in the window, not the window.
/// The Edit menu stays, since text fields get ⌘C, ⌘V and undo through it.
#[cfg(target_os = "macos")]
fn menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, PredefinedMenuItem as Item, Submenu};
    use tsubamedo_core::tr_static;
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                "TsubameDo",
                true,
                &[
                    &Item::about(app, Some(tr_static!("О TsubameDo", "About TsubameDo")), None)?,
                    &Item::separator(app)?,
                    &Item::hide(app, Some(tr_static!("Скрыть TsubameDo", "Hide TsubameDo")))?,
                    &Item::hide_others(app, Some(tr_static!("Скрыть остальные", "Hide Others")))?,
                    &Item::show_all(app, Some(tr_static!("Показать все", "Show All")))?,
                    &Item::separator(app)?,
                    &Item::quit(app, Some(tr_static!("Завершить TsubameDo", "Quit TsubameDo")))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                tr_static!("Правка", "Edit"),
                true,
                &[
                    &Item::undo(app, Some(tr_static!("Отменить", "Undo")))?,
                    &Item::redo(app, Some(tr_static!("Повторить", "Redo")))?,
                    &Item::separator(app)?,
                    &Item::cut(app, Some(tr_static!("Вырезать", "Cut")))?,
                    &Item::copy(app, Some(tr_static!("Скопировать", "Copy")))?,
                    &Item::paste(app, Some(tr_static!("Вставить", "Paste")))?,
                    &Item::select_all(app, Some(tr_static!("Выбрать все", "Select All")))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                tr_static!("Окно", "Window"),
                true,
                &[&Item::minimize(app, Some(tr_static!("Свернуть", "Minimize")))?, &Item::maximize(app, Some(tr_static!("Изменить масштаб", "Zoom")))?, &Item::fullscreen(app, None)?],
            )?,
        ],
    )
}

fn main() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu);
    builder
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // History lives in ~/Library/Application Support/app.tsubamedo.TsubameDo.
            app.manage(Session::with_data_dir(app.path().app_data_dir()?));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![call])
        .run(tauri::generate_context!())
        .expect("TsubameDo failed to start");
}
