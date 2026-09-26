//! История разговоров: по файлу на разговор в `chats\` рядом с настройками.
//!
//! Пока JSON: разговоров у одного человека сотни, а не миллионы, и читать папку
//! быстрее, чем тянуть базу. Поиск — перебором файлов: замер в `tests::search_speed`
//! показал, что на таких объёмах база не нужна (цифры — в docs/phase-2.md).

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

/// Найденный разговор: строка списка и кусок реплики вокруг совпадения.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub summary: Summary,
    /// `None` — слова нашлись только в названии.
    pub snippet: Option<String>,
}

/// Сколько символов показать вокруг найденного.
const SNIPPET_BEFORE: usize = 30;
const SNIPPET_AFTER: usize = 90;

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

    /// Разговоры, где есть все слова запроса (в названии или репликах), новые сверху.
    /// Регистр и «ё/е» не важны: человек не помнит, как именно писал.
    pub fn search(&self, query: &str) -> Vec<Hit> {
        let words: Vec<String> = query.split_whitespace().map(fold).collect();
        if words.is_empty() {
            return vec![];
        }
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut hits: Vec<Hit> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| read(&e.path()))
            .filter_map(|c| find(&c, &words))
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.summary.updated));
        hits
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

/// Строчные буквы и «е» вместо «ё». Символ в символ: номер символа совпадения
/// указывает и в исходный текст — из него вырезается кусок для списка.
fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    out.extend(text.chars().map(fold_char));
    out
}

/// Латиница и кириллица — напрямую: общий `to_lowercase` ищет по таблицам Юникода,
/// и на 23 МБ разговоров это 0,3 с из 0,4 (замер в `search_speed`).
fn fold_char(c: char) -> char {
    match c {
        _ if c.is_ascii() => c.to_ascii_lowercase(),
        'а'..='я' => c,
        'А'..='Я' => char::from_u32(c as u32 + 32).unwrap_or(c),
        'ё' | 'Ё' => 'е',
        _ => c.to_lowercase().next().unwrap_or(c),
    }
}

/// Номер символа (не байта) начала совпадения.
fn position(hay: &str, needle: &str) -> Option<usize> {
    hay.find(needle).map(|at| hay[..at].chars().count())
}

/// Разговор подходит, если каждое слово есть в названии или хоть в одной реплике.
fn find(chat: &Chat, words: &[String]) -> Option<Hit> {
    let title = fold(&chat.title);
    let texts: Vec<String> = chat.messages.iter().map(|m| fold(&m.content)).collect();
    let everywhere = words
        .iter()
        .all(|w| title.contains(w.as_str()) || texts.iter().any(|t| t.contains(w.as_str())));
    if !everywhere {
        return None;
    }
    // Кусок — вокруг первого слова, которое нашлось в репликах.
    let snippet = words.iter().find_map(|w| {
        texts
            .iter()
            .zip(&chat.messages)
            .find_map(|(t, m)| position(t, w).map(|at| snippet_at(&m.content, at, w.chars().count())))
    });
    Some(Hit {
        summary: Summary {
            id: chat.id.clone(),
            title: chat.title.clone(),
            updated: chat.updated,
            messages: chat.messages.len(),
        },
        snippet,
    })
}

/// Кусок текста вокруг совпадения в одну строку, с «…» там, где обрезано.
fn snippet_at(text: &str, at: usize, len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(SNIPPET_BEFORE);
    let end = (at + len + SNIPPET_AFTER).min(chars.len());
    let body: String = chars[start..end].iter().collect();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        body,
        if end < chars.len() { "…" } else { "" }
    )
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
    fn search_finds_all_words_ignoring_case_and_yo() {
        let s = store("search");
        let borsch = s
            .save(empty(vec![
                msg("user", "Как сварить борщ?"),
                msg("assistant", "Возьмите свёклу, капусту и говядину. Варите два часа."),
            ]))
            .unwrap();
        s.save(empty(vec![msg("user", "Переведи на английский: кот")])).unwrap();

        // Регистр, «ё» и порядок слов не важны.
        let hits = s.search("СВЕКЛУ  борщ");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].summary.id, borsch.id);
        // Кусок — вокруг первого слова запроса, оно в ответе.
        assert_eq!(hits[0].snippet.as_deref(), Some("Возьмите свёклу, капусту и говядину. Варите два часа."));
        assert_eq!(s.search("борщ")[0].snippet.as_deref(), Some("Как сварить борщ?"));

        // Одного слова нет — разговор не подходит.
        assert!(s.search("борщ кот").is_empty());
        assert!(s.search("   ").is_empty());
    }

    #[test]
    fn snippet_is_cut_around_match() {
        let text = format!("{}ИСКОМОЕ
{}", "а".repeat(100), "б".repeat(200));
        let at = position(&fold(&text), "искомое").unwrap();
        let s = snippet_at(&text, at, 7);
        assert!(s.starts_with('…') && s.ends_with('…'));
        assert!(s.contains("ИСКОМОЕ б"), "перевод строки стал пробелом: {s}");
        assert_eq!(s.chars().count(), 1 + SNIPPET_BEFORE + 7 + SNIPPET_AFTER + 1);
    }

    /// Быстрый путь для латиницы и кириллицы даёт то же, что `to_lowercase`.
    #[test]
    fn fast_fold_matches_unicode() {
        for c in (0..128u32).chain(0x400..0x460).filter_map(char::from_u32) {
            let want = match c.to_lowercase().next().unwrap() {
                'ё' => 'е',
                l => l,
            };
            assert_eq!(fold_char(c), want, "{c:?}");
        }
        assert_eq!(fold("ЁЛКА Straße"), "елка straße");
    }

    /// Хватает ли перебора файлов. 500 разговоров по 40 реплик ≈ 12 МБ —
    /// больше, чем у человека накопится за годы.
    #[test]
    #[ignore]
    fn search_speed() {
        let s = store("speed");
        let para = "Нейросеть отвечает на вопросы про рецепты, путешествия и программирование. ";
        for n in 0..500 {
            let messages = (0..40)
                .map(|i| msg(if i % 2 == 0 { "user" } else { "assistant" }, &format!("{n} {}", para.repeat(8))))
                .collect();
            // Свой номер: 500 разговоров за одну секунду `new_id` не различит.
            s.save(Chat { id: format!("c{n}"), ..empty(messages) }).unwrap();
        }
        let size: u64 = std::fs::read_dir(&s.dir).unwrap().flatten().map(|e| e.metadata().unwrap().len()).sum();
        let t = std::time::Instant::now();
        println!("только прочитать: {} за {:?}", s.list().len(), t.elapsed());
        for query in ["программирование", "нет такого слова", "рецепты 499"] {
            let t = std::time::Instant::now();
            let hits = s.search(query).len();
            println!("{query:?}: {hits} за {:?}, всего {} МБ", t.elapsed(), size / 1_000_000);
        }
    }

    #[test]
    fn broken_file_is_skipped() {
        let s = store("broken");
        s.save(empty(vec![msg("user", "Целый")])).unwrap();
        std::fs::write(s.dir.join("мусор.json"), "{битый").unwrap();
        assert_eq!(s.list().len(), 1);
    }
}
