mod download;
mod engines;
mod hardware;
mod hf;
mod manifest;
mod llm;
mod net;
mod process;
mod settings;
mod update;
mod setup;
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
    /// Все движки — в одном Job Object: закроется Ollivo — закроются и они.
    supervisor: process::Supervisor,
    /// Найденное обновление программы ждёт согласия пользователя.
    update: tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>,
    /// Запущенная текстовая модель (одна за раз).
    llm: tokio::sync::Mutex<Option<llm::Llm>>,
    /// Отмена текущей загрузки модели: `llm_start` держит `llm` всё время загрузки,
    /// поэтому остановка и новый запуск сначала отменяют её через этот токен.
    llm_loading: Mutex<CancellationToken>,
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

/// Загрузчик по настройкам: прокси + токен HF (только для хоста HF/зеркала).
fn build_downloader(s: &settings::Settings) -> Result<download::Downloader, String> {
    let proxy = s.proxy.reqwest(&net::load_password())?;
    Ok(download::Downloader::with_proxy(proxy).with_bearer(s.hf.token_hosts(), hf::load_token()))
}

type CoreState<'a> = State<'a, Arc<Core>>;

#[tauri::command]
async fn hardware_info() -> hardware::Hardware {
    tauri::async_runtime::spawn_blocking(hardware::detect).await.expect("detect")
}

#[derive(serde::Serialize)]
struct SettingsView {
    settings: settings::Settings,
    /// Пароль прокси и токен HF сохранены в диспетчере учётных данных (сами не отдаём).
    proxy_has_password: bool,
    hf_has_token: bool,
    data_dir: PathBuf,
}

#[tauri::command]
fn settings_get(core: CoreState<'_>) -> SettingsView {
    SettingsView {
        settings: core.settings.get(),
        proxy_has_password: !net::load_password().is_empty(),
        hf_has_token: !hf::load_token().is_empty(),
        data_dir: core.data_dir(),
    }
}

/// Секреты: `None` — не менять сохранённый, `""` — удалить.
#[tauri::command]
fn settings_save(
    core: CoreState<'_>,
    settings: settings::Settings,
    proxy_password: Option<String>,
    hf_token: Option<String>,
) -> Result<(), String> {
    settings.hf.base()?;
    if let Some(p) = &proxy_password {
        net::secret::store(net::secret::PROXY_PASSWORD, p)?;
    }
    if let Some(t) = &hf_token {
        net::secret::store(net::secret::HF_TOKEN, t.trim())?;
    }
    let downloader = build_downloader(&settings)?;
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

/// Проверка токена HF через текущие прокси и зеркало. `token: None` — сохранённый.
#[tauri::command]
async fn hf_check_token(
    core: CoreState<'_>,
    hf: hf::HfSettings,
    token: Option<String>,
) -> Result<hf::TokenCheck, String> {
    let base = hf.base()?;
    let token = token.map(|t| t.trim().to_string()).unwrap_or_else(hf::load_token);
    let dl = core.downloader();
    Ok(hf::check_token(dl.client(), &base, &token).await)
}

#[derive(serde::Serialize)]
struct SetupInfo {
    hardware: hardware::Hardware,
    checks: Vec<setup::Check>,
    disks: Vec<setup::DiskChoice>,
    setup_done: bool,
}

#[tauri::command]
async fn setup_check(core: CoreState<'_>) -> Result<SetupInfo, String> {
    let hw = tauri::async_runtime::spawn_blocking(hardware::detect).await.map_err(|e| e.to_string())?;
    Ok(SetupInfo {
        checks: setup::checks(&hw),
        disks: setup::disk_choices(&hw.disks),
        hardware: hw,
        setup_done: core.settings.get().setup_done,
    })
}

#[tauri::command]
fn setup_choose_dir(core: CoreState<'_>, path: PathBuf) -> Result<(), String> {
    setup::prepare_dir(&path)?;
    let mut s = core.settings.get();
    s.data_dir = Some(path);
    core.settings.set(s).map_err(|e| e.to_string())
}

#[tauri::command]
fn setup_finish(core: CoreState<'_>) -> Result<(), String> {
    let mut s = core.settings.get();
    s.setup_done = true;
    core.settings.set(s).map_err(|e| e.to_string())
}

/// Скачивает официальный установщик VC++ и запускает его (Windows спросит права администратора).
#[tauri::command]
async fn vcredist_install(core: CoreState<'_>) -> Result<(), String> {
    let dest = std::env::temp_dir().join("ollivo-vc_redist.x64.exe");
    let _ = std::fs::remove_file(&dest);
    let req = download::Request {
        urls: vec![setup::VC_REDIST_URL.into()],
        dest: dest.clone(),
        sha256: None,
        connections: 4,
        chunk_size: 8 << 20,
    };
    core.downloader()
        .download(&req, &CancellationToken::new(), &|_| {})
        .await
        .map_err(|e| format!("не скачался установщик Microsoft: {e}"))?;
    let exe = dest.clone();
    let res = tauri::async_runtime::spawn_blocking(move || setup::run_vc_redist(&exe))
        .await
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&dest);
    res?;
    if !setup::has_vc_runtime() {
        return Err("установщик отработал, но библиотек всё ещё нет — перезагрузите компьютер".into());
    }
    Ok(())
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
    /// Вид ошибки для интерфейса: `net` | `disk` | `broken` | `other`.
    kind: Option<&'static str>,
    result: Option<T>,
}

