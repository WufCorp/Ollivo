//! Обновления самой Ollivo: проверка, загрузка, установка.
//!
//! Файл с описанием новой версии и установщик лежат в S3 (Timeweb), подписаны
//! ключом обновлений (`~/.tauri/ollivo.key`, публичная половина — в `tauri.conf.json`).
//! Канал `beta` — отдельный файл `latest-beta.json` в том же месте.
//!
//! Проверка выключается в настройках: тогда программа не делает ни одного запроса сама.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

/// Адрес описания версии для канала.
pub fn endpoint(channel: &str) -> String {
    let file = if channel == "beta" { "latest-beta.json" } else { "latest.json" };
    format!("https://s3.twcstorage.ru/prisma-prava/ollivo/updates/{file}")
}

#[derive(Debug, Clone, Serialize)]
pub struct Available {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// Ищет обновление в выбранном канале. `Ok(None)` — установлена свежая версия.
pub async fn check(app: &AppHandle, channel: &str, proxy: Option<url::Url>) -> Result<Option<Update>, String> {
    let url = endpoint(channel).parse().map_err(|e| format!("неверный адрес обновлений: {e}"))?;
    let mut builder = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| format!("неверный адрес обновлений: {e}"))?;
    if let Some(p) = proxy {
        builder = builder.proxy(p);
    }
    let updater = builder.build().map_err(|e| e.to_string())?;
    updater.check().await.map_err(human)
}

/// Скачивает и ставит обновление; `on_progress` — скачано и всего байт (всего может быть неизвестно).
pub async fn install(
    update: &Update,
    on_progress: impl Fn(u64, Option<u64>) + Send + Sync,
) -> Result<(), String> {
    let mut done = 0u64;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                on_progress(done, total);
            },
            || {},
        )
        .await
        .map_err(human)
}

/// Понятная причина вместо «error sending request».
fn human(e: tauri_plugin_updater::Error) -> String {
    match &e {
        tauri_plugin_updater::Error::Reqwest(_) => {
            "не получилось связаться с сервером обновлений — проверьте интернет".into()
        }
        _ => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_picks_file() {
        assert!(endpoint("stable").ends_with("/ollivo/updates/latest.json"));
        assert!(endpoint("beta").ends_with("/ollivo/updates/latest-beta.json"));
        // Неизвестный канал — как стабильный, а не ошибка.
        assert_eq!(endpoint("что-то"), endpoint("stable"));
    }
}
