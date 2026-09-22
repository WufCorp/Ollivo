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

/// Понятная причина вместо «error sending request» и английских текстов плагина.
fn human(e: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match &e {
        // Файла с описанием версии нет: в этот канал ещё ничего не выпускали.
        Error::ReleaseNotFound => NO_RELEASES.into(),
        Error::Reqwest(r) if r.status().map(|s| s.as_u16()) == Some(404) => NO_RELEASES.into(),
        Error::Reqwest(r) if r.is_connect() || r.is_timeout() || r.is_request() => {
            "не получилось связаться с сервером обновлений — проверьте интернет".into()
        }
        Error::Reqwest(r) => match r.status() {
            Some(s) => format!("сервер обновлений ответил ошибкой {}", s.as_u16()),
            None => format!("сервер обновлений недоступен: {r}"),
        },
        _ => e.to_string(),
    }
}

const NO_RELEASES: &str = "в этом канале пока нет выпущенных версий";

#[cfg(test)]
mod tests {
    use super::*;

    /// Настоящая проверка выпущенного обновления: берём описание из S3, скачиваем
    /// установщик и сверяем подпись тем же ключом, что зашит в программе.
    /// `cargo test update::tests::published_update_real -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn published_update_real() {
        use base64::Engine;
        let manifest: serde_json::Value = reqwest::get(endpoint("beta")).await.unwrap().json().await.unwrap();
        let win = &manifest["platforms"]["windows-x86_64"];
        println!("версия {}, файл {}", manifest["version"], win["url"]);

        let setup = reqwest::get(win["url"].as_str().unwrap()).await.unwrap().bytes().await.unwrap();
        assert!(setup.len() > 1_000_000, "установщик подозрительно мал: {} байт", setup.len());

        let b64 = base64::engine::general_purpose::STANDARD;
        let key = minisign_verify::PublicKey::decode(
            &String::from_utf8(b64.decode(PUBKEY).unwrap()).unwrap(),
        )
        .unwrap();
        let sig = minisign_verify::Signature::decode(
            &String::from_utf8(b64.decode(win["signature"].as_str().unwrap()).unwrap()).unwrap(),
        )
        .unwrap();
        key.verify(&setup, &sig, true).expect("подпись обновления не сходится с ключом программы");

        // Подпись привязана к версии (--app-version), её видно в доверенном комментарии.
        let version = manifest["version"].as_str().unwrap();
        assert!(sig.trusted_comment().contains(&format!("version:{version}")), "{}", sig.trusted_comment());
    }

    /// Публичная половина ключа обновлений — та же, что в `tauri.conf.json`.
    const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE0MTU3QjlFMTU2Qzk0RTUKUldUbGxHd1ZubnNWRk8yakxFSEFLZWtRQXp1Y3JkczFrMWxRZ0t5YlFxVUt0b01UdXUxRlFvM0EK";

    #[test]
    fn channel_picks_file() {
        assert!(endpoint("stable").ends_with("/ollivo/updates/latest.json"));
        assert!(endpoint("beta").ends_with("/ollivo/updates/latest-beta.json"));
        // Неизвестный канал — как стабильный, а не ошибка.
        assert_eq!(endpoint("что-то"), endpoint("stable"));
    }
}
