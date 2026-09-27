//! Папка проекта у разговора: список файлов, чтение, поиск и запись — только внутри неё.
//!
//! Проект целиком в память модели не влезает (у локальной модели 4–32 тыс. токенов),
//! поэтому модель всегда видит только список путей, а содержимое открывает сама
//! инструментами (`Tools`) или получает файлы, приложенные человеком через «@».
//! Записать файл модель может только с согласия человека, старый файл перед
//! заменой копируется — его можно вернуть.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Папки, которые модели не показываем, даже если их нет в `.gitignore`: сборка,
/// зависимости и служебное — тысячи файлов, которые только съедят память модели.
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", "out", ".next", ".nuxt", ".cache", "coverage",
    ".venv", "venv", "env", "__pycache__", ".mypy_cache", ".pytest_cache", ".idea", ".vs", "obj",
];
/// Больше — это уже не проект, а диск целиком; дальше не обходим.
const MAX_FILES: usize = 20_000;
/// Текстовый файл больше — скорее данные (дамп, лог), а не код; в поиске пропускаем.
const MAX_SEARCH_FILE: u64 = 1 << 20;
/// Сколько совпадений поиска отдать модели: дальше она всё равно не прочтёт.
const MAX_HITS: usize = 60;
/// Документы, которые читаем через `attach` (не как простой текст).
const DOCS: &[&str] = &["pdf", "docx", "odt"];
const IMAGES: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "webp", "ico", "svgz", "tif", "tiff", "heic", "avif"];

/// Файлы папки для окна (список по «@») и для модели.
#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub root: PathBuf,
    pub name: String,
    /// Пути от корня папки, через «/», по алфавиту.
    pub files: Vec<String>,
    /// Файлов больше `MAX_FILES` — показаны не все.
    pub truncated: bool,
}

/// Обходит папку с учётом `.gitignore`; скрытые файлы и `SKIP_DIRS` пропускает.
pub fn list(root: &Path) -> Result<Listing, String> {
    if !root.is_dir() {
        return Err(t!("папка не найдена — возможно, её переместили или удалили", "folder not found — it may have been moved or deleted").into());
    }
    let walker = ignore::WalkBuilder::new(root)
        // .gitignore работает и без репозитория: проект могли скачать архивом.
        .require_git(false)
        .git_global(false)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir()) && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build();
    let mut files = Vec::new();
    let mut truncated = false;
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        if files.len() >= MAX_FILES {
            truncated = true;
            break;
        }
        files.push(slashed(rel));
    }
    files.sort();
    Ok(Listing {
        root: root.to_path_buf(),
        name: root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned()),
        files,
        truncated,
    })
}

fn slashed(rel: &Path) -> String {
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Имена устройств: файл «nul.txt» Windows создать не даст, а «con» повесит запись.
fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// Путь, который назвала модель, — внутри папки или ошибка. Модель не должна дотянуться
/// до чужих файлов ни через «..», ни через абсолютный путь, ни через ссылку-ярлык внутри
/// папки, указывающую наружу: поэтому сверяем настоящие (разыменованные) пути.
pub fn resolve(root: &Path, rel: &str, write: bool) -> Result<PathBuf, String> {
    let rel = rel.trim().replace('\\', "/");
    // «/src/main.py» маленькие модели пишут, имея в виду корень папки проекта.
    let rel = rel.trim_start_matches('/');
    let mut path = root.to_path_buf();
    let mut any = false;
    for part in rel.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(outside().into()),
            _ if part.contains(':') => return Err(outside().into()),
            _ if reserved(part) => return Err(tf!("имя «{part}» Windows не разрешает", "Windows doesn't allow the name “{part}”")),
            _ if write && part.eq_ignore_ascii_case(".git") => {
                return Err(t!("служебную папку .git менять нельзя", "the service folder .git can't be changed").into())
            }
            _ => {}
        }
        path.push(part);
        any = true;
    }
    if !any && write {
        return Err(t!("не указано имя файла", "no file name given").into());
    }
    // Проверяем ближайшую существующую часть пути: у нового файла это его папка.
    let real_root = root.canonicalize().map_err(|_| t!("папка проекта не найдена", "project folder not found").to_string())?;
    let mut probe = path.as_path();
    while !probe.exists() {
        probe = probe.parent().ok_or(outside())?;
    }
    let real = probe.canonicalize().map_err(|e| tf!("путь не открывается: {e}", "the path won't open: {e}"))?;
    if !real.starts_with(&real_root) {
        return Err(outside().into());
    }
    Ok(path)
}

fn outside() -> &'static str {
    t!("путь выходит за папку проекта", "the path goes outside the project folder")
}

fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Список путей для модели в пределах `budget` байт. Не влезает — глубокие папки
/// сворачиваются в строку «папка/ — N файлов»: модель откроет её сама (`list_files`).
pub fn tree(files: &[String], budget: usize) -> String {
    let full = files.join("\n");
    if full.len() <= budget {
        return full;
    }
    let deepest = files.iter().map(|f| f.matches('/').count()).max().unwrap_or(0);
    for depth in (0..deepest).rev() {
        let t = collapsed(files, depth);
        if t.len() <= budget {
            return t;
        }
    }
    // Даже верхний уровень не влез — обрезаем по строкам.
    let top = collapsed(files, 0);
    let mut out = String::new();
    let lines: Vec<&str> = top.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if out.len() + line.len() + 40 > budget {
            out.push_str(&tf!("… (не поместилось строк: {})", "… ({} more lines didn't fit)", lines.len() - i));
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Файлы не глубже `depth` папок — поимённо, остальное — по папкам с числом файлов.
/// Список отсортирован, поэтому файлы одной папки идут подряд.
fn collapsed(files: &[String], depth: usize) -> String {
    let mut out: Vec<(String, usize)> = Vec::new();
    for f in files {
        let parts: Vec<&str> = f.split('/').collect();
        if parts.len() <= depth + 1 {
            out.push((f.clone(), 0));
            continue;
        }
        let dir = format!("{}/", parts[..=depth].join("/"));
        match out.last_mut() {
            Some((d, n)) if *n > 0 && *d == dir => *n += 1,
            _ => out.push((dir, 1)),
        }
    }
    out.into_iter()
        .map(|(p, n)| if n > 0 { tf!("{p} — {} (посмотреть: list_files)", "{p} — {} (to see them: list_files)", files_word(n)) } else { p })
        .collect::<Vec<_>>()
        .join("\n")
}

fn files_word(n: usize) -> String {
    format!("{n} {}", crate::i18n::plural(n as u64, ["файл", "файла", "файлов"], ["file", "files"]))
}

/// Что лежит прямо в папке `dir` (пусто — корень): подпапки с числом файлов и файлы.
pub fn children(files: &[String], dir: &str) -> Result<String, String> {
    let dir = dir.trim().replace('\\', "/");
    let dir = dir.trim_matches('/');
    let prefix = if dir.is_empty() || dir == "." { String::new() } else { format!("{dir}/") };
    let mut dirs: Vec<(String, usize)> = Vec::new();
    let mut plain = Vec::new();
    for f in files.iter().filter_map(|f| f.strip_prefix(&prefix)) {
        match f.split_once('/') {
            Some((sub, _)) => match dirs.last_mut() {
                Some((d, n)) if d == sub => *n += 1,
                _ => dirs.push((sub.to_string(), 1)),
            },
            None => plain.push(f.to_string()),
        }
    }
    if dirs.is_empty() && plain.is_empty() {
        return Err(tf!("папки «{dir}» нет или в ней нет файлов", "folder “{dir}” doesn't exist or has no files"));
    }
    let mut out: Vec<String> = dirs.into_iter().map(|(d, n)| format!("{prefix}{d}/ — {}", files_word(n))).collect();
    out.extend(plain.into_iter().map(|f| format!("{prefix}{f}")));
    Ok(out.join("\n"))
}

/// Текст файла для модели, начиная со строки `from` (с 1), не больше `max_chars` знаков.
/// Код читаем как есть — без чистки пробелов, как у документов: модель будет его
/// переписывать, и пропавшие пустые строки попали бы в файл.
pub fn read(root: &Path, rel: &str, from: usize, max_chars: usize) -> Result<String, String> {
    let path = resolve(root, rel, false)?;
    if path.is_dir() {
        return Err(t!("это папка, а не файл — посмотрите её через list_files", "this is a folder, not a file — look at it with list_files").into());
    }
    if !path.is_file() {
        return Err(no_file(rel));
    }
    let ext = ext_of(&path);
    if IMAGES.contains(&ext.as_str()) {
        return Err(t!("это картинка — как текст её не прочесть", "this is an image — it can't be read as text").into());
    }
    let text = if DOCS.contains(&ext.as_str()) {
        crate::attach::read(&path, Path::new(""), None)?.text
    } else {
        let bytes = std::fs::read(&path).map_err(|e| unreadable(&e))?;
        if crate::attach::looks_binary(&bytes) {
            return Err(t!("это не текст — такой файл модель прочесть не может", "this isn't text — the model can't read such a file").into());
        }
        crate::attach::decode(&bytes).replace("\r\n", "\n")
    };
    Ok(window(&text, from.max(1), max_chars))
}

/// Кусок текста со строки `from`; не влез целиком — в конце подсказка, как читать дальше.
fn window(text: &str, from: usize, max_chars: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if from == 1 && text.chars().count() <= max_chars {
        return text.to_string();
    }
    if from > lines.len() {
        return tf!("[в файле всего {} строк]", "[the file has only {} lines]", lines.len());
    }
    let mut out = String::new();
    let mut last = from - 1;
    for line in &lines[from - 1..] {
        if !out.is_empty() && out.chars().count() + line.chars().count() > max_chars {
            break;
        }
        out.push_str(line);
        out.push('\n');
        last += 1;
    }
    if last < lines.len() {
        out.push_str(&tf!(
            "[показаны строки {from}–{last} из {}. Продолжение — read_file с from_line = {}]",
            "[showing lines {from}–{last} of {}. To continue — read_file with from_line = {}]",
            lines.len(),
            last + 1
        ));
    }
    out
}

/// Строки со словами запроса во всех текстовых файлах: «путь:строка: текст».
/// Регистр и «ё/е» не важны — как в поиске по разговорам.
pub fn search(root: &Path, files: &[String], query: &str) -> Result<String, String> {
    let q = crate::chats::fold(query.trim());
    if q.is_empty() {
        return Err(t!("пустой запрос", "empty query").into());
    }
    let mut hits = Vec::new();
    let mut more = 0;
    for rel in files {
        let path = root.join(rel);
        let ext = ext_of(&path);
        if IMAGES.contains(&ext.as_str()) || DOCS.contains(&ext.as_str()) {
            continue;
        }
        if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_SEARCH_FILE) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if crate::attach::looks_binary(&bytes) {
            continue;
        }
        for (i, line) in crate::attach::decode(&bytes).lines().enumerate() {
            if crate::chats::fold(line).contains(&q) {
                if hits.len() < MAX_HITS {
                    let short: String = line.trim().chars().take(160).collect();
                    hits.push(format!("{rel}:{}: {short}", i + 1));
                } else {
                    more += 1;
                }
            }
        }
    }
    if hits.is_empty() {
        return Ok(tf!("«{query}» не нашлось ни в одном файле", "“{query}” was not found in any file"));
    }
    if more > 0 {
        hits.push(tf!("[и ещё {more} совпадений — уточните запрос]", "[and {more} more matches — narrow the query]"));
    }
    Ok(hits.join("\n"))
}

/// Что модель сделала с папкой — для окна и истории разговора.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Step {
    /// `read`, `list`, `search`, `write`, `edit`, `delete`.
    pub kind: String,
    /// Путь от корня папки; у поиска — запрос.
    pub path: String,
    pub ok: bool,
    /// Почему не вышло: «файла нет», «вы отказались сохранять».
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// У записи: номер копии старого файла (`None` — файл новый).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    /// У записи: SHA256 записанного. «Вернуть как было» сверяет его, чтобы
    /// не затереть то, что человек поменял в файле после модели.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub undone: bool,
}