/// Вид ошибки загрузки: интерфейс решает по нему, что сказать и что предложить.
fn download_kind(e: &download::Error) -> &'static str {
    match e {
        download::Error::Net(_) => "net",
        download::Error::Status(s) if *s >= 500 || *s == 429 => "net",
        download::Error::Io(_) => "disk",
        download::Error::Hash { .. } => "broken",
        _ => "other",
    }
}

/// Итог установки или починки движка для события `engine://finished`.
fn engine_outcome<T>(res: Result<T, engines::Error>) -> (Option<String>, Option<&'static str>, Option<T>) {
    match res {
        Ok(done) => (None, None, Some(done)),
        Err(engines::Error::Download(download::Error::Cancelled)) => (Some("paused".into()), None, None),
        Err(e) => {
            let kind = match &e {
                engines::Error::Download(d) => download_kind(d),
                engines::Error::Io(_) => "disk",
                engines::Error::Unpack(..) | engines::Error::NoExe(_) => "broken",
            };
            (Some(e.to_string()), Some(kind), None)
        }
    }
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
        let (error, kind) = match res {
            Ok(()) => (None, None),
            Err(download::Error::Cancelled) => (Some("paused".into()), None),
            Err(e) => (Some(e.to_string()), Some(download_kind(&e))),
        };
        let _ = app.emit("download://finished", Finished::<()> { id, error, kind, result: None });
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
    let pick = engine.pick(hw.cuda_build, setup::has_vulkan(), None);
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
    let chosen = engine
        .pick(hw.cuda_build, setup::has_vulkan(), build)
        .ok_or("нет сборки для этого ПК: нужна видеокарта NVIDIA или Vulkan")?
        .clone();
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
        let (error, kind, result) = engine_outcome(res);
        let _ = app.emit("engine://finished", Finished { id, error, kind, result });
    });
    Ok(())
}

