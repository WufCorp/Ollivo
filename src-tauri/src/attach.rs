//! Вложения в чат: документ → текст, который увидит модель.
//!
//! Текст хранится в самой реплике разговора, а не ссылкой на файл: файл потом переедут
//! или удалят, а разговор должен продолжаться с тем же, что модель уже прочла.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Больше — это уже не документ для разговора, а архив или образ диска.
const MAX_FILE: u64 = 100 << 20;
/// PDF на сотни страниц разбирается секунды; дольше минуты — файл с подвохом.
const PDF_TIMEOUT: Duration = Duration::from_secs(60);
/// Ключ запуска `ollivo.exe` как разборщика PDF (см. `helper_main`).
pub const PDF_HELPER_ARG: &str = "--pdf-text";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub name: String,
    /// Пока только `document`; дальше — `image`.
    pub kind: String,
    /// Текст документа — ровно то, что получит модель.
    #[serde(default)]
    pub text: String,
    /// Сколько токенов занимает текст: точно, если модель запущена, иначе прикидка.
    #[serde(default)]
    pub tokens: u64,
    /// Приложено только начало: целиком документ не поместился в память модели.
    #[serde(default)]
    pub trimmed: bool,
}

/// Прикидка без модели: 2,7 знака на токен — замер на русском тексте (`probe.rs`).
/// Для английского это с запасом: там знаков на токен больше.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.chars().count() as f64 / 2.7).ceil() as u64
}

/// Что за файл и как его читать — по расширению; неизвестное пробуем как текст.
pub fn read(path: &Path) -> Result<Attachment, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|_| "файл не открывается — возможно, его переместили".to_string())?;
    if meta.is_dir() {
        return Err("это папка, а не файл".into());
    }
    if meta.len() > MAX_FILE {
        return Err("файл больше 100 МБ — это слишком много для разговора".into());
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let text = match ext.as_str() {
        "pdf" => pdf_text(path)?,
        "docx" => docx_text(path)?,
        "odt" => odt_text(path)?,
        "doc" | "rtf" => {
            return Err("старый формат Word — откройте файл в Word и сохраните как DOCX или PDF".into())
        }
        "xls" | "xlsx" | "ods" => return Err("таблицы пока не читаю — сохраните таблицу как CSV".into()),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" => return Err("картинки в чат — скоро".into()),
        "mp3" | "wav" | "ogg" | "m4a" | "flac" | "opus" => return Err("расшифровка аудио — скоро".into()),
        _ => {
            let bytes = std::fs::read(path).map_err(|e| format!("файл не читается: {e}"))?;
            if looks_binary(&bytes) {
                return Err("это не текст и не документ — такие файлы модель прочесть не может".into());
            }
            decode(&bytes)
        }
    };
    let text = tidy(&text);
    if text.trim().is_empty() {
        // Скан без текстового слоя: внутри PDF только картинки страниц.
        return Err(if ext == "pdf" {
            "в этом PDF нет текста — похоже, это отсканированные страницы".into()
        } else {
            "в файле нет текста".into()
        });
    }
    Ok(Attachment { name, kind: "document".into(), tokens: estimate_tokens(&text), text, trimmed: false })
}

/// Только начало документа, `max_tokens` из `tokens`. Режем по границе абзаца,
/// чтобы модель не получила оборванную на полуслове фразу.
pub fn trim(mut a: Attachment, max_tokens: u64) -> Attachment {
    if a.tokens <= max_tokens || a.tokens == 0 {
        return a;
    }
    let chars = a.text.chars().count() as u64;
    let keep = (chars * max_tokens / a.tokens) as usize;
    let cut = a.text.char_indices().nth(keep).map_or(a.text.len(), |(i, _)| i);
    let head = &a.text[..cut];
    // Абзац, если он недалеко; иначе хотя бы конец строки или предложения.
    let end = head
        .rfind("\n\n")
        .filter(|&i| i > cut * 3 / 4)
        .or_else(|| head.rfind('\n').filter(|&i| i > cut * 9 / 10))
        .or_else(|| head.rfind(". ").map(|i| i + 1).filter(|&i| i > cut * 9 / 10))
        .unwrap_or(cut);
    a.text = a.text[..end].trim_end().to_string();
    a.tokens = a.tokens * a.text.chars().count() as u64 / chars.max(1);
    a.trimmed = true;
    a
}