/// Модель просит записать или удалить файл — окно показывает это человеку.
#[derive(Debug, Clone, Serialize)]
pub struct WriteAsk {
    /// `write` — целиком, `edit` — кусок, `delete` — удалить (`content` — что удалится).
    pub kind: String,
    pub path: String,
    pub content: String,
    pub exists: bool,
    pub old_lines: usize,
    pub new_lines: usize,
    /// Правка куском (`edit_file`): что было и что станет. У записи целиком — нет.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// Правка куском: `old` — точный кусок файла, `new` — на что заменить. Кусок должен
/// найтись ровно один раз, иначе непонятно, что менять. Маленькие модели часто теряют
/// отступы и хвостовые пробелы, поэтому, если точного совпадения нет, ищем те же строки
/// без учёта отступов и возвращаем правке отступ файла.
pub fn apply_edit(text: &str, old: &str, new: &str) -> Result<String, String> {
    let (text, old, new) = (text.replace("\r\n", "\n"), old.replace("\r\n", "\n"), new.replace("\r\n", "\n"));
    if old.trim().is_empty() {
        return Err(t!("old_text пустой — новый файл создавай через write_file", "old_text is empty — create a new file with write_file").into());
    }
    match text.matches(old.as_str()).count() {
        1 => {
            // Кусок начинается после отступа («return x» в теле функции), а в новых строках
            // модель отступ не повторила — добавляем отступ строки файла.
            let at = text.find(old.as_str()).unwrap();
            let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
            let pad = &text[line_start..at];
            let new = if pad.trim().is_empty() && !pad.is_empty() {
                new.split('\n')
                    .enumerate()
                    .map(|(i, l)| if i == 0 || l.is_empty() || l.starts_with(pad) { l.to_string() } else { format!("{pad}{l}") })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                new
            };
            return Ok(text.replacen(old.as_str(), &new, 1));
        }
        0 => {}
        n => {
            return Err(tf!(
                "этот кусок встречается в файле {n} раз — возьми кусок побольше, чтобы он был один",
                "this piece occurs {n} times in the file — take a bigger piece so it is unique"
            ))
        }
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let want: Vec<&str> = old.trim_matches('\n').split('\n').collect();
    let same = |at: usize| want.iter().enumerate().all(|(i, w)| lines[at + i].trim() == w.trim());
    let found: Vec<usize> = (0..=lines.len().saturating_sub(want.len())).filter(|&at| same(at)).collect();
    let at = match found[..] {
        [at] => at,
        [] => {
            return Err(t!(
                "такого куска в файле нет — прочитай файл заново и скопируй кусок точно",
                "there is no such piece in the file — read the file again and copy the piece exactly"
            )
            .into())
        }
        _ => {
            return Err(tf!(
                "этот кусок встречается в файле {} раз — возьми кусок побольше",
                "this piece occurs {} times in the file — take a bigger piece",
                found.len()
            ))
        }
    };
    // Отступ, который модель потеряла: разница между файлом и её куском в первой строке.
    let indent = |s: &str| s.len() - s.trim_start().len();
    let lost = indent(lines[at]).saturating_sub(indent(want[0]));
    let pad = &lines[at][..lost];
    let fixed: Vec<String> = new
        .trim_matches('\n')
        .split('\n')
        .map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") })
        .collect();
    let mut out: Vec<String> = lines[..at].iter().map(|s| s.to_string()).collect();
    if !new.trim().is_empty() {
        out.extend(fixed);
    }
    out.extend(lines[at + want.len()..].iter().map(|s| s.to_string()));
    Ok(out.join("\n"))
}

/// Текст в байты так же, как был записан старый файл: те же концы строк и кодировка.
/// Иначе Git покажет изменённым каждую строку, а старый проект в Windows-1251 — кракозябры.
fn encode_like(old: Option<&[u8]>, text: &str) -> Vec<u8> {
    let text = text.replace("\r\n", "\n");
    let Some(old) = old else { return text.into_bytes() };
    let text = if old.windows(2).any(|w| w == b"\r\n") { text.replace('\n', "\r\n") } else { text };
    match encoding_rs::Encoding::for_bom(old) {
        Some((enc, _)) if enc == encoding_rs::UTF_16LE => {
            let mut out = vec![0xFF, 0xFE];
            out.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
            out
        }
        Some(_) => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        None if !old.is_empty() && std::str::from_utf8(old).is_err() => {
            let (bytes, _, lossy) = encoding_rs::WINDOWS_1251.encode(&text);
            if lossy { text.into_bytes() } else { bytes.into_owned() }
        }
        None => text.into_bytes(),
    }
}

/// Записывает файл; старый сначала копирует в `backups`. Возвращает шаг для истории.
pub fn write(root: &Path, rel: &str, content: &str, backups: &Path) -> Result<Step, String> {
    let path = resolve(root, rel, true)?;
    if path.is_dir() {
        return Err(t!("по этому пути лежит папка", "there is a folder at this path").into());
    }
    let old = if path.is_file() { Some(std::fs::read(&path).map_err(|e| unreadable(&e))?) } else { None };
    let bytes = encode_like(old.as_deref(), content);
    let backup = match &old {
        Some(old) => {
            std::fs::create_dir_all(backups).map_err(|e| no_backup(&e))?;
            let id = backup_id(backups);
            std::fs::write(backups.join(&id), old).map_err(|e| no_backup(&e))?;
            Some(id)
        }
        None => None,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| tf!("не создать папку: {e}", "could not create the folder: {e}"))?;
    }
    // Через временный файл: сбой посреди записи не оставит полфайла.
    let tmp = path.with_file_name(format!(".{}.ollivo-tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, &bytes)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            no_write(&e)
        })?;
    Ok(Step {
        kind: "write".into(),
        path: rel_clean(rel),
        ok: true,
        backup,
        hash: Some(hex::encode(Sha256::digest(&bytes))),
        ..Step::default()
    })
}

/// Путь через «/», без «./» и «/» в начале. Точку в начале имени не трогаем: «.gitignore».
fn rel_clean(rel: &str) -> String {
    let mut rel = rel.trim().replace('\\', "/");
    while let Some(rest) = rel.strip_prefix("./").or_else(|| rel.strip_prefix('/')) {
        rel = rest.to_string();
    }
    rel
}

/// Номер копии — время в наносекундах: копий немного, и так их легко упорядочить.
fn backup_id(backups: &Path) -> String {
    let base = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    (0..)
        .map(|n| format!("{}", base + n))
        .find(|id| !backups.join(id).exists())
        .unwrap()
}

/// Удаляет файл, сначала скопировав его в `backups`: удалённое можно вернуть.
pub fn remove(root: &Path, rel: &str, backups: &Path) -> Result<Step, String> {
    let path = resolve(root, rel, true)?;
    if path.is_dir() {
        return Err(t!("это папка — удалять можно только файлы", "this is a folder — only files can be deleted").into());
    }
    let old = std::fs::read(&path).map_err(|_| no_file(rel))?;
    std::fs::create_dir_all(backups).map_err(|e| no_backup(&e))?;
    let id = backup_id(backups);
    std::fs::write(backups.join(&id), &old).map_err(|e| no_backup(&e))?;
    std::fs::remove_file(&path).map_err(|e| tf!("не удалить файл: {e}", "could not delete the file: {e}"))?;
    Ok(Step {
        kind: "delete".into(),
        path: rel_clean(rel),
        ok: true,
        backup: Some(id),
        hash: Some(hex::encode(Sha256::digest(&old))),
        ..Step::default()
    })
}

/// Сколько хранятся копии заменённых файлов. Вернуть правку модели через месяц — редкость,
/// а копии большого проекта занимают место.
pub const BACKUP_DAYS: u64 = 30;
fn gone() -> &'static str {
    t!("копии старого файла уже нет — они хранятся 30 дней", "the copy of the old file is gone — copies are kept for 30 days")
}

fn path_hint() -> &'static str {
    t!("путь от корня папки проекта", "path from the project folder root")
}

fn no_file(rel: &str) -> String {
    tf!("файла «{rel}» нет", "there is no file “{rel}”")
}

fn unreadable(e: &std::io::Error) -> String {
    tf!("файл не читается: {e}", "the file can't be read: {e}")
}

fn no_backup(e: &std::io::Error) -> String {
    tf!("не сохранить копию старого файла: {e}", "could not save a copy of the old file: {e}")
}

fn no_write(e: &std::io::Error) -> String {
    tf!("не записать файл: {e}", "could not write the file: {e}")
}

