mod download;
mod engines;
mod hardware;
mod manifest;
mod net;
mod settings;
#[cfg(test)]
mod testserver;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;

/// Общее состояние ядра.
struct Core {
    settings: settings::Store,
    manifest: manifest::Manifest,
    /// Пересобирается при смене прокси; идущие загрузки доживают на старом клиенте.
    downloader: RwLock<Arc<download::Downloader>>,
    running: Mutex<HashMap<String, CancellationToken>>,
}

impl Core {
    fn downloader(&self) -> Arc<download::Downloader> {
        self.downloader.read().unwrap().clone()
    }

    fn data_dir(&self) -> PathBuf {
        self.settings
            .get()
            .data_dir
            .unwrap_or_else(|| settings::suggest_data_dir(&hardware::detect_disks()))
    }

    /// Регистрирует фоновую задачу; ошибка, если задача с таким id уже идёт.
    fn start(&self, id: &str) -> Result<CancellationToken, String> {
        let mut running = self.running.lock().unwrap();
        if running.contains_key(id) {
            return Err("уже идёт".into());
        }
        let cancel = CancellationToken::new();
        running.insert(id.to_string(), cancel.clone());
        Ok(cancel)
    }

    fn finish(&self, id: &str) {
        self.running.lock().unwrap().remove(id);
    }
}

fn build_downloader(proxy: &net::ProxySettings) -> Result<download::Downloader, String> {
    Ok(download::Downloader::with_proxy(proxy.reqwest(&net::load_password())?))
}

type CoreState<'a> = State<'a, Arc<Core>>;

#[tauri::command]
async fn hardware_info() -> hardware::Hardware {
    tauri::async_runtime::spawn_blocking(hardware::detect).await.expect("detect")
}

#[derive(serde::Serialize)]
struct SettingsView {
    settings: settings::Settings,
    /// Пароль прокси сохранён в диспетчере учётных данных (сам пароль не отдаём).
    proxy_has_password: bool,
    data_dir: PathBuf,
}

#[tauri::command]
fn settings_get(core: CoreState<'_>) -> SettingsView {
    SettingsView {
        settings: core.settings.get(),
        proxy_has_password: net::has_password(),
        data_dir: core.data_dir(),
    }
}

/// `proxy_password`: `None` — не менять сохранённый, `""` — удалить.
#[tauri::command]
fn settings_save(
    core: CoreState<'_>,
    settings: settings::Settings,
    proxy_password: Option<String>,
) -> Result<(), String> {
    if let Some(p) = &proxy_password {
        net::store_password(p)?;
    }
    let downloader = build_downloader(&settings.proxy)?;
    core.settings.set(settings).map_err(|e| e.to_string())?;
    *core.downloader.write().unwrap() = Arc::new(downloader);
    Ok(())
}

/// Проверка прокси до сохранения. `password: None` — взять сохранённый.
#[tauri::command]
async fn proxy_test(proxy: net::ProxySettings, password: Option<String>) -> net::TestReport {
    let password = password.unwrap_or_else(net::load_password);
    net::test(&proxy, &password).await
}

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    id: String,
    #[serde(flatten)]
    progress: download::Progress,
}

#[derive(Clone, serde::Serialize)]
struct Finished<T: Clone> {
    id: String,
    /// `None` — успешно; `Some("paused")` — поставлено на паузу.
    error: Option<String>,
    result: Option<T>,
}

/// Запускает загрузку в фоне. Прогресс — событие `download://progress`,
/// итог — `download://finished`.
#[tauri::command]
fn download_start(
    app: AppHandle,
    core: CoreState<'_>,
    id: String,
    request: download::Request,
) -> Result<(), String> {
    let cancel = core.start(&id)?;
    let core = core.inner().clone();
    tauri::async_runtime::spawn(async move {
        let (progress_app, progress_id) = (app.clone(), id.clone());
        let on_progress = move |progress| {
            let _ = progress_app
                .emit("download://progress", DownloadProgress { id: progress_id.clone(), progress });
        };
        let res = core.downloader().download(&request, &cancel, &on_progress).await;
        core.finish(&id);
        let error = match res {
            Ok(()) => None,
            Err(download::Error::Cancelled) => Some("paused".into()),
            Err(e) => Some(e.to_string()),
        };
        let _ = app.emit("download://finished", Finished::<()> { id, error, result: None });
    });
    Ok(())
}