/// Как документ попадает к модели: имя файла и текст в явных границах, чтобы модель
/// не спутала документ с вопросом.
pub fn for_model(files: &[Attachment], question: &str) -> String {
    let mut out = String::new();
    for f in files.iter().filter(|f| f.kind == "document") {
        let note = if f.trimmed { " (только начало — целиком не поместился)" } else { "" };
        out.push_str(&format!("Документ «{}»{note}:\n<<<\n{}\n>>>\n\n", f.name, f.text));
    }
    out.push_str(question);
    out
}

// ---------- Текст ----------

/// Нулевые байты в начале — верный признак двоичного файла. UTF-16 их тоже содержит,
/// но у него есть метка порядка байтов, её проверяем раньше.
fn looks_binary(bytes: &[u8]) -> bool {
    if encoding_rs::Encoding::for_bom(bytes).is_some() {
        return false;
    }
    bytes.iter().take(8192).any(|&b| b == 0)
}

/// UTF-8 или UTF-16 с меткой — как есть. Иначе, если не UTF-8, — Windows-1251:
/// в ней сохранены старые русские текстовые файлы из Блокнота.
fn decode(bytes: &[u8]) -> String {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode_without_bom_handling(&bytes[bom..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1251.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

/// `\r\n` → `\n`, без хвостовых пробелов и лишних пустых строк: они только съедают память модели.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

// ---------- DOCX и ODT ----------

fn zip_entry(path: &Path, entry: &str) -> Result<String, String> {
    let broken = || "документ повреждён или это не документ Word".to_string();
    let file = std::fs::File::open(path).map_err(|e| format!("файл не читается: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| broken())?;
    let mut f = zip.by_name(entry).map_err(|_| broken())?;
    let mut xml = String::new();
    f.read_to_string(&mut xml).map_err(|_| broken())?;
    Ok(xml)
}

fn docx_text(path: &Path) -> Result<String, String> {
    Ok(docx_from_xml(&zip_entry(path, "word/document.xml")?))
}

/// Текст — только в `w:t`: `w:delText` — удалённое правкой, `w:instrText` — коды полей.
/// Абзац в ячейке таблицы — пробел, а не новая строка, иначе строка таблицы рассыпается.
fn docx_from_xml(xml: &str) -> String {
    let mut out = String::new();
    let (mut in_t, mut in_cell) = (false, 0);
    walk(xml, |tok| match tok {
        Tok::Open("w:t") => in_t = true,
        Tok::Close("w:t") => in_t = false,
        Tok::Text(t) if in_t => out.push_str(&t),
        Tok::Empty("w:tab") => out.push('\t'),
        Tok::Empty("w:br" | "w:cr") => out.push('\n'),
        Tok::Open("w:tc") => in_cell += 1,
        Tok::Close("w:tc") => {
            in_cell -= 1;
            out.push_str(" | ");
        }
        Tok::Close("w:tr") => out.push('\n'),
        Tok::Close("w:p") => out.push(if in_cell > 0 { ' ' } else { '\n' }),
        _ => {}
    });
    out
}

fn odt_text(path: &Path) -> Result<String, String> {
    Ok(odt_from_xml(&zip_entry(path, "content.xml")?))
}

/// Текст ODT — прямо внутри абзацев и их частей, всё до `office:body` — стили.
fn odt_from_xml(xml: &str) -> String {
    let mut out = String::new();
    let mut in_body = false;
    walk(xml, |tok| match tok {
        Tok::Open("office:body") => in_body = true,
        Tok::Close("office:body") => in_body = false,
        Tok::Text(t) if in_body => out.push_str(&t),
        Tok::Empty("text:s") => out.push(' '),
        Tok::Empty("text:tab") => out.push('\t'),
        Tok::Empty("text:line-break") => out.push('\n'),
        Tok::Close("text:p" | "text:h") => out.push('\n'),
        Tok::Close("table:table-cell") => out.push_str(" | "),
        _ => {}
    });
    out
}

enum Tok<'a> {
    Open(&'a str),
    Close(&'a str),
    Empty(&'a str),
    Text(String),
}

/// Простейший проход по XML: документы Word и LibreOffice пишутся программой, без
/// CDATA и хитростей, и полноценный разборщик ради них не нужен.
fn walk<'a>(xml: &'a str, mut on: impl FnMut(Tok<'a>)) {
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        if lt > 0 {
            on(Tok::Text(unescape(&rest[..lt])));
        }
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |i| &rest[i + 3..]);
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let name_of = |s: &'a str| s.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if let Some(name) = tag.strip_prefix('/') {
            on(Tok::Close(name.trim()));
        } else if tag.ends_with('/') {
            on(Tok::Empty(name_of(tag)));
        } else {
            on(Tok::Open(name_of(tag)));
        }
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|&i| i < 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------- PDF ----------

/// PDF разбирается в отдельном процессе — этой же программой с ключом `--pdf-text`.
/// Разборщик на кривом файле может запаниковать или зациклиться, а в release-сборке
/// паника сразу закрывает программу (`panic = "abort"`): упал бы весь Ollivo с разговором.
fn pdf_text(path: &Path) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg(PDF_HELPER_ARG)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("не удалось открыть PDF: {e}"))?;
    let mut stdout = child.stdout.take().unwrap();
    // Читаем в отдельном потоке: большой текст забьёт канал, и разборщик встанет,
    // пока мы ждём его завершения.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if started.elapsed() > PDF_TIMEOUT {
            let _ = child.kill();
            return Err("PDF разбирается слишком долго — возможно, он повреждён".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = reader.join().unwrap_or_default();
    if !status.success() {
        return Err("PDF не читается — возможно, он повреждён или защищён паролем".into());
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Сам разбор PDF — вызывается только в процессе-помощнике (и в тестах).
pub fn pdf_text_here(path: &Path) -> Result<String, String> {
    pdf_extract::extract_text(path).map_err(|e| e.to_string())
}

/// Если программу запустили разборщиком PDF — делает дело и возвращает код выхода.
/// Вызывается из `main` до запуска окна.
pub fn helper_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != PDF_HELPER_ARG {
        return None;
    }
    let path = args.next()?;
    Some(match pdf_text_here(Path::new(&path)) {
        Ok(text) => {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            if out.write_all(text.as_bytes()).is_ok() && out.flush().is_ok() {
                0
            } else {
                1
            }
        }
        Err(_) => 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ollivo-attach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// Старый Блокнот сохранял по-русски в Windows-1251, новый — в UTF-8, иногда с меткой.
    #[test]
    fn text_in_any_encoding() {
        let (w1251, _, _) = encoding_rs::WINDOWS_1251.encode("Привет, мир");
        for (name, bytes) in [
            ("utf8.txt", "Привет, мир".as_bytes().to_vec()),
            ("bom.txt", [&[0xEF, 0xBB, 0xBF][..], "Привет, мир".as_bytes()].concat()),
            ("cp1251.txt", w1251.into_owned()),
            (
                "utf16.txt",
                [vec![0xFF, 0xFE], "Привет, мир".encode_utf16().flat_map(|u| u.to_le_bytes()).collect()].concat(),
            ),
        ] {
            let p = tmp(name);
            std::fs::write(&p, bytes).unwrap();
            assert_eq!(read(&p).unwrap().text, "Привет, мир", "{name}");
        }
    }

    #[test]
    fn code_file_is_text_binary_is_refused() {
        let p = tmp("main.rs");
        std::fs::write(&p, "fn main() {\r\n    println!(\"hi\");   \r\n}\r\n\r\n\r\n\r\n").unwrap();
        let a = read(&p).unwrap();
        assert_eq!(a.text, "fn main() {\n    println!(\"hi\");\n}");
        assert_eq!(a.name, "main.rs");
        assert!(a.tokens > 0);

        let p = tmp("data.bin");
        std::fs::write(&p, [0x4D, 0x5A, 0x90, 0x00, 0x03]).unwrap();
        assert!(read(&p).unwrap_err().contains("не текст"));
    }

    #[test]
    fn old_formats_get_advice() {
        let p = tmp("old.doc");
        std::fs::write(&p, b"x").unwrap();
        assert!(read(&p).unwrap_err().contains("DOCX"));
    }

    fn zip_with(path: &Path, entry: &str, xml: &str) {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        z.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(xml.as_bytes()).unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn docx_paragraphs_tables_and_edits() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="x"><w:body>
<w:p><w:r><w:t>Договор &amp; условия</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Цена: </w:t></w:r><w:r><w:delText>100</w:delText></w:r><w:r><w:t>200</w:t><w:tab/><w:t>руб.</w:t></w:r></w:p>
<w:p><w:r><w:instrText> PAGE </w:instrText></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>А</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Б</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#;
        let p = tmp("dogovor.docx");
        zip_with(&p, "word/document.xml", xml);
        assert_eq!(read(&p).unwrap().text, "Договор & условия\nЦена: 200\tруб.\n\nА  | Б  |");
    }

    #[test]
    fn odt_text_without_styles() {
        let xml = r#"<?xml version="1.0"?><office:document-content>
<office:automatic-styles><style:style style:name="P1">стиль</style:style></office:automatic-styles>
<office:body><office:text><text:h>Глава&#160;1</text:h><text:p>Первый<text:s/>абзац</text:p></office:text></office:body>
</office:document-content>"#;
        let p = tmp("kniga.odt");
        zip_with(&p, "content.xml", xml);
        assert_eq!(read(&p).unwrap().text, "Глава\u{a0}1\nПервый абзац");
    }

    #[test]
    fn broken_docx_is_explained() {
        let p = tmp("broken.docx");
        std::fs::write(&p, b"PK\x03\x04 not really").unwrap();
        assert!(read(&p).unwrap_err().contains("повреждён"));
    }

    #[test]
    fn trim_keeps_whole_paragraphs() {
        let text = (1..=20).map(|i| format!("Абзац номер {i}. Здесь немного текста.")).collect::<Vec<_>>().join("\n\n");
        let a = Attachment { name: "a.txt".into(), kind: "document".into(), tokens: estimate_tokens(&text), text, trimmed: false };
        let half = a.tokens / 2;
        let t = trim(a.clone(), half);
        assert!(t.trimmed && t.tokens <= half, "{} > {half}", t.tokens);
        assert!(t.text.ends_with("текста."), "{}", t.text);
        assert!(a.text.starts_with(&t.text));
        // Помещается — не трогаем.
        assert_eq!(trim(a.clone(), a.tokens), a);
    }

    #[test]
    fn model_sees_document_in_borders() {
        let doc = Attachment { name: "план.txt".into(), kind: "document".into(), text: "пункт 1".into(), tokens: 3, trimmed: true };
        let s = for_model(&[doc], "О чём план?");
        assert_eq!(s, "Документ «план.txt» (только начало — целиком не поместился):\n<<<\nпункт 1\n>>>\n\nО чём план?");
    }

    /// PDF из Edge («Печать в PDF») — шрифты со своей кодировкой, как у большинства PDF
    /// из браузеров и Word. Разбор в процессе-помощнике проверяется только в собранной
    /// программе: в тестах `current_exe` — сам тест.
    #[test]
    fn pdf_russian_text() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/borsch.pdf");
        let text = tidy(&pdf_text_here(&p).unwrap());
        println!("{text}");
        assert!(text.contains("Рецепт борща"), "{text}");
        assert!(text.contains("Свёкла"), "{text}");
        assert!(text.contains("Hello, world!"), "{text}");
    }
}