/// «Вернуть как было»: старый файл из копии, а новый — удалить. Только если файл
/// с тех пор не меняли: иначе пропала бы чужая правка.
pub fn undo(root: &Path, step: &Step, backups: &Path) -> Result<(), String> {
    if !matches!(step.kind.as_str(), "write" | "edit" | "delete") || !step.ok || step.undone {
        return Err(t!("тут нечего возвращать", "there is nothing to undo here").into());
    }
    let path = resolve(root, &step.path, true)?;
    if step.kind == "delete" {
        if path.exists() {
            return Err(t!(
                "файл с таким именем уже появился снова — возвращать не стану, чтобы его не затереть",
                "a file with this name has appeared again — I won't restore it so as not to overwrite it"
            )
            .into());
        }
        let id = step.backup.as_deref().filter(|id| id.chars().all(|c| c.is_ascii_digit())).ok_or(gone())?;
        let old = std::fs::read(backups.join(id)).map_err(|_| gone().to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| tf!("не создать папку: {e}", "could not create the folder: {e}"))?;
        }
        return std::fs::write(&path, old).map_err(|e| no_write(&e));
    }
    let now = std::fs::read(&path).ok();
    let same = now.as_ref().is_some_and(|b| Some(hex::encode(Sha256::digest(b))) == step.hash);
    if !same {
        return Err(t!(
            "файл уже поменяли после модели — возвращать не стану, чтобы не потерять эти правки",
            "the file was changed after the model — I won't restore it so as not to lose those edits"
        )
        .into());
    }
    match &step.backup {
        Some(id) => {
            if !id.chars().all(|c| c.is_ascii_digit()) {
                return Err(gone().into());
            }
            let old = std::fs::read(backups.join(id)).map_err(|_| gone().to_string())?;
            std::fs::write(&path, old).map_err(|e| no_write(&e))
        }
        None => {
            std::fs::remove_file(&path).map_err(|e| tf!("не удалить файл: {e}", "could not delete the file: {e}"))?;
            // Папки, которые появились ради этого файла, тоже убираем — если в них пусто.
            let mut dir = path.parent();
            while let Some(d) = dir.filter(|d| *d != root) {
                if std::fs::remove_dir(d).is_err() {
                    break; // не пустая — дальше вверх тем более
                }
                dir = d.parent();
            }
            Ok(())
        }
    }
}

/// Как модели обращаться с файлами — выбирает человек, соблюдает ядро.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// «Вручную»: каждое создание, правку и удаление человек подтверждает.
    Ask,
    /// «Авто»: модель меняет файлы сама; копии старых всё равно делаются.
    Auto,
    /// «План»: ничего не меняет — инструментов записи у модели просто нет.
    Plan,
}

impl Mode {
    /// id из окна; неизвестное — «Вручную», самый осторожный из рабочих режимов.
    pub fn from_id(id: &str) -> Self {
        match id {
            "auto" => Mode::Auto,
            "plan" => Mode::Plan,
            _ => Mode::Ask,
        }
    }
}

/// Спросить человека, можно ли записать файл. Ответ — `true`, если он согласился.
pub type AskWrite = Arc<dyn Fn(WriteAsk) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// Инструменты модели для одной папки на время одного ответа.
pub struct Tools {
    pub root: PathBuf,
    pub name: String,
    files: Mutex<Vec<String>>,
    /// Что уже прочитано за этот ответ: путь и строка начала. Qwen2.5 3B читала один
    /// и тот же main.py пять раз подряд — второй раз отдаём короткую пометку, а не файл.
    seen: Mutex<std::collections::HashSet<(String, usize)>>,
    truncated: bool,
    /// Память модели в токенах: от неё зависят длина списка файлов и кусок файла за раз.
    ctx: u32,
    backups: PathBuf,
    mode: Mode,
    ask_write: AskWrite,
}

impl Tools {
    pub fn new(listing: Listing, ctx: u32, backups: PathBuf, mode: Mode, ask_write: AskWrite) -> Self {
        Tools {
            root: listing.root,
            name: listing.name,
            files: Mutex::new(listing.files),
            seen: Mutex::default(),
            truncated: listing.truncated,
            ctx,
            backups,
            mode,
            ask_write,
        }
    }

    /// Путь от модели — от корня папки. Маленькие модели часто начинают его с имени
    /// самой папки («shop/main.py» в папке shop): Qwen3.5 2B так создала файл
    /// во вложенной папке. Если такой подпапки нет — имя папки убираем.
    fn path(&self, rel: &str) -> String {
        let rel = rel_clean(rel);
        match rel.strip_prefix(&format!("{}/", self.name)) {
            Some(rest) if !self.root.join(&self.name).is_dir() => rest.to_string(),
            _ if rel == self.name && !self.root.join(&self.name).exists() => String::new(),
            _ => rel,
        }
    }

    /// Память модели в токенах.
    pub fn ctx(&self) -> u32 {
        self.ctx
    }

    /// Около трети памяти модели на кусок файла (~3 знака кода на токен).
    fn read_chars(&self) -> usize {
        (self.ctx as usize).max(2000)
    }

    /// Описание инструментов в формате OpenAI — llama-server подставит его в шаблон модели.
    /// В режиме «План» инструментов записи нет вовсе: модели нечем что-то поменять.
    pub fn specs(&self) -> serde_json::Value {
        let tool = |name: &str, description: &str, props: serde_json::Value, required: &[&str]| {
            serde_json::json!({"type": "function", "function": {
                "name": name,
                "description": description,
                "parameters": {"type": "object", "properties": props, "required": required},
            }})
        };
        let mut all = vec![
            tool(
                "read_file",
                t!(
                    "Прочитать файл из папки проекта. Длинный файл отдаётся кусками — продолжение по from_line.",
                    "Read a file from the project folder. A long file comes in pieces — continue with from_line."
                ),
                serde_json::json!({
                    "path": {"type": "string", "description": t!("путь от корня папки проекта, например src/main.py", "path from the project folder root, e.g. src/main.py")},
                    "from_line": {"type": "integer", "description": t!("с какой строки читать, по умолчанию 1", "line to start reading from, 1 by default")},
                }),
                &["path"],
            ),
            tool(
                "list_files",
                t!("Показать, что лежит в папке внутри проекта: подпапки и файлы.", "Show what is in a folder inside the project: subfolders and files."),
                serde_json::json!({"path": {"type": "string", "description": t!("папка от корня проекта; пусто — корень", "folder from the project root; empty — the root")}}),
                &[],
            ),
            tool(
                "search_files",
                t!(
                    "Найти текст во всех файлах проекта. Возвращает строки вида путь:номер_строки: текст.",
                    "Find text in all project files. Returns lines like path:line_number: text."
                ),
                serde_json::json!({"text": {"type": "string", "description": t!("что искать; регистр не важен", "what to find; case doesn't matter")}}),
                &["text"],
            ),
        ];
        if self.mode == Mode::Plan {
            return all.into();
        }
        all.extend([
            tool(
                "edit_file",
                t!(
                    "Поправить кусок существующего файла: old_text заменить на new_text. old_text — точная копия \
                     куска из файла, несколько строк целиком; он должен встречаться в файле один раз. \
                     Пользователь сначала подтвердит правку.",
                    "Fix a piece of an existing file: replace old_text with new_text. old_text is an exact copy \
                     of a piece of the file, several whole lines; it must occur in the file once. \
                     The user will confirm the edit first."
                ),
                serde_json::json!({
                    "path": {"type": "string", "description": path_hint()},
                    "old_text": {"type": "string", "description": t!("что заменить — точно как в файле", "what to replace — exactly as in the file")},
                    "new_text": {"type": "string", "description": t!("на что заменить", "what to replace it with")},
                }),
                &["path", "old_text", "new_text"],
            ),
            tool(
                "write_file",
                t!(
                    "Создать новый файл или заменить его целиком новым содержимым. Пользователь сначала подтвердит запись.",
                    "Create a new file or replace it entirely with new content. The user will confirm the write first."
                ),
                serde_json::json!({
                    "path": {"type": "string", "description": path_hint()},
                    "content": {"type": "string", "description": t!("полное новое содержимое файла", "the full new content of the file")},
                }),
                &["path", "content"],
            ),
            tool(
                "delete_file",
                t!("Удалить файл из папки проекта. Только если пользователь об этом просил.", "Delete a file from the project folder. Only if the user asked for it."),
                serde_json::json!({"path": {"type": "string", "description": path_hint()}}),
                &["path"],
            ),
        ]);
        all.into()
    }

