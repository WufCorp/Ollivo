//! Настройки программы: `settings.json` в папке настроек (`%APPDATA%\ru.ollivo.app`).
//! Секретов здесь нет — пароль прокси лежит в диспетчере учётных данных Windows.

use crate::hf::HfSettings;
use crate::net::ProxySettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Папка данных: движки, модели, загрузки. `None` — ещё не выбрана мастером.
    pub data_dir: Option<PathBuf>,
    pub proxy: ProxySettings,
    pub hf: HfSettings,
    /// Мастер первого запуска пройден.
    pub setup_done: bool,
    pub updates: UpdateSettings,
}

/// Обновления программы. Выключенная проверка = ни одного сетевого запроса.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    pub auto_check: bool,
    /// `stable` — всем, `beta` — свежее и рискованнее.
    pub channel: String,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { auto_check: true, channel: "stable".into() }
    }
}

pub struct Store {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl Store {
    /// Битый или отсутствующий файл — настройки по умолчанию, программа всё равно стартует.
    pub fn open(path: PathBuf) -> Self {
        let current = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { path, current: Mutex::new(current) }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().unwrap().clone()
    }

    pub fn set(&self, s: Settings) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Через временный файл: при сбое посреди записи старые настройки не пропадут.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&s).unwrap())?;
        std::fs::rename(&tmp, &self.path)?;
        *self.current.lock().unwrap() = s;
        Ok(())
    }
}

/// Папка данных по умолчанию: `<диск с наибольшим свободным местом>:\Ollivo`.
pub fn suggest_data_dir(disks: &[crate::hardware::Disk]) -> PathBuf {
    disks
        .iter()
        .max_by_key(|d| d.free)
        .map(|d| Path::new(&d.mount).join("Ollivo"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Ollivo"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Disk;

    #[test]
    fn roundtrip_and_broken_file() {
        let dir = std::env::temp_dir().join(format!("ollivo-set-{}", std::process::id()));
        let path = dir.join("settings.json");
        let store = Store::open(path.clone());
        let mut s = store.get();
        s.proxy.enabled = true;
        s.proxy.host = "10.0.0.1".into();
        store.set(s).unwrap();
        assert_eq!(Store::open(path.clone()).get().proxy.host, "10.0.0.1");

        std::fs::write(&path, b"{broken").unwrap();
        assert!(!Store::open(path).get().proxy.enabled);
    }

    #[test]
    fn suggests_disk_with_most_space() {
        let disks = vec![
            Disk { mount: r"C:\".into(), total: 250, free: 9 },
            Disk { mount: r"D:\".into(), total: 2000, free: 136 },
        ];
        assert_eq!(suggest_data_dir(&disks), PathBuf::from(r"D:\Ollivo"));
    }
}
