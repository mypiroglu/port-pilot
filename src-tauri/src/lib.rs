mod ports;

use std::sync::Mutex;

use ports::{KillResult, PortEntry};
use sysinfo::System;
use tauri::State;

/// `System` is kept between calls so CPU usage can be computed from deltas.
struct AppState {
    sys: Mutex<System>,
}

#[tauri::command(async)]
fn list_ports(state: State<'_, AppState>, listening_only: bool) -> Result<Vec<PortEntry>, String> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    ports::list_ports(&mut sys, listening_only)
}

#[tauri::command(async)]
fn kill_process(state: State<'_, AppState>, pid: u32, force: bool) -> Result<KillResult, String> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    ports::kill_process(&mut sys, pid, force)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let sys = System::new().expect("failed to initialise sysinfo");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            sys: Mutex::new(sys),
        })
        .invoke_handler(tauri::generate_handler![list_ports, kill_process])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