    /// Что модель знает о папке: имя, список файлов и как с ними работать.
    /// `tools` — умеет ли модель вызывать инструменты; не умеет — видит только список
    /// и то, что человек приложил сам.
    pub fn prompt(&self, tools: bool) -> String {
        let files = self.files.lock().unwrap();
        let tree = tree(&files, (self.ctx as usize / 2).max(1500));
        let more = if self.truncated { t!("\n[файлов очень много — показаны не все]", "\n[too many files — not all are shown]") } else { "" };
        let see = if tools {
            t!(
                "Прежде чем отвечать про файл, прочитай его инструментом read_file — не придумывай код, \
                 которого не видел. Искать по всем файлам — search_files, смотреть папку — list_files. \
                 Пути пиши от корня папки, без её имени: например, main.py или src/app.py. \
                 Файлы, приложенные к вопросу, уже перед тобой — их читать не нужно.",
                "Before answering about a file, read it with the read_file tool — don't make up code \
                 you haven't seen. To search all files — search_files, to look at a folder — list_files. \
                 Write paths from the folder root, without its name: for example, main.py or src/app.py. \
                 Files attached to the question are already in front of you — no need to read them."
            )
        } else {
            t!(
                "Содержимое файлов ты видишь, только если пользователь приложил их к вопросу. \
                 Если для ответа нужен другой файл — попроси приложить его.",
                "You see file contents only if the user attached them to the question. \
                 If you need another file to answer, ask to attach it."
            )
        };
        let change = match (self.mode, tools) {
            (Mode::Plan, _) => t!(
                "Сейчас режим плана: ты ничего не создаёшь, не меняешь и не удаляешь. Изучи нужные \
                 файлы и напиши план по шагам: какие файлы создать, изменить или удалить и что именно \
                 в них сделать. Полный код не пиши — только ключевые места. В конце спроси, выполнять ли план.",
                "This is plan mode: you don't create, change or delete anything. Study the files you need \
                 and write a step-by-step plan: which files to create, change or delete and what exactly \
                 to do in them. Don't write the full code — only the key parts. At the end, ask whether to carry out the plan."
            ),
            (_, false) => t!(
                "Новый код давай целиком в блоке кода и пиши, в какой файл его сохранить.",
                "Give new code in full in a code block and say which file to save it to."
            ),
            (Mode::Ask, true) => t!(
                "Поправить часть существующего файла — edit_file (точный кусок и на что его заменить), \
                 создать файл или переписать целиком — write_file, удалить — delete_file. Каждое \
                 изменение пользователь подтверждает. Запускать программы ты не можешь.",
                "To fix part of an existing file — edit_file (the exact piece and what to replace it with), \
                 to create a file or rewrite it entirely — write_file, to delete — delete_file. The user \
                 confirms every change. You can't run programs."
            ),
            (Mode::Auto, true) => t!(
                "Поправить часть существующего файла — edit_file (точный кусок и на что его заменить), \
                 создать файл или переписать целиком — write_file, удалить — delete_file. Изменения \
                 применяются сразу, без подтверждения, — меняй только то, о чём просили. Запускать \
                 программы ты не можешь.",
                "To fix part of an existing file — edit_file (the exact piece and what to replace it with), \
                 to create a file or rewrite it entirely — write_file, to delete — delete_file. Changes \
                 apply at once, without confirmation — change only what was asked. You can't run programs."
            ),
        };
        let how = format!("{see} {change}");
        tf!(
            "Ты работаешь с папкой проекта «{}» на компьютере пользователя. Файлы в ней:\n{tree}{more}\n\n{how}",
            "You are working with the project folder “{}” on the user's computer. Its files:\n{tree}{more}\n\n{how}",
            self.name
        )
    }