/// «Починить» движок в фоне: сверка файлов (`engine://progress`, этап `verify`),
/// при поломке — переустановка. Итог — `engine://finished` с полями `broken` и `reinstalled`.
/// Чинит ту сборку текущей версии, что стоит; не стоит ни одной — ставит сборку для этого ПК.
#[tauri::command]
async fn engine_repair(app: AppHandle, core: CoreState<'_>, id: String) -> Result<(), String> {
    let engine = core.manifest.engine(&id).ok_or("нет такого движка")?.clone();
    let root = core.data_dir();
    let current = engines::installed(&root, &id).into_iter().find(|i| i.version == engine.version);
    let chosen = match current.and_then(|i| engine.builds.iter().find(|b| b.build == i.build)) {
        Some(b) => b.clone(),
        None => {
            let hw = tauri::async_runtime::spawn_blocking(hardware::detect).await.map_err(|e| e.to_string())?;
            engine
                .pick(hw.cuda_build, setup::has_vulkan(), None)
                .ok_or("нет сборки для этого ПК: нужна видеокарта NVIDIA или Vulkan")?
                .clone()
        }
    };
    // Движок чата держит свои файлы открытыми — перед починкой останавливаем.
    if id == "llama.cpp" {
        core.llm_loading.lock().unwrap().cancel();
        if let Some(l) = core.llm.lock().await.take() {
            l.handle.stop().await;
            emit_llm(&app, LlmState::of("stopped"));
        }
    }
    let task = format!("engine:{id}");
    let cancel = core.start(&task)?;
    let core = core.inner().clone();
    tauri::async_runtime::spawn(async move {
        let (progress_app, progress_id) = (app.clone(), id.clone());
        let on_progress = move |progress| {
            let _ = progress_app.emit("engine://progress", EngineProgress { id: progress_id.clone(), progress });
        };
        let res = engines::repair(&core.downloader(), &root, &engine, &chosen, &cancel, &on_progress).await;
        core.finish(&task);
        let (error, kind, result) = engine_outcome(res);
        let _ = app.emit("engine://finished", Finished { id, error, kind, result });
    });
    Ok(())
}

#[derive(Clone, serde::Serialize)]
struct LlmState {
    /// `starting` | `ready` | `stopped` | `crashed`.
    state: &'static str,
    model: Option<PathBuf>,
    port: Option<u16>,
    started_in: Option<f64>,
    error: Option<String>,
}

impl LlmState {
    fn of(state: &'static str) -> Self {
        Self { state, model: None, port: None, started_in: None, error: None }
    }
}

fn emit_llm(app: &AppHandle, s: LlmState) {
    let _ = app.emit("llm://state", s);
}

#[tauri::command]
async fn llm_status(core: CoreState<'_>) -> Result<LlmState, String> {
    // Занято — значит, идёт загрузка (llm_start держит слот до конца), ждать её не будем.
    let Ok(slot) = core.llm.try_lock() else { return Ok(LlmState::of("starting")) };
    Ok(match slot.as_ref() {
        Some(l) => LlmState {
            state: "ready",
            model: Some(l.model.clone()),
            port: Some(l.port),
            started_in: Some(l.started_in.as_secs_f64()),
            error: None,
        },
        None => LlmState::of("stopped"),
    })
}

