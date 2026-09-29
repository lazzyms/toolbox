use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{App, AppHandle, Manager, PhysicalPosition, PhysicalSize, RunEvent, WebviewWindow, Window, WindowEvent};

const GEOMETRY_FILE: &str = "window-geometry.json";
const QUIET_PERIOD: Duration = Duration::from_millis(400);
const MIN_WIDTH: u32 = 640;
const MIN_HEIGHT: u32 = 480;
const MAX_WIDTH: u32 = 7680;
const MAX_HEIGHT: u32 = 4320;

#[derive(Clone, Copy, Deserialize, Serialize)]
struct WindowGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub struct WindowGeometryState {
    current: Mutex<WindowGeometry>,
    updates: mpsc::Sender<GeometryMessage>,
}

enum GeometryMessage {
    Changed(WindowGeometry),
    Flush { geometry: WindowGeometry, completed: mpsc::SyncSender<()> },
}

pub fn install_geometry(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app.get_webview_window("main").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Main window is unavailable"))?;
    let path = app.path().app_config_dir()?.join(GEOMETRY_FILE);
    if let Ok(saved) = fs::read(&path).and_then(|bytes| serde_json::from_slice::<WindowGeometry>(&bytes).map_err(io::Error::other)) {
        if valid_geometry(saved, &window) {
            let _ = window.set_size(PhysicalSize::new(saved.width, saved.height));
            let _ = window.set_position(PhysicalPosition::new(saved.x, saved.y));
        }
    }
    let position = window.outer_position()?;
    let size = window.outer_size()?;
    let current = WindowGeometry { x: position.x, y: position.y, width: size.width, height: size.height };
    let (updates, receiver) = mpsc::channel();
    let writer_path = path.clone();
    thread::Builder::new().name("toolbox-window-geometry".into()).spawn(move || persist_after_quiet_period(receiver, writer_path))?;
    app.manage(WindowGeometryState { current: Mutex::new(current), updates });
    Ok(())
}

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" { return; }
    let Some(state) = window.try_state::<WindowGeometryState>() else { return; };
    let mut current = state.current.lock().expect("window geometry lock poisoned");
    match event {
        WindowEvent::Moved(position) => { current.x = position.x; current.y = position.y; }
        WindowEvent::Resized(size) => { current.width = size.width; current.height = size.height; }
        _ => return,
    }
    let _ = state.updates.send(GeometryMessage::Changed(*current));
}

pub fn flush_geometry(app: &AppHandle) {
    let Some(state) = app.try_state::<WindowGeometryState>() else { return; };
    let geometry = *state.current.lock().expect("window geometry lock poisoned");
    let (completed, wait) = mpsc::sync_channel(0);
    if state.updates.send(GeometryMessage::Flush { geometry, completed }).is_ok() {
        let _ = wait.recv();
    }
}

pub fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn set_document_title(app: &AppHandle, title: &str) -> Result<(), String> {
    let window: WebviewWindow = app.get_webview_window("main").ok_or_else(|| "Main window is unavailable.".to_string())?;
    window.set_title(title).map_err(|error| error.to_string())
}

pub fn handle_run_event(app: &AppHandle, event: &RunEvent) {
    if matches!(event, RunEvent::Exit) {
        flush_geometry(app);
    }
}

fn valid_geometry(geometry: WindowGeometry, window: &WebviewWindow) -> bool {
    if !(MIN_WIDTH..=MAX_WIDTH).contains(&geometry.width) || !(MIN_HEIGHT..=MAX_HEIGHT).contains(&geometry.height) {
        return false;
    }
    let Ok(monitors) = window.available_monitors() else { return false; };
    monitors.iter().any(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        let left = i64::from(geometry.x).max(i64::from(position.x));
        let top = i64::from(geometry.y).max(i64::from(position.y));
        let right = (i64::from(geometry.x) + i64::from(geometry.width)).min(i64::from(position.x) + i64::from(size.width));
        let bottom = (i64::from(geometry.y) + i64::from(geometry.height)).min(i64::from(position.y) + i64::from(size.height));
        right - left >= 80 && bottom - top >= 40
    })
}

fn persist_after_quiet_period(receiver: mpsc::Receiver<GeometryMessage>, path: PathBuf) {
    while let Ok(message) = receiver.recv() {
        let mut latest = match message {
            GeometryMessage::Changed(geometry) => geometry,
            GeometryMessage::Flush { geometry, completed } => {
                persist_with_log(&path, geometry);
                let _ = completed.send(());
                return;
            }
        };
        loop {
            match receiver.recv_timeout(QUIET_PERIOD) {
                Ok(GeometryMessage::Changed(next)) => latest = next,
                Ok(GeometryMessage::Flush { geometry, completed }) => {
                    persist_with_log(&path, geometry);
                    let _ = completed.send(());
                    return;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => { persist_with_log(&path, latest); break; }
                Err(mpsc::RecvTimeoutError::Disconnected) => { persist_with_log(&path, latest); return; }
            }
        }
    }
}

fn persist_with_log(path: &Path, geometry: WindowGeometry) {
    if let Err(error) = persist_atomically(path, geometry) {
        eprintln!("Could not persist Toolbox window geometry: {error}");
    }
}

fn persist_atomically(path: &Path, geometry: WindowGeometry) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Geometry path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let bytes = serde_json::to_vec(&geometry).map_err(io::Error::other)?;
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
