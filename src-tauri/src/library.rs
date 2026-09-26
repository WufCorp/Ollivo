//! Библиотека моделей: список файлов, которые человек добавил.
//!
//! Файлы не копируются и не переносятся — храним путь. Разбор заголовка (`probe`)
//! медленный только для больших GGUF, поэтому результат кешируется в `models.json`
//! рядом с настройками; заново читаем, если файл изменился (размер или дата).
//! «Светофор» в кеш не попадает — он зависит от свободной памяти и считается каждый раз.

use crate::hardware::Hardware;
use crate::probe::{self, Kind, ModelInfo, Verdict};
use crate::scan::Found;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Запись в `models.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub size: u64,
    /// Дата изменения файла, unix-секунды: по ней ловим подмену файла.
    pub mtime: u64,
    /// Когда добавили, unix-секунды.
    pub added: u64,
    /// Понятное имя, если по имени файла не разобрать (у Ollama файлы — по хешу).
    #[serde(default)]
    pub title: Option<String>,
    /// Откуда скачали (`автор/репозиторий` на HF) — для ссылки на условия модели.
    #[serde(default)]
    pub repo: Option<String>,
    /// Лицензия со страницы модели (`apache-2.0`, `other`…). В заголовке файла её
    /// часто нет, а на HF она есть почти всегда — поэтому запоминаем при скачивании.
    #[serde(default)]
    pub license: Option<String>,
    pub info: ModelInfo,
}

/// Что известно о файле кроме него самого: имя из поиска, страница на HF, лицензия.
#[derive(Debug, Clone, Default)]
pub struct Source {
    pub title: Option<String>,
    pub repo: Option<String>,
    pub license: Option<String>,
}

/// Модель для окна: запись плюс всё, что считается на лету.
#[derive(Debug, Clone, Serialize)]
pub struct Model {
    #[serde(flatten)]
    pub entry: Entry,
    /// Имя файла — его и показываем в списке.
    pub file: String,
    pub kind_ru: &'static str,
    /// Файла нет на месте (диск отключили, файл удалили).
    pub missing: bool,
    /// `None`, если файла нет — оценивать нечего.
    pub verdict: Option<Verdict>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Размер и дата изменения файла; `None` — файла нет.
fn stat(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some((m.len(), mtime))
}

/// Windows не различает регистр путей: `D:\M\a.gguf` и `d:\m\A.GGUF` — один файл.
fn same_file(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
}

/// Итог поиска моделей по папкам.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub added: usize,
    /// Уже были в списке.
    pub already: usize,
    /// Не наш формат или файл не открылся.
    pub skipped: usize,
}

pub struct Library {
    path: PathBuf,
    entries: Mutex<Vec<Entry>>,
}

impl Library {
    /// Битый или отсутствующий файл — пустая библиотека, программа всё равно стартует.
    pub fn open(path: PathBuf) -> Self {
        let entries = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { path, entries: Mutex::new(entries) }
    }

    fn save(&self, entries: &[Entry]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Через временный файл: при сбое посреди записи старый список не пропадёт.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(entries).unwrap())?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Добавляет файл: читает заголовок и запоминает. Уже добавленный — обновляет.
    /// Ошибка — только если файл не открылся или формат не наш.
    pub fn add(&self, path: &Path, src: Source) -> Result<Entry, String> {
        let (size, mtime) = stat(path).ok_or_else(|| format!("файл не найден: {}", path.display()))?;
        let info = probe::probe(path).map_err(|e| format!("{e:#}"))?;
        let mut entries = self.entries.lock().unwrap();
        let old = entries.iter().find(|e| same_file(&e.path, path));
        let added = old.map_or_else(now, |e| e.added);
        // Имя, страницу и лицензию, известные с прошлого раза, не теряем,
        // если файл добавляют ещё раз вручную.
        let title = src.title.or_else(|| old.and_then(|e| e.title.clone()));
        let repo = src.repo.or_else(|| old.and_then(|e| e.repo.clone()));
        let license = src.license.or_else(|| old.and_then(|e| e.license.clone()));
        let entry = Entry { path: path.to_path_buf(), size, mtime, added, title, repo, license, info };
        entries.retain(|e| !same_file(&e.path, path));
        entries.push(entry.clone());
        let list = entries.clone();
        drop(entries);
        self.save(&list).map_err(|e| format!("не записать список моделей: {e}"))?;
        Ok(entry)
    }

    /// Добавляет найденное поиском. Уже известные файлы не перечитываем — их
    /// может быть много, а заголовок большого GGUF читается не мгновенно.
    pub fn add_found(&self, found: Vec<Found>) -> Report {
        let mut r = Report::default();
        for f in found {
            let known = self
                .entries
                .lock()
                .unwrap()
                .iter()
                .any(|e| same_file(&e.path, &f.path) && stat(&f.path) == Some((e.size, e.mtime)));
            if known {
                r.already += 1;
            } else if self.add(&f.path, Source { title: f.title, ..Source::default() }).is_ok() {
                r.added += 1;
            } else {
                r.skipped += 1;
            }
        }
        r
    }

    pub fn remove(&self, path: &Path) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|e| !same_file(&e.path, path));
        let list = entries.clone();
        drop(entries);
        self.save(&list).map_err(|e| format!("не записать список моделей: {e}"))
    }

    /// Список для окна: пропавшие файлы помечены, изменённые — перечитаны.
    pub fn list(&self, hw: &Hardware) -> Vec<Model> {
        let entries = self.entries.lock().unwrap().clone();
        let mut changed = false;
        let mut fresh = Vec::with_capacity(entries.len());
        for mut e in entries {
            let missing = match stat(&e.path) {
                None => true,
                Some((size, mtime)) => {
                    if (size, mtime) != (e.size, e.mtime) {
                        // Файл подменили или дозакачали: перечитываем заголовок.
                        if let Ok(info) = probe::probe(&e.path) {
                            e = Entry { size, mtime, info, ..e };
                            changed = true;
                        }
                    }
                    false
                }
            };
            fresh.push(Model {
                file: e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                kind_ru: e.info.kind.ru(),
                verdict: (!missing).then(|| probe::assess(&e.info, hw)),
                missing,
                entry: e,
            });
        }
        if changed {
            let list: Vec<Entry> = fresh.iter().map(|m| m.entry.clone()).collect();
            *self.entries.lock().unwrap() = list.clone();
            let _ = self.save(&list);
        }
        // Сначала то, что запускается в чате, потом остальное; внутри — новые сверху.
        fresh.sort_by_key(|m| (m.entry.info.kind != Kind::Llm, std::cmp::Reverse(m.entry.added)));
        fresh
    }

    /// Запись из библиотеки, если файл на месте.
    pub fn find(&self, path: &Path) -> Option<Entry> {
        let e = self.entries.lock().unwrap().iter().find(|e| same_file(&e.path, path)).cloned()?;
        stat(&e.path).map(|_| e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ollivo-lib-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Минимальный GGUF: заголовок без тензоров и метаданных.
    fn write_gguf(path: &Path) {
        let mut b = Vec::new();
        b.extend(b"GGUF");
        b.extend(3u32.to_le_bytes()); // версия
        b.extend(0u64.to_le_bytes()); // тензоров
        b.extend(0u64.to_le_bytes()); // ключей
        std::fs::write(path, b).unwrap();
    }

    fn hw() -> Hardware {
        crate::hardware::detect()
    }

    #[test]
    fn add_list_remove() {
        let dir = tmp("add");
        let model = dir.join("model.gguf");
        write_gguf(&model);
        let lib = Library::open(dir.join("models.json"));
        lib.add(&model, Source::default()).unwrap();
        // Повторное добавление не плодит дубли.
        lib.add(&model, Source::default()).unwrap();
        let list = lib.list(&hw());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file, "model.gguf");
        assert!(!list[0].missing);

        // Список переживает перезапуск.
        let again = Library::open(dir.join("models.json"));
        assert_eq!(again.list(&hw()).len(), 1);

        again.remove(&model).unwrap();
        assert!(again.list(&hw()).is_empty());
        assert!(Library::open(dir.join("models.json")).list(&hw()).is_empty());
    }

    /// Лицензия из каталога не теряется, когда тот же файл добавляют руками.
    #[test]
    fn license_from_catalog_survives_manual_add() {
        let dir = tmp("license");
        let model = dir.join("model.gguf");
        write_gguf(&model);
        let lib = Library::open(dir.join("models.json"));
        let src = Source { title: None, repo: Some("unsloth/X-GGUF".into()), license: Some("apache-2.0".into()) };
        lib.add(&model, src).unwrap();
        lib.add(&model, Source::default()).unwrap();
        let m = &Library::open(dir.join("models.json")).list(&hw())[0];
        assert_eq!(m.entry.license.as_deref(), Some("apache-2.0"));
        assert_eq!(m.entry.repo.as_deref(), Some("unsloth/X-GGUF"));
    }

    /// Путь в другом регистре — тот же файл (Windows).
    #[test]
    fn same_path_other_case() {
        let dir = tmp("case");
        let model = dir.join("model.gguf");
        write_gguf(&model);
        let lib = Library::open(dir.join("models.json"));
        lib.add(&model, Source::default()).unwrap();
        lib.add(&dir.join("MODEL.GGUF"), Source::default()).unwrap();
        assert_eq!(lib.list(&hw()).len(), 1);
    }

    #[test]
    fn missing_file_is_marked_not_dropped() {
        let dir = tmp("missing");
        let model = dir.join("model.gguf");
        write_gguf(&model);
        let lib = Library::open(dir.join("models.json"));
        lib.add(&model, Source::default()).unwrap();
        std::fs::remove_file(&model).unwrap();
        let list = lib.list(&hw());
        assert_eq!(list.len(), 1);
        assert!(list[0].missing && list[0].verdict.is_none());
        assert!(lib.find(&model).is_none());
    }

    #[test]
    fn unknown_format_is_an_error() {
        let dir = tmp("junk");
        let junk = dir.join("readme.txt");
        std::fs::write(&junk, "не модель").unwrap();
        let lib = Library::open(dir.join("models.json"));
        assert!(lib.add(&junk, Source::default()).is_err());
        assert!(lib.list(&hw()).is_empty());
    }
}