/// Запускает модель; уже запущенная останавливается. Итог — событием `llm://state`.
#[tauri::command]
async fn llm_start(app: AppHandle, core: CoreState<'_>, config: llm::Config) -> Result<(), String> {
    let root = core.data_dir();
    let engine = engines::installed(&root, "llama.cpp")
        .into_iter()
        .find(|i| core.manifest.engine("llama.cpp").is_some_and(|e| e.version == i.version))
        .ok_or("движок чата не установлен")?;
    let core = core.inner().clone();
    let cancel = CancellationToken::new();
    std::mem::replace(&mut *core.llm_loading.lock().unwrap(), cancel.clone()).cancel();
    tauri::async_runtime::spawn(async move {
        let mut slot = core.llm.lock().await;
        if cancel.is_cancelled() {
            return; // пока ждали, запустили другую модель или нажали «Остановить»
        }
        if let Some(old) = slot.take() {
            old.handle.stop().await;
        }
        emit_llm(&app, LlmState { model: Some(config.model.clone()), ..LlmState::of("starting") });
        match llm::start(&core.supervisor, &engine, &config, &root.join("logs"), &cancel).await {
            Ok(l) => {
                emit_llm(
                    &app,
                    LlmState {
                        state: "ready",
                        model: Some(l.model.clone()),
                        port: Some(l.port),
                        started_in: Some(l.started_in.as_secs_f64()),
                        error: None,
                    },
                );
                // Сторож: движок упал сам — сообщаем с хвостом лога.
                let (handle, app2, core2) = (l.handle.clone(), app.clone(), core.clone());
                *slot = Some(l);
                tauri::async_runtime::spawn(async move {
                    let exit = handle.wait().await;
                    if exit.by_us {
                        return;
                    }
                    let mut slot = core2.llm.lock().await;
                    if slot.as_ref().is_some_and(|l| l.handle.pid == handle.pid) {
                        *slot = None;
                    }
                    let tail = process::log_tail(&handle.log, 15);
                    emit_llm(
                        &app2,
                        LlmState { error: Some(format!("движок чата упал (код {:?})
{tail}", exit.code)), ..LlmState::of("crashed") },
                    );
                });
            }
            // Отменили: итог сообщит тот, кто отменил (llm_stop или новый llm_start).
            Err(e) if e == llm::CANCELLED => {}
            Err(e) => emit_llm(&app, LlmState { error: Some(e), ..LlmState::of("crashed") }),
        }
    });
    Ok(())
}

#[tauri::command]
async fn llm_stop(app: AppHandle, core: CoreState<'_>) -> Result<(), String> {
    core.llm_loading.lock().unwrap().cancel();
    if let Some(l) = core.llm.lock().await.take() {
        l.handle.stop().await;
    }
    emit_llm(&app, LlmState::of("stopped"));
    Ok(())
}

#[tauri::command]
async fn llm_ask(core: CoreState<'_>, prompt: String) -> Result<llm::Answer, String> {
    let port = match core.llm.try_lock() {
        Ok(slot) => slot.as_ref().map(|l| l.port).ok_or("модель не запущена")?,
        Err(_) => return Err("модель ещё загружается".into()),
    };
    llm::ask(port, &prompt, 256).await
}

/// Прокси из настроек — чтобы обновления ходили тем же путём, что и загрузки.
fn proxy_url(s: &settings::Settings) -> Option<url::Url> {
    s.proxy.url(&net::load_password()).ok().flatten().and_then(|u| u.parse().ok())
}

/// Ищет обновление в выбранном канале. `None` — установлена свежая версия.
#[tauri::command]
async fn update_check(app: AppHandle, core: CoreState<'_>) -> Result<Option<update::Available>, String> {
    let s = core.settings.get();
    let core = core.inner().clone();
    let found = update::check(&app, &s.updates.channel, proxy_url(&s)).await?;
    let info = found.as_ref().map(|u| update::Available {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    });
    *core.update.lock().await = found;
    Ok(info)
}

#[derive(Clone, serde::Serialize)]
struct UpdateProgress {
    done: u64,
    total: Option<u64>,
}

/// Ставит найденное обновление и перезапускает программу.
/// Прогресс — `update://progress`, ошибка — `update://failed`.
#[tauri::command]
async fn update_install(app: AppHandle, core: CoreState<'_>) -> Result<(), String> {
    let core = core.inner().clone();
    let found = core.update.lock().await.take().ok_or("обновление не найдено")?;
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let on_progress = move |done, total| {
            let _ = app2.emit("update://progress", UpdateProgress { done, total });
        };
        match update::install(&found, on_progress).await {
            // Движки закроются сами: они в Job Object, привязанном к этому процессу.
            Ok(()) => app.restart(),
            Err(e) => {
                let _ = app.emit("update://failed", e);
            }
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            let settings = settings::Store::open(path);
            // Кривые настройки прокси не должны мешать запуску: тогда без прокси,
            // а ошибку пользователь увидит при проверке в настройках.
            let downloader = build_downloader(&settings.get()).unwrap_or_default();
            app.manage(Arc::new(Core {
                settings,
                manifest: manifest::Manifest::bundled(),
                downloader: RwLock::new(Arc::new(downloader)),
                running: Mutex::new(HashMap::new()),
                supervisor: process::Supervisor::new(),
                update: tokio::sync::Mutex::new(None),
                llm: tokio::sync::Mutex::new(None),
                llm_loading: Mutex::new(CancellationToken::new()),
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hardware_info,
            settings_get,
            settings_save,
            proxy_test,
            hf_check_token,
            setup_check,
            setup_choose_dir,
            setup_finish,
            vcredist_install,
            download_start,
            task_pause,
            engine_status,
            engine_install,
            engine_repair,
            llm_status,
            llm_start,
            llm_stop,
            llm_ask,
            update_check,
            update_install,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