    /// Выполняет вызов модели. Возвращает текст для модели и шаг для окна.
    pub async fn call(&self, name: &str, arguments: &str) -> (String, Step) {
        let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
        let arg = |k: &str| args[k].as_str().unwrap_or_default().to_string();
        let (kind, path, res) = match name {
            "read_file" => {
                let path = self.path(&arg("path"));
                let from = args["from_line"].as_u64().unwrap_or(1) as usize;
                let (root, p, max) = (self.root.clone(), path.clone(), self.read_chars());
                let res = if self.seen.lock().unwrap().contains(&(path.clone(), from)) {
                    Ok(t!(
                        "Этот файл уже прочитан выше и с тех пор не менялся. Отвечай по нему.",
                        "This file was already read above and hasn't changed since. Answer from it."
                    )
                    .into())
                } else {
                    let res = blocking(move || read(&root, &p, from, max)).await;
                    if res.is_ok() {
                        self.seen.lock().unwrap().insert((path.clone(), from));
                    }
                    res
                };
                ("read", path, res)
            }
            "list_files" => {
                let path = self.path(&arg("path"));
                let res = children(&self.files.lock().unwrap(), &path);
                ("list", path, res)
            }
            "search_files" => {
                let text = arg("text");
                let (root, files, q) = (self.root.clone(), self.files.lock().unwrap().clone(), text.clone());
                let res = blocking(move || search(&root, &files, &q)).await;
                ("search", text, res)
            }
            "write_file" | "edit_file" | "delete_file" if self.mode == Mode::Plan => {
                // Инструментов записи в «Плане» модели не даём; сюда попадёт только выдуманный вызов.
                ("plan", self.path(&arg("path")), Err(t!("сейчас режим плана — файлы не меняем, опиши это в плане", "this is plan mode — we don't change files, describe it in the plan").into()))
            }
            "write_file" => {
                let path = self.path(&arg("path"));
                let content = arg("content");
                return self.save("write", path, Some(content), None).await;
            }
            "delete_file" => {
                let path = self.path(&arg("path"));
                return self.save("delete", path, None, None).await;
            }
            "edit_file" => {
                let path = self.path(&arg("path"));
                let (old, new) = (arg("old_text"), arg("new_text"));
                let (root, p) = (self.root.clone(), path.clone());
                let current = blocking(move || {
                    let full = resolve(&root, &p, true)?;
                    let bytes = std::fs::read(&full)
                        .map_err(|_| t!("такого файла нет — новый файл создавай через write_file", "there is no such file — create a new file with write_file").to_string())?;
                    Ok(crate::attach::decode(&bytes))
                })
                .await;
                let text = match current {
                    Ok(t) => t,
                    Err(e) => return fail_step("edit", path, e, String::new()),
                };
                match apply_edit(&text, &old, &new) {
                    Ok(content) => return self.save("edit", path, Some(content), Some((old, new))).await,
                    // Промахнулась куском — отдаём файл как есть: Qwen2.5 3B правила по памяти
                    // и дважды «цитировала» строки, которых в файле не было.
                    Err(e) if text.chars().count() <= self.read_chars() => {
                        let now = text.replace("\r\n", "\n");
                        let hint = tf!("\nФайл сейчас такой — копируй кусок отсюда точно:\n{now}", "\nThe file is now like this — copy the piece from here exactly:\n{now}");
                        return fail_step("edit", path, e, hint);
                    }
                    Err(e) => return fail_step("edit", path, e, String::new()),
                }
            }
            other => ("unknown", other.to_string(), Err(tf!("инструмента «{other}» нет", "there is no tool “{other}”"))),
        };
        let step = Step { kind: kind.into(), path, ok: res.is_ok(), note: res.as_ref().err().cloned().unwrap_or_default(), ..Step::default() };
        (res.unwrap_or_else(|e| tf!("Ошибка: {e}", "Error: {e}")), step)
    }

    /// Записать или удалить файл — в «Вручную» после согласия человека. `kind` — `write`
    /// (целиком), `edit` (кусок: `change` — было и стало, для карточки) или `delete`
    /// (`content` — `None`); у записи `content` — всегда файл целиком.
    async fn save(&self, kind: &str, path: String, content: Option<String>, change: Option<(String, String)>) -> (String, Step) {
        let fail = |note: String| {
            (tf!("Ошибка: {note}", "Error: {note}"), Step { kind: kind.into(), path: path.clone(), note, ..Step::default() })
        };
        let full = match resolve(&self.root, &path, true) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let old = std::fs::read(&full).ok().map(|b| crate::attach::decode(&b));
        if content.is_none() && old.is_none() {
            return fail(no_file(&path));
        }
        if self.mode == Mode::Ask {
            let ask = WriteAsk {
                kind: kind.into(),
                path: path.clone(),
                exists: old.is_some(),
                old_lines: old.as_deref().map_or(0, |t| t.lines().count()),
                new_lines: content.as_deref().map_or(0, |t| t.lines().count()),
                // У удаления показываем, что пропадёт.
                content: content.clone().or(old).unwrap_or_default(),
                before: change.as_ref().map(|c| c.0.clone()),
                after: change.map(|c| c.1),
            };
            if !(self.ask_write)(ask).await {
                let (_, step) = fail(t!("вы не разрешили", "you didn't allow it").into());
                let answer = t!("Пользователь не разрешил это изменение. Спроси, что поправить.", "The user didn't allow this change. Ask what to fix.");
                return (answer.into(), step);
            }
        }
        let (root, backups, p) = (self.root.clone(), self.backups.clone(), path.clone());
        let res = match content {
            Some(c) => blocking(move || write(&root, &p, &c, &backups)).await,
            None => blocking(move || remove(&root, &p, &backups)).await,
        };
        match res {
            Ok(mut step) => {
                step.kind = kind.into();
                let mut files = self.files.lock().unwrap();
                // Файл поменялся — его снова можно прочитать целиком.
                self.seen.lock().unwrap().retain(|(p, _)| *p != step.path);
                match (files.binary_search(&step.path), kind) {
                    (Ok(i), "delete") => {
                        files.remove(i);
                    }
                    (Err(i), "write" | "edit") => files.insert(i, step.path.clone()),
                    _ => {}
                }
                (if kind == "delete" { t!("Файл удалён.", "File deleted.") } else { t!("Файл сохранён.", "File saved.") }.into(), step)
            }
            Err(e) => fail(e),
        }
    }
}

