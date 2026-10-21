use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{App, AppHandle};

use super::events::{ShellCommand, ShellEvent};

pub fn install(app: &mut App) -> tauri::Result<()> {
    let handle = app.handle();
    let open = item(handle, "open", "Open…", "CmdOrCtrl+O")?;
    let export = item(handle, "export", "Export", "CmdOrCtrl+E")?;
    let close = PredefinedMenuItem::close_window(handle, Some("Close Window"))?;
    #[cfg(not(target_os = "macos"))]
    let file = Submenu::with_items(handle, "File", true, &[&open, &export, &PredefinedMenuItem::separator(handle)?, &close, &PredefinedMenuItem::quit(handle, None)?])?;
    #[cfg(target_os = "macos")]
    let file = Submenu::with_items(handle, "File", true, &[&open, &export, &PredefinedMenuItem::separator(handle)?, &close])?;

    let undo = item(handle, "undo", "Undo", "CmdOrCtrl+Z")?;
    let redo = item(handle, "redo", "Redo", "CmdOrCtrl+Shift+Z")?;
    let edit = Submenu::with_items(handle, "Edit", true, &[&undo, &redo, &PredefinedMenuItem::separator(handle)?, &PredefinedMenuItem::cut(handle, None)?, &PredefinedMenuItem::copy(handle, None)?, &PredefinedMenuItem::paste(handle, None)?, &PredefinedMenuItem::select_all(handle, None)?])?;

    let zoom_in = item(handle, "zoom-in", "Zoom In", "CmdOrCtrl++")?;
    let zoom_out = item(handle, "zoom-out", "Zoom Out", "CmdOrCtrl+-")?;
    let zoom_reset = item(handle, "zoom-reset", "Actual Size", "CmdOrCtrl+0")?;
    let theme = item(handle, "toggle-theme", "Toggle Theme", "")?;
    let view = Submenu::with_items(handle, "View", true, &[&zoom_in, &zoom_out, &zoom_reset, &PredefinedMenuItem::separator(handle)?, &theme, &PredefinedMenuItem::fullscreen(handle, None)?])?;

    let minimize = PredefinedMenuItem::minimize(handle, None)?;
    let maximize = PredefinedMenuItem::maximize(handle, None)?;
    let window = Submenu::with_items(handle, "Window", true, &[&minimize, &maximize, &PredefinedMenuItem::separator(handle)?, &PredefinedMenuItem::close_window(handle, None)?])?;

    let shortcuts = item(handle, "shortcuts", "Keyboard Shortcuts", "")?;
    #[cfg(not(target_os = "macos"))]
    let help_about = PredefinedMenuItem::about(handle, Some("About Toolbox"), None)?;
    #[cfg(target_os = "macos")]
    let help = Submenu::with_items(handle, "Help", true, &[&shortcuts])?;
    #[cfg(not(target_os = "macos"))]
    let help = Submenu::with_items(handle, "Help", true, &[&shortcuts, &PredefinedMenuItem::separator(handle)?, &help_about])?;

    #[cfg(target_os = "macos")]
    let app_about = PredefinedMenuItem::about(handle, Some("About Toolbox"), None)?;
    #[cfg(target_os = "macos")]
    let app_menu = Submenu::with_items(handle, "Toolbox", true, &[&app_about, &PredefinedMenuItem::separator(handle)?, &PredefinedMenuItem::quit(handle, None)?])?;
    #[cfg(target_os = "macos")]
    let menu = Menu::with_items(handle, &[&app_menu, &file, &edit, &view, &window, &help])?;
    #[cfg(not(target_os = "macos"))]
    let menu = Menu::with_items(handle, &[&file, &edit, &view, &window, &help])?;

    app.set_menu(menu)?;
    app.on_menu_event(|handle, event| {
        let command = match event.id().as_ref() {
            "open" => Some(ShellCommand::Open),
            "export" => Some(ShellCommand::Export),
            "undo" => Some(ShellCommand::Undo),
            "redo" => Some(ShellCommand::Redo),
            "zoom-in" => Some(ShellCommand::ZoomIn),
            "zoom-out" => Some(ShellCommand::ZoomOut),
            "zoom-reset" => Some(ShellCommand::ZoomReset),
            "toggle-theme" => Some(ShellCommand::ToggleTheme),
            "shortcuts" => Some(ShellCommand::Shortcuts),
            _ => None,
        };
        if let Some(command) = command {
            super::dispatch(handle, ShellEvent::Command { command });
        }
    });
    Ok(())
}

fn item(handle: &AppHandle, id: &str, label: &str, accelerator: &str) -> tauri::Result<MenuItem<tauri::Wry>> {
    MenuItem::with_id(handle, id, label, true, (!accelerator.is_empty()).then_some(accelerator))
}
