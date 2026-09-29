mod activation;
mod events;
mod menu;
mod window;

use std::sync::Mutex;

use events::ShellEvent;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct ShellQueue {
    ready: bool,
    pending: Vec<ShellEvent>,
}

#[derive(Default)]
pub struct ShellState(Mutex<ShellQueue>);

pub(crate) fn dispatch(app: &AppHandle, event: ShellEvent) {
    let state = app.state::<ShellState>();
    let mut queue = state.0.lock().expect("shell event queue poisoned");
    if !queue.ready {
        queue.pending.push(event);
        return;
    }
    if matches!(event, ShellEvent::Files { .. }) {
        queue.pending.push(event.clone());
    }
    drop(queue);
    let _ = app.emit(events::SHELL_EVENT, event);
}

#[tauri::command]
pub fn shell_ready(app: AppHandle, state: State<'_, ShellState>) {
    let pending = {
        let mut queue = state.0.lock().expect("shell event queue poisoned");
        queue.ready = true;
        let pending = queue.pending.clone();
        queue.pending.retain(|event| matches!(event, ShellEvent::Files { .. }));
        pending
    };
    for event in pending {
        let _ = app.emit(events::SHELL_EVENT, event);
    }
}

#[tauri::command]
pub fn acknowledge_activation(activation_id: String, state: State<'_, ShellState>) {
    state.0.lock().expect("shell event queue poisoned").pending.retain(|event| {
        !matches!(event, ShellEvent::Files { activation_id: queued, .. } if *queued == activation_id)
    });
}

#[tauri::command]
pub fn open_paths(app: AppHandle, paths: Vec<String>) {
    dispatch(&app, activation::parse_external_paths(paths));
}

#[tauri::command]
pub fn set_document_title(app: AppHandle, title: String) -> Result<(), String> {
    window::set_document_title(&app, &title)
}

pub fn menu(app: &mut tauri::App) -> tauri::Result<()> {
    menu::install(app)
}

pub fn install_geometry(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    window::install_geometry(app)
}

pub(crate) fn parse_paths(paths: impl IntoIterator<Item = String>) -> ShellEvent {
    activation::parse_external_paths(paths)
}

pub fn focus(app: &AppHandle) {
    window::focus_main_window(app);
}

pub fn handle_drop(app: &AppHandle, paths: Vec<std::path::PathBuf>) {
    let paths = paths.into_iter().map(|path| path.to_string_lossy().into_owned()).collect::<Vec<_>>();
    match activation::parse_external_paths(paths.clone()) {
        event @ ShellEvent::Files { .. } => dispatch(app, event),
        _ => dispatch(app, ShellEvent::DroppedFiles { paths }),
    }
}

pub fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    window::handle_window_event(window, event);
}

pub fn handle_run_event(app: &AppHandle, event: &tauri::RunEvent) {
    window::handle_run_event(app, event);
    #[cfg(target_os = "macos")]
    if let tauri::RunEvent::Opened { urls } = event {
        dispatch(app, activation::parse_external_paths(urls.iter().filter_map(|url| url.to_file_path().ok()).map(|path| path.to_string_lossy().into_owned())));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = event;
}