/// Пауза загрузки или установки: недокачанное остаётся, повторный запуск продолжит.
#[tauri::command]
fn task_pause(core: CoreState<'_>, id: String) {
    if let Some(cancel) = core.running.lock().unwrap().get(&id) {
        cancel.cancel();
    }
}

#[derive(serde::Serialize)]
struct EngineStatus {
    id: String,
    title: String,
    version: String,
    installed: Vec<engines::Installed>,
    /// Сборка, которую поставим на этот ПК, и сколько качать.
    build: Option<hardware::Build>,
    size: u64,
}

#[tauri::command]
async fn engine_status(core: CoreState<'_>, id: String) -> Result<EngineStatus, String> {
    let engine = core.manifest.engine(&id).ok_or("нет такого движка")?;
    let hw = tauri::async_runtime::spawn_blocking(hardware::detect).await.map_err(|e| e.to_string())?;
    let pick = engine.pick(hw.cuda_build, None);
    Ok(EngineStatus {
        id: engine.id.clone(),
        title: engine.title.clone(),
        version: engine.version.clone(),
        installed: engines::installed(&core.data_dir(), &id),
        build: pick.map(|b| b.build),
        size: pick.map_or(0, |b| b.size()),
    })
}

#[derive(Clone, serde::Serialize)]
struct EngineProgress {
    id: String,
    #[serde(flatten)]
    progress: engines::InstallProgress,
}

/// Установка движка в фоне: `engine://progress`, итог — `engine://finished`.
/// `build: None` — сборка по умолчанию для этого ПК.
#[tauri::command]
async fn engine_install(
    app: AppHandle,
    core: CoreState<'_>,
    id: String,
    build: Option<hardware::Build>,
) -> Result<(), String> {
    let engine = core.manifest.engine(&id).ok_or("нет такого движка")?.clone();
    let hw = tauri::async_runtime::spawn_blocking(hardware::detect).await.map_err(|e| e.to_string())?;
    let chosen = engine.pick(hw.cuda_build, build).ok_or("нет сборки для этого ПК")?.clone();
    let task = format!("engine:{id}");
    let cancel = core.start(&task)?;
    let core = core.inner().clone();
    tauri::async_runtime::spawn(async move {
        let (progress_app, progress_id) = (app.clone(), id.clone());
        let on_progress = move |progress| {
            let _ = progress_app.emit("engine://progress", EngineProgress { id: progress_id.clone(), progress });
        };
        let root = core.data_dir();
        let res = engines::install(&core.downloader(), &root, &engine, &chosen, &cancel, &on_progress).await;
        core.finish(&task);
        let (error, result) = match res {
            Ok(done) => (None, Some(done)),
            Err(engines::Error::Download(download::Error::Cancelled)) => (Some("paused".into()), None),
            Err(e) => (Some(e.to_string()), None),
        };
        let _ = app.emit("engine://finished", Finished { id, error, result });
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            let settings = settings::Store::open(path);
            // Кривые настройки прокси не должны мешать запуску: тогда без прокси,
            // а ошибку пользователь увидит при проверке в настройках.
            let downloader = build_downloader(&settings.get().proxy).unwrap_or_default();
            app.manage(Arc::new(Core {
                settings,
                manifest: manifest::Manifest::bundled(),
                downloader: RwLock::new(Arc::new(downloader)),
                running: Mutex::new(HashMap::new()),
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hardware_info,
            settings_get,
            settings_save,
            proxy_test,
            download_start,
            task_pause,
            engine_status,
            engine_install,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
