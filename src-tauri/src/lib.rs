mod download;
mod hardware;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Downloads {
    downloader: download::Downloader,
    running: Mutex<HashMap<String, CancellationToken>>,
}

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    id: String,
    #[serde(flatten)]
    progress: download::Progress,
}

#[derive(Clone, serde::Serialize)]
struct DownloadFinished {
    id: String,
    /// `None` — успешно; `Some("paused")` — поставлено на паузу.
    error: Option<String>,
}

#[tauri::command]
async fn hardware_info() -> hardware::Hardware {
    tauri::async_runtime::spawn_blocking(hardware::detect).await.expect("detect")
}

/// Запускает загрузку в фоне. Прогресс — событие `download://progress`,
/// итог — `download://finished`.
#[tauri::command]
fn download_start(
    app: AppHandle,
    state: State<'_, Arc<Downloads>>,
    id: String,
    request: download::Request,
) -> Result<(), String> {
    let cancel = CancellationToken::new();
    {
        let mut running = state.running.lock().unwrap();
        if running.contains_key(&id) {
            return Err("эта загрузка уже идёт".into());
        }
        running.insert(id.clone(), cancel.clone());
    }
    let downloads = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let progress_app = app.clone();
        let progress_id = id.clone();
        let on_progress = move |progress| {
            let _ = progress_app
                .emit("download://progress", DownloadProgress { id: progress_id.clone(), progress });
        };
        let res = downloads.downloader.download(&request, &cancel, &on_progress).await;
        downloads.running.lock().unwrap().remove(&id);
        let error = match res {
            Ok(()) => None,
            Err(download::Error::Cancelled) => Some("paused".into()),
            Err(e) => Some(e.to_string()),
        };
        let _ = app.emit("download://finished", DownloadFinished { id, error });
    });
    Ok(())
}

/// Пауза: недокачанный файл остаётся, `download_start` продолжит с того же места.
#[tauri::command]
fn download_pause(state: State<'_, Arc<Downloads>>, id: String) {
    if let Some(cancel) = state.running.lock().unwrap().get(&id) {
        cancel.cancel();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(Downloads::default()))
        .invoke_handler(tauri::generate_handler![hardware_info, download_start, download_pause])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
