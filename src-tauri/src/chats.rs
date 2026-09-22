//! История разговоров: по файлу на разговор в `chats\` рядом с настройками.
//!
//! Пока JSON: разговоров у одного человека сотни, а не миллионы, и читать папку
//! быстрее, чем тянуть базу. Поиск по всем разговорам (фаза 2) — повод перейти на SQLite.

use crate::llm::Msg;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Длина названия, которое делаем из первого вопроса.
const TITLE_LEN: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    pub title: String,
    /// Unix-секунды.
    pub created: u64,
    pub updated: u64,
    /// Модель, на которой шёл разговор — по ней понятно, чем его продолжать.
    #[serde(default)]
    pub model: Option<PathBuf>,
    pub messages: Vec<Msg>,
}

/// Строка в списке разговоров: без самих реплик.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub updated: u64,
    pub messages: usize,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Название из первого вопроса: одна строка, не длиннее `TITLE_LEN`.
pub fn title_from(messages: &[Msg]) -> String {
    let first = messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| m.content.trim())
        .unwrap_or_default();
    let line = first.lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        return "Без названия".into();
    }
    let short: String = line.chars().take(TITLE_LEN).collect();
    if line.chars().count() > TITLE_LEN {
        format!("{}…", short.trim_end())
    } else {
        short
    }
}

/// Имя файла — сам id, поэтому в id пускаем только цифры и латиницу.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(&self, id: &str) -> Option<PathBuf> {
        safe_id(id).then(|| self.dir.join(format!("{id}.json")))
    }

    /// Разговоры, новые сверху. Битые файлы просто пропускаем.
    pub fn list(&self) -> Vec<Summary> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut all: Vec<Summary> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| read(&e.path()))
            .map(|c| Summary {
                id: c.id,
                title: c.title,
                updated: c.updated,
                messages: c.messages.len(),
            })
            .collect();
        all.sort_by_key(|c| std::cmp::Reverse(c.updated));
        all
    }

    pub fn get(&self, id: &str) -> Option<Chat> {
        read(&self.file(id)?)
    }

    /// Сохраняет разговор: без id — заводит новый. Возвращает сохранённое.
    pub fn save(&self, mut chat: Chat) -> Result<Chat, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("не создать папку разговоров: {e}"))?;
        if !safe_id(&chat.id) {
            chat.id = self.new_id();
            chat.created = now();
        }
        chat.updated = now();
        if chat.title.trim().is_empty() {
            chat.title = title_from(&chat.messages);
        }
        let path = self.file(&chat.id).ok_or("плохой номер разговора")?;
        // Через временный файл: сбой посреди записи не съест прошлый разговор.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&chat).unwrap())
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("не записать разговор: {e}"))?;
        Ok(chat)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let path = self.file(id).ok_or("плохой номер разговора")?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Уже удалён — значит, всё как просили.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("не удалить разговор: {e}")),
        }
    }

    /// Номер разговора — время создания; если такой файл уже есть, добавляем букву.
    fn new_id(&self) -> String {
        let base = now();
        for n in 0..100 {
            let id = if n == 0 { base.to_string() } else { format!("{base}-{n}") };
            if self.file(&id).is_some_and(|p| !p.exists()) {
                return id;
            }
        }
        format!("{base}-x")
    }
}

fn read(path: &Path) -> Option<Chat> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Msg {
        Msg { role: role.into(), content: content.into() }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("ollivo-chats-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::new(dir)
    }

    fn empty(messages: Vec<Msg>) -> Chat {
        Chat { id: String::new(), title: String::new(), created: 0, updated: 0, model: None, messages }
    }

    #[test]
    fn saves_names_lists_and_removes() {
        let s = store("crud");
        assert!(s.list().is_empty());

        let saved = s.save(empty(vec![msg("user", "Как сварить борщ?"), msg("assistant", "Возьмите свёклу…")])).unwrap();
        assert!(!saved.id.is_empty());
        assert_eq!(saved.title, "Как сварить борщ?");

        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].title.as_str(), list[0].messages), ("Как сварить борщ?", 2));

        // Дописали разговор — тот же файл, не второй.
        let mut again = s.get(&saved.id).unwrap();
        again.messages.push(msg("user", "А без мяса?"));
        s.save(again).unwrap();
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.get(&saved.id).unwrap().messages.len(), 3);

        s.remove(&saved.id).unwrap();
        assert!(s.list().is_empty());
        // Повторное удаление — не ошибка.
        s.remove(&saved.id).unwrap();
    }

    #[test]
    fn two_chats_newest_first() {
        let s = store("order");
        let first = s.save(empty(vec![msg("user", "Первый")])).unwrap();
        let second = s.save(empty(vec![msg("user", "Второй")])).unwrap();
        assert_ne!(first.id, second.id);
        let ids: Vec<String> = s.list().into_iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 2);
        // Сохранены в одну секунду — порядок по времени не проверить, важно, что оба на месте.
        assert!(ids.contains(&first.id) && ids.contains(&second.id));
    }

    #[test]
    fn long_question_is_cut_and_empty_one_named() {
        let long = "а".repeat(200);
        assert_eq!(title_from(&[msg("user", &long)]).chars().count(), TITLE_LEN + 1);
        assert_eq!(title_from(&[msg("user", "Строка\nи ещё строка")]), "Строка");
        assert_eq!(title_from(&[]), "Без названия");
    }

    /// Имя файла берётся из id, поэтому путь наружу не должен пролезать.
    #[test]
    fn strange_id_is_refused() {
        let s = store("evil");
        let mut chat = empty(vec![msg("user", "Привет")]);
        chat.id = r"..\..\settings".into();
        // Плохой id не сохраняется как есть — разговору выдаётся новый.
        let saved = s.save(chat).unwrap();
        assert!(safe_id(&saved.id));
        assert!(s.get(r"..\..\settings").is_none());
    }

    #[test]
    fn broken_file_is_skipped() {
        let s = store("broken");
        s.save(empty(vec![msg("user", "Целый")])).unwrap();
        std::fs::write(s.dir.join("мусор.json"), "{битый").unwrap();
        assert_eq!(s.list().len(), 1);
    }
}