/// Неудавшийся шаг: причина — в окно, причина и подсказка — модели.
fn fail_step(kind: &str, path: String, why: String, hint: String) -> (String, Step) {
    (tf!("Ошибка: {why}{hint}", "Error: {why}{hint}"), Step { kind: kind.into(), path, note: why, ..Step::default() })
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Справка для модели: что она на самом деле сделала с файлами в прошлом ответе.
/// Содержимое файлов не повторяем — оно съело бы память; нужно снова — прочитает.
/// Справка идёт перед следующим вопросом человека (`carry_steps`), а не в конце ответа
/// модели: Qwen2.5 3B, видя «[С файлами: …]» в своих ответах, стала дописывать такую
/// строку сама — «удалён notes.txt», — ничего не удалив.
pub fn steps_note(steps: &[Step]) -> String {
    let mut parts = Vec::new();
    for s in steps.iter().filter(|s| s.ok) {
        let what = match (s.kind.as_str(), s.undone) {
            ("read", _) => t!("прочитан", "read"),
            ("write", false) => t!("записан", "written"),
            ("write", true) => t!("записан, но пользователь вернул как было:", "written, but the user restored the old version:"),
            ("edit", false) => t!("исправлен", "edited"),
            ("edit", true) => t!("исправлен, но пользователь вернул как было:", "edited, but the user restored the old version:"),
            ("delete", false) => t!("удалён", "deleted"),
            ("delete", true) => t!("удалён, но пользователь вернул:", "deleted, but the user restored:"),
            _ => continue,
        };
        parts.push(format!("{what} {}", s.path));
    }
    if parts.is_empty() {
        String::new()
    } else {
        tf!(
            "(Справка от программы, не от пользователя: в прошлом ответе инструментами {}. Другие файлы не менялись.)",
            "(Note from the program, not from the user: in the previous answer, with tools: {}. No other files changed.)",
            parts.join("; ")
        )
    }
}

/// Переносит справку о действиях с файлами из ответов модели в следующий вопрос человека.
pub fn carry_steps(messages: &mut [crate::llm::Msg]) {
    let mut note = String::new();
    for m in messages.iter_mut() {
        match m.role.as_str() {
            "assistant" => note = steps_note(&m.steps),
            "user" if !note.is_empty() => {
                m.content = format!("{note}\n\n{}", m.content);
                note.clear();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = crate::testserver::tmp().join(format!("ollivo-project-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn list_skips_ignored_and_build_dirs() {
        let root = tmp("list");
        put(&root, "main.py", "print(1)");
        put(&root, "src/app/Мой модуль.py", "x = 1");
        put(&root, "node_modules/left-pad/index.js", "x");
        put(&root, ".git/config", "x");
        put(&root, "secret.log", "x");
        put(&root, ".gitignore", "*.log\n");
        let l = list(&root).unwrap();
        assert_eq!(l.files, ["main.py", "src/app/Мой модуль.py"]);
        assert!(!l.truncated);
    }

    /// Как `download_path_stays_inside_models_folder`: имя пришло от модели — наружу не пускаем.
    #[test]
    fn paths_stay_inside_project() {
        let root = tmp("paths");
        put(&root, "a/b.txt", "x");
        assert!(resolve(&root, "a/b.txt", false).is_ok());
        assert!(resolve(&root, "/a/b.txt", false).is_ok(), "«/» в начале — от корня проекта");
        assert!(resolve(&root, r".\a\b.txt", false).is_ok());
        assert!(resolve(&root, "new/dir/file.rs", true).unwrap().starts_with(&root));
        for bad in ["../x.txt", "a/../../x", r"C:\Windows\win.ini", "C:/Windows/win.ini", "a/b.txt:stream", "nul.txt", "a/COM1"] {
            assert!(resolve(&root, bad, true).is_err(), "{bad}");
        }
        assert!(resolve(&root, ".git/config", true).is_err());
        assert!(resolve(&root, "", true).is_err());
    }

    /// Ссылка-переход (junction) внутри папки на внешний каталог: модель не должна пройти.
    /// Junction создаётся без прав администратора.
    #[test]
    fn junction_outside_is_refused() {
        let root = tmp("junction");
        let outside = tmp("junction-outside");
        put(&outside, "secret.txt", "пароль");
        let link = root.join("link");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .is_ok_and(|o| o.status.success());
        if !made {
            eprintln!("junction не создать — проверка пропущена");
            return;
        }
        assert!(resolve(&root, "link/secret.txt", false).is_err());
        assert!(resolve(&root, "link/new.txt", true).is_err());
        let _ = std::fs::remove_dir(&link);
    }

    #[test]
    fn tree_collapses_deep_folders_when_long() {
        let mut files: Vec<String> = (0..50).map(|i| format!("src/deep/mod{i:02}.rs")).collect();
        files.push("README.md".into());
        files.push("src/main.rs".into());
        files.sort();
        assert_eq!(tree(&files, 100_000).lines().count(), 52);
        let short = tree(&files, 200);
        assert!(short.contains("src/deep/ — 50 файлов"), "{short}");
        assert!(short.contains("src/main.rs") && short.contains("README.md"), "{short}");
        let tiny = tree(&files, 75);
        assert!(tiny.contains("src/ — 51 файл"), "{tiny}");
    }

    #[test]
    fn children_of_folder() {
        let files: Vec<String> = ["a.txt", "src/x/1.rs", "src/x/2.rs", "src/y.rs"].map(String::from).to_vec();
        assert_eq!(children(&files, "").unwrap(), "src/ — 3 файла\na.txt");
        assert_eq!(children(&files, "src/").unwrap(), "src/x/ — 2 файла\nsrc/y.rs");
        assert!(children(&files, "нет").is_err());
    }

    #[test]
    fn read_keeps_code_as_is_and_pages_long_files() {
        let root = tmp("read");
        put(&root, "a.py", "def f():\r\n    pass   \r\n\r\n\r\n\r\nf()\r\n");
        assert_eq!(read(&root, "a.py", 1, 1000).unwrap(), "def f():\n    pass   \n\n\n\nf()\n");
        let long: String = (1..=100).map(|i| format!("строка {i}\n")).collect();
        put(&root, "long.txt", &long);
        let head = read(&root, "long.txt", 1, 100).unwrap();
        assert!(head.starts_with("строка 1\n") && head.contains("from_line = "), "{head}");
        let tail = read(&root, "long.txt", 99, 1000).unwrap();
        assert_eq!(tail, "строка 99\nстрока 100\n");
        assert!(read(&root, "нет.py", 1, 100).unwrap_err().contains("нет"));
        put(&root, "pic.png", "x");
        assert!(read(&root, "pic.png", 1, 100).unwrap_err().contains("картинка"));
    }

    #[test]
    fn search_ignores_case_and_yo() {
        let root = tmp("search");
        put(&root, "a.py", "x = 1\n# Считаем Ёжиков\n");
        put(&root, "b/c.txt", "ежики тут\n");
        let files = list(&root).unwrap().files;
        let out = search(&root, &files, "ЕЖИК").unwrap();
        assert_eq!(out, "a.py:2: # Считаем Ёжиков\nb/c.txt:1: ежики тут");
        assert!(search(&root, &files, "кот").unwrap().contains("не нашлось"));
    }

    #[test]
    fn write_backs_up_and_undo_restores() {
        let root = tmp("write");
        let backups = tmp("write-backups");
        put(&root, "a.txt", "старое\r\n");
        let step = write(&root, "a.txt", "новое\nвторая\n", &backups).unwrap();
        // Концы строк — как были в файле.
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "новое\r\nвторая\r\n");
        assert!(step.backup.is_some());
        undo(&root, &step, &backups).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "старое\r\n");

        // Новый файл в новой папке; «вернуть» — удалить.
        let step = write(&root, "new/dir/b.rs", "fn main() {}\n", &backups).unwrap();
        assert!(step.backup.is_none() && root.join("new/dir/b.rs").is_file());
        undo(&root, &step, &backups).unwrap();
        assert!(!root.join("new").exists(), "пустые папки после файла тоже убраны");

        // Человек поправил файл после модели — не трогаем.
        let step = write(&root, "c.txt", "от модели", &backups).unwrap();
        std::fs::write(root.join("c.txt"), "поправил сам").unwrap();
        assert!(undo(&root, &step, &backups).unwrap_err().contains("поменяли"));
        assert_eq!(std::fs::read_to_string(root.join("c.txt")).unwrap(), "поправил сам");
    }

    #[test]
    fn write_keeps_old_encoding() {
        let (w1251, _, _) = encoding_rs::WINDOWS_1251.encode("Привет");
        assert_eq!(encode_like(Some(&w1251), "Пока"), encoding_rs::WINDOWS_1251.encode("Пока").0.to_vec());
        assert_eq!(encode_like(Some(b"\xEF\xBB\xBFx"), "y"), b"\xEF\xBB\xBFy");
        assert_eq!(encode_like(None, "a\r\nb"), b"a\nb");
    }

    /// «Авто» не спрашивает, «План» не даёт инструментов записи и не пишет даже по выдуманному вызову.
    #[tokio::test]
    async fn modes_auto_and_plan() {
        let names = |t: &Tools| t.specs().as_array().unwrap().iter().map(|s| s["function"]["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let never: AskWrite = Arc::new(|_| panic!("в этом режиме спрашивать нельзя"));
        let root = tmp("modes");
        put(&root, "a.txt", "1\n");
        let auto = Tools::new(list(&root).unwrap(), 4096, tmp("modes-backups"), Mode::Auto, never.clone());
        assert!(names(&auto).contains(&"delete_file".to_string()));
        assert_eq!(auto.call("write_file", r#"{"path":"b.txt","content":"2"}"#).await.0, "Файл сохранён.");
        assert_eq!(auto.call("delete_file", r#"{"path":"a.txt"}"#).await.0, "Файл удалён.");
        assert!(!root.join("a.txt").exists() && root.join("b.txt").exists());

        let plan = Tools::new(list(&root).unwrap(), 4096, tmp("modes-backups"), Mode::Plan, never);
        assert_eq!(names(&plan), ["read_file", "list_files", "search_files"]);
        assert!(plan.prompt(true).contains("режим плана"));
        let (text, step) = plan.call("write_file", r#"{"path":"c.txt","content":"3"}"#).await;
        assert!(text.contains("режим плана") && !step.ok);
        assert!(!root.join("c.txt").exists());
    }

    /// Справка о действиях — перед следующим вопросом, а не в ответе модели.
    #[test]
    fn steps_note_goes_before_next_question() {
        use crate::llm::Msg;
        let mut bot = Msg::new("assistant", "Готово.".into());
        bot.steps.push(Step { kind: "write".into(), path: "notes.txt".into(), ok: true, ..Step::default() });
        bot.steps.push(Step { kind: "delete".into(), path: "x.txt".into(), ok: false, ..Step::default() });
        let mut talk = vec![Msg::new("user", "Создай notes.txt".into()), bot, Msg::new("user", "Удали его".into())];
        carry_steps(&mut talk);
        assert_eq!(talk[1].content, "Готово.");
        assert!(talk[2].content.starts_with("(Справка от программы") && talk[2].content.contains("записан notes.txt"));
        assert!(!talk[2].content.contains("x.txt"), "неудавшееся не пишем");
        assert!(talk[2].content.ends_with("Удали его"));
    }

    #[test]
    fn edit_replaces_one_piece() {
        let text = "def total(prices):\r\n    return sum(prices)\r\n\r\nDISCOUNT = 0.15\r\n";
        assert_eq!(
            apply_edit(text, "DISCOUNT = 0.15", "DISCOUNT = 0.2").unwrap(),
            "def total(prices):\n    return sum(prices)\n\nDISCOUNT = 0.2\n"
        );
        // Модель потеряла отступ — находим по строкам и возвращаем отступ файла.
        let fixed = apply_edit(text, "return sum(prices)", "s = sum(prices)\nreturn s").unwrap();
        assert_eq!(fixed, "def total(prices):\n    s = sum(prices)\n    return s\n\nDISCOUNT = 0.15\n");
        assert!(apply_edit(text, "нет такого", "x").unwrap_err().contains("нет"));
        assert!(apply_edit("a\na\n", "a", "b").unwrap_err().contains("2 раз"));
        assert!(apply_edit(text, "  ", "x").is_err());
    }

    #[tokio::test]
    async fn tools_ask_before_writing() {
        let root = tmp("tools");
        put(&root, "main.py", "print('hi')\n");
        let asked = Arc::new(Mutex::new(Vec::<WriteAsk>::new()));
        let answer = Arc::new(Mutex::new(false));
        let (a, ans) = (asked.clone(), answer.clone());
        let ask: AskWrite = Arc::new(move |w| {
            a.lock().unwrap().push(w);
            let ok = *ans.lock().unwrap();
            Box::pin(async move { ok })
        });
        let tools = Tools::new(list(&root).unwrap(), 4096, tmp("tools-backups"), Mode::Ask, ask);
        assert!(tools.prompt(true).contains("main.py"));

        let (text, step) = tools.call("read_file", r#"{"path":"main.py"}"#).await;
        assert_eq!((text.as_str(), step.kind.as_str(), step.ok), ("print('hi')\n", "read", true));
        // Второй раз тот же файл — короткая пометка вместо текста.
        assert!(tools.call("read_file", r#"{"path":"main.py"}"#).await.0.contains("уже прочитан"));

        let (text, step) = tools.call("write_file", r#"{"path":"main.py","content":"print('bye')\n"}"#).await;
        assert!(text.contains("не разрешил") && !step.ok);
        assert_eq!(std::fs::read_to_string(root.join("main.py")).unwrap(), "print('hi')\n");
        assert!(asked.lock().unwrap()[0].exists);

        *answer.lock().unwrap() = true;
        let (text, step) = tools.call("write_file", r#"{"path":"util/new.py","content":"x = 1\n"}"#).await;
        assert_eq!(text, "Файл сохранён.");
        assert!(step.ok && step.backup.is_none());
        // Правка куском: в вопросе — было и стало, записан файл целиком.
        let (text, step) = tools.call("edit_file", r#"{"path":"util/new.py","old_text":"x = 1","new_text":"x = 2"}"#).await;
        assert_eq!((text.as_str(), step.kind.as_str()), ("Файл сохранён.", "edit"));
        assert_eq!(std::fs::read_to_string(root.join("util/new.py")).unwrap(), "x = 2\n");
        assert_eq!(asked.lock().unwrap().last().unwrap().before.as_deref(), Some("x = 1"));
        assert!(tools.call("edit_file", r#"{"path":"nope.py","old_text":"a","new_text":"b"}"#).await.0.contains("write_file"));
        // Промах куском — модель получает файл, чтобы скопировать точно.
        let (text, step) = tools.call("edit_file", r#"{"path":"util/new.py","old_text":"y = 5","new_text":"y = 6"}"#).await;
        assert!(text.contains("копируй кусок отсюда") && text.ends_with("x = 2\n"), "{text}");
        assert!(!step.ok && !step.note.contains("x = 2"), "в окно — только причина");
        // Новый файл сразу виден модели.
        assert!(tools.call("list_files", r#"{"path":"util"}"#).await.0.contains("util/new.py"));
        // Удаление — тоже с вопросом, и его можно вернуть.
        let (text, step) = tools.call("delete_file", r#"{"path":"util/new.py"}"#).await;
        assert_eq!((text.as_str(), step.kind.as_str()), ("Файл удалён.", "delete"));
        assert_eq!(asked.lock().unwrap().last().unwrap().kind, "delete");
        assert!(!root.join("util/new.py").exists());
        undo(&root, &step, &tools.backups).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("util/new.py")).unwrap(), "x = 2\n");
        assert!(tools.call("read_file", r#"{"path":"../x"}"#).await.0.starts_with("Ошибка"));
        assert_eq!(rel_clean(r".\.gitignore"), ".gitignore");
        assert_eq!(rel_clean("//src/a.py"), "src/a.py");
        // Путь с именем самой папки впереди — тот же файл.
        let with_name = format!(r#"{{"path":"{}/main.py"}}"#, tools.name);
        assert_eq!(tools.call("read_file", &with_name).await.1.path, "main.py");
    }
}
