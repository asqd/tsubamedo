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

/// The macOS menu without "Close Window": ⌘W closes a tab in the window, not the window.
/// The Edit menu stays, since text fields get ⌘C, ⌘V and undo through it.
#[cfg(target_os = "macos")]
fn menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, PredefinedMenuItem as Item, Submenu};
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                "TsubameDo",
                true,
                &[
                    &Item::about(app, Some("О TsubameDo"), None)?,
                    &Item::separator(app)?,
                    &Item::hide(app, Some("Скрыть TsubameDo"))?,
                    &Item::hide_others(app, Some("Скрыть остальные"))?,
                    &Item::show_all(app, Some("Показать все"))?,
                    &Item::separator(app)?,
                    &Item::quit(app, Some("Завершить TsubameDo"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Правка",
                true,
                &[
                    &Item::undo(app, Some("Отменить"))?,
                    &Item::redo(app, Some("Повторить"))?,
                    &Item::separator(app)?,
                    &Item::cut(app, Some("Вырезать"))?,
                    &Item::copy(app, Some("Скопировать"))?,
                    &Item::paste(app, Some("Вставить"))?,
                    &Item::select_all(app, Some("Выбрать все"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Окно",
                true,
                &[&Item::minimize(app, Some("Свернуть"))?, &Item::maximize(app, Some("Изменить масштаб"))?, &Item::fullscreen(app, None)?],
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
        .manage(Session::new())
        .invoke_handler(tauri::generate_handler![call])
        .run(tauri::generate_context!())
        .expect("TsubameDo failed to start");
}
