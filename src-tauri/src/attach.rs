//! Вложения в чат: документ → текст, который увидит модель; картинка → копия в папке программы.
//!
//! Текст хранится в самой реплике разговора, а картинка — копией в `attachments\`, а не
//! ссылкой на исходный файл: файл потом переедут или удалят, а разговор должен
//! продолжаться с тем, что модель уже видела.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Больше — это уже не документ для разговора, а архив или образ диска.
const MAX_FILE: u64 = 100 << 20;
/// PDF на сотни страниц разбирается секунды; дольше минуты — файл с подвохом.
const PDF_TIMEOUT: Duration = Duration::from_secs(60);
/// Ключ запуска `ollivo.exe` как разборщика PDF (см. `helper_main`).
pub const PDF_HELPER_ARG: &str = "--pdf-text";
/// Фото с телефона — 3–8 МБ; больше — скорее всего не фото, а скан-«простыня».
const MAX_IMAGE: u64 = 20 << 20;
/// Сколько места картинка занимает в памяти модели. У Qwen3.5 с `--image-min-tokens 1024`
/// картинка 512×384 заняла ~1030 токенов (docs/phase-3.md); у больших картинок бывает
/// больше, но и память разговора тогда проверит сам движок.
pub const IMAGE_TOKENS: u64 = 1100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub name: String,
    /// `document` или `image`; расшифровка записи — `audio`.
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
    /// Копия картинки в папке программы.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// Прикидка без модели: 2,7 знака на токен — замер на русском тексте (`probe.rs`).
/// Для английского это с запасом: там знаков на токен больше.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.chars().count() as f64 / 2.7).ceil() as u64
}

/// Что за файл и как его читать — по расширению; неизвестное пробуем как текст.
/// Картинки копируются в `images`; HEIC и AVIF открывает `ffmpeg`, если он установлен.
pub fn read(path: &Path, images: &Path, ffmpeg: Option<&Path>) -> Result<Attachment, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|_| t!("файл не открывается — возможно, его переместили", "the file won't open — it may have been moved").to_string())?;
    if meta.is_dir() {
        return Err(t!("это папка, а не файл", "this is a folder, not a file").into());
    }
    if meta.len() > MAX_FILE {
        return Err(t!("файл больше 100 МБ — это слишком много для разговора", "the file is over 100 MB — too much for a conversation").into());
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let text = match ext.as_str() {
        "pdf" => pdf_text(path)?,
        "docx" => docx_text(path)?,
        "odt" => odt_text(path)?,
        "doc" | "rtf" => {
            return Err(t!(
                "старый формат Word — откройте файл в Word и сохраните как DOCX или PDF",
                "old Word format — open the file in Word and save it as DOCX or PDF"
            )
            .into())
        }
        "xls" | "xlsx" | "ods" => {
            return Err(t!("таблицы пока не читаю — сохраните таблицу как CSV", "spreadsheets aren't supported yet — save the table as CSV").into())
        }
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "heif" | "hif" | "avif" => {
            return store_image(path, name, images, meta.len(), ffmpeg)
        }
        // Записи и видео расшифровывает `speech.rs` — сюда они не доходят.
        _ => {
            let bytes = std::fs::read(path).map_err(|e| tf!("файл не читается: {e}", "the file can't be read: {e}"))?;
            if looks_binary(&bytes) {
                return Err(t!("это не текст и не документ — такие файлы модель прочесть не может", "this is neither text nor a document — the model can't read such files").into());
            }
            decode(&bytes)
        }
    };
    let text = tidy(&text);
    if text.trim().is_empty() {
        // Скан без текстового слоя: внутри PDF только картинки страниц.
        return Err(if ext == "pdf" {
            t!("в этом PDF нет текста — похоже, это отсканированные страницы", "this PDF has no text — it looks like scanned pages").into()
        } else {
            t!("в файле нет текста", "the file has no text").into()
        });
    }
    Ok(Attachment { name, kind: "document".into(), tokens: estimate_tokens(&text), text, trimmed: false, path: None })
}

/// Копия картинки под именем по её содержимому: одну и ту же картинку в десяти разговорах
/// храним один раз. Движок открывает картинки через stb_image: JPG, PNG, GIF, BMP он знает,
/// WebP и TIFF — нет, их перекодируем сами. В HEIC и AVIF — кадр HEVC и AV1: чистых
/// разборщиков на Rust нет, а кодеки Windows для них ставятся из Store и есть не у всех,
/// поэтому их открывает ffmpeg.
fn store_image(path: &Path, name: String, images: &Path, size: u64, ffmpeg: Option<&Path>) -> Result<Attachment, String> {
    use sha2::{Digest, Sha256};
    if size > MAX_IMAGE {
        return Err(t!("картинка больше 20 МБ — уменьшите её", "the image is over 20 MB — make it smaller").into());
    }
    let bytes = std::fs::read(path).map_err(|e| tf!("файл не читается: {e}", "the file can't be read: {e}"))?;
    // По первым байтам, а не по имени: бывает PNG, переименованный в .jpg.
    let format = sniff(&bytes).ok_or(t!("файл называется картинкой, но внутри не картинка", "the file is named like an image, but it isn't one inside"))?;
    let hash = hex::encode(Sha256::digest(&bytes));
    let stem = &hash[..16];
    // Уже прикладывали: перекодированная лежит под хешем исходного файла.
    let ready = ["jpg", "png", "gif", "bmp"].iter().map(|e| images.join(format!("{stem}.{e}"))).find(|p| p.exists());
    let dest = match ready {
        Some(p) => p,
        None => {
            let (bytes, ext) = match format {
                "webp" | "tiff" => convert(&bytes)?,
                "heif" => {
                    let ffmpeg = ffmpeg.ok_or(t!(
        "для таких картинок нужно докачать чтение файлов — приложите картинку ещё раз",
        "such images need an extra file reader to be downloaded — attach the image again"
    ))?;
                    convert(&crate::media::to_png(ffmpeg, path)?)?
                }
                "jpg" => upright_jpeg(&bytes).unwrap_or((bytes, "jpg")),
                other => (bytes, other),
            };
            let dest = images.join(format!("{stem}.{ext}"));
            std::fs::create_dir_all(images).map_err(|e| tf!("не удалось сохранить картинку: {e}", "could not save the image: {e}"))?;
            std::fs::write(&dest, &bytes).map_err(|e| tf!("не удалось сохранить картинку: {e}", "could not save the image: {e}"))?;
            dest
        }
    };
    Ok(Attachment {
        name,
        kind: "image".into(),
        text: String::new(),
        tokens: IMAGE_TOKENS,
        trimmed: false,
        path: Some(dest),
    })
}

/// Что за картинка по первым байтам; `None` — не картинка или формат, которого мы не знаем.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    Some(match bytes {
        [0xFF, 0xD8, 0xFF, ..] => "jpg",
        [0x89, b'P', b'N', b'G', ..] => "png",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        [b'B', b'M', ..] => "bmp",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        [b'I', b'I', b'*', 0, ..] | [b'M', b'M', 0, b'*', ..] => "tiff",
        // HEIC и AVIF: коробка `ftyp` и марка формата. Просто `ftyp` не годится — так же
        // начинается видео MP4.
        [_, _, _, _, b'f', b't', b'y', b'p', b0, b1, b2, b3, ..]
            if HEIF_BRANDS.contains(&[*b0, *b1, *b2, *b3]) =>
        {
            "heif"
        }
        _ => return None,
    })
}

/// Марки HEIF: HEIC с телефонов, AVIF, общие `mif1`/`msf1`.
const HEIF_BRANDS: &[[u8; 4]] = &[*b"heic", *b"heix", *b"heim", *b"heis", *b"hevc", *b"hevx", *b"mif1", *b"msf1", *b"avif", *b"avis"];

/// Качество JPG при перекодировании: мелкий текст на снимке экрана ещё читается,
/// а размер в разы меньше PNG.
const JPEG_QUALITY: u8 = 92;

/// Картинка, которую движок не откроет, → JPG или PNG. С прозрачностью — PNG, иначе
/// JPG: фото с телефона в PNG весит 15–20 МБ, а в разговоре оно уходит движку
/// с каждым вопросом заново.
fn convert(bytes: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
    let img = decode_upright(bytes)?;
    let transparent = img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p.0[3] < 255);
    let mut out = std::io::Cursor::new(Vec::new());
    let ext = if transparent {
        img.write_to(&mut out, image::ImageFormat::Png).map(|_| "png")
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .encode_image(&img.to_rgb8())
            .map(|_| "jpg")
    }
    .map_err(|e| tf!("не удалось перекодировать картинку: {e}", "could not convert the image: {e}"))?;
    Ok((out.into_inner(), ext))
}

/// Картинка с учётом поворота из EXIF. Телефон пишет кадр как держали камеру и помечает
/// «повернуть на 90°»; stb_image пометку не читает, и модель увидела бы фото на боку —
/// а надписи на боку она читает плохо.
fn decode_upright(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder;
    let broken = |e: image::ImageError| match e {
        image::ImageError::Limits(_) => t!("картинка слишком большая — уменьшите её", "the image is too big — make it smaller").to_string(),
        _ => t!("картинка повреждена — не удалось её открыть", "the image is damaged — couldn't open it").to_string(),
    };
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| t!("картинка повреждена — не удалось её открыть", "the image is damaged — couldn't open it").to_string())?
        .into_decoder()
        .map_err(broken)?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(broken)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// JPG с пометкой о повороте → повёрнутый JPG; `None` — поворачивать не нужно
/// (или не вышло — тогда пусть уходит как есть).
fn upright_jpeg(bytes: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    use image::ImageDecoder;
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    if decoder.orientation().ok()? == image::metadata::Orientation::NoTransforms {
        return None;
    }
    convert(bytes).ok()
}

/// Картинка как `data:`-адрес — так её принимает движок и показывает окно.
pub fn image_data_url(path: &Path) -> Result<String, String> {
    use base64::Engine;
    let bytes = std::fs::read(path).map_err(|_| t!("картинка потерялась", "the image is missing").to_string())?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "image/jpeg",
    };
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
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
    for f in files.iter().filter(|f| f.kind != "image") {
        let what = if f.kind == "audio" { t!("Расшифровка записи", "Recording transcript") } else { t!("Документ", "Document") };
        let note = if f.trimmed { t!(" (только начало — целиком не поместился)", " (only the beginning — it didn't fit whole)") } else { "" };
        out.push_str(&format!("{what} «{}»{note}:\n<<<\n{}\n>>>\n\n", f.name, f.text));
    }
    out.push_str(question);
    out
}

/// Удаляет из `dir` файлы старше `older_than`, кроме тех, что `keep`. Возвращает, сколько
/// удалено. Ошибки молча пропускаем: занятый файл уберём в следующий раз.
pub fn sweep(dir: &Path, older_than: Duration, keep: impl Fn(&Path) -> bool) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let now = std::time::SystemTime::now();
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            let age = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok());
            age.is_some_and(|a| a > older_than)
        })
        .filter(|e| !keep(&e.path()))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

// ---------- Текст ----------

/// Нулевые байты в начале — верный признак двоичного файла. UTF-16 их тоже содержит,
/// но у него есть метка порядка байтов, её проверяем раньше.
pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    if encoding_rs::Encoding::for_bom(bytes).is_some() {
        return false;
    }
    bytes.iter().take(8192).any(|&b| b == 0)
}

/// UTF-8 или UTF-16 с меткой — как есть. Иначе, если не UTF-8, — Windows-1251:
/// в ней сохранены старые русские текстовые файлы из Блокнота.
pub(crate) fn decode(bytes: &[u8]) -> String {
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
    let broken = || t!("документ повреждён или это не документ Word", "the document is damaged or isn't a Word document").to_string();
    let file = std::fs::File::open(path).map_err(|e| tf!("файл не читается: {e}", "the file can't be read: {e}"))?;
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
        .map_err(|e| tf!("не удалось открыть PDF: {e}", "could not open the PDF: {e}"))?;
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
            return Err(t!("PDF разбирается слишком долго — возможно, он повреждён", "the PDF takes too long to parse — it may be damaged").into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = reader.join().unwrap_or_default();
    if !status.success() {
        return Err(t!("PDF не читается — возможно, он повреждён или защищён паролем", "the PDF can't be read — it may be damaged or password-protected").into());
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
        let dir = crate::testserver::tmp().join(format!("ollivo-attach-{}", std::process::id()));
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
            assert_eq!(read(&p, &tmp("images"), None).unwrap().text, "Привет, мир", "{name}");
        }
    }

    #[test]
    fn code_file_is_text_binary_is_refused() {
        let p = tmp("main.rs");
        std::fs::write(&p, "fn main() {\r\n    println!(\"hi\");   \r\n}\r\n\r\n\r\n\r\n").unwrap();
        let a = read(&p, &tmp("images"), None).unwrap();
        assert_eq!(a.text, "fn main() {\n    println!(\"hi\");\n}");
        assert_eq!(a.name, "main.rs");
        assert!(a.tokens > 0);

        let p = tmp("data.bin");
        std::fs::write(&p, [0x4D, 0x5A, 0x90, 0x00, 0x03]).unwrap();
        assert!(read(&p, &tmp("images"), None).unwrap_err().contains("не текст"));
    }

    #[test]
    fn old_formats_get_advice() {
        let p = tmp("old.doc");
        std::fs::write(&p, b"x").unwrap();
        assert!(read(&p, &tmp("images"), None).unwrap_err().contains("DOCX"));
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
        assert_eq!(read(&p, &tmp("images"), None).unwrap().text, "Договор & условия\nЦена: 200\tруб.\n\nА  | Б  |");
    }

    #[test]
    fn odt_text_without_styles() {
        let xml = r#"<?xml version="1.0"?><office:document-content>
<office:automatic-styles><style:style style:name="P1">стиль</style:style></office:automatic-styles>
<office:body><office:text><text:h>Глава&#160;1</text:h><text:p>Первый<text:s/>абзац</text:p></office:text></office:body>
</office:document-content>"#;
        let p = tmp("kniga.odt");
        zip_with(&p, "content.xml", xml);
        assert_eq!(read(&p, &tmp("images"), None).unwrap().text, "Глава\u{a0}1\nПервый абзац");
    }

    #[test]
    fn broken_docx_is_explained() {
        let p = tmp("broken.docx");
        std::fs::write(&p, b"PK\x03\x04 not really").unwrap();
        assert!(read(&p, &tmp("images"), None).unwrap_err().contains("повреждён"));
    }

    #[test]
    fn trim_keeps_whole_paragraphs() {
        let text = (1..=20).map(|i| format!("Абзац номер {i}. Здесь немного текста.")).collect::<Vec<_>>().join("\n\n");
        let a = Attachment { name: "a.txt".into(), kind: "document".into(), tokens: estimate_tokens(&text), text, trimmed: false, path: None };
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
        let doc = Attachment { name: "план.txt".into(), kind: "document".into(), text: "пункт 1".into(), tokens: 3, trimmed: true, path: None };
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

    #[test]
    fn sweep_removes_only_old_and_unused() {
        let dir = tmp("sweep");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(40 * 86400);
        for name in ["old-unused.png", "old-used.png", "fresh.png"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        for name in ["old-unused.png", "old-used.png"] {
            std::fs::File::options().write(true).open(dir.join(name)).unwrap().set_modified(old).unwrap();
        }
        let n = sweep(&dir, Duration::from_secs(86400), |p| p.ends_with("old-used.png"));
        assert_eq!(n, 1);
        assert!(!dir.join("old-unused.png").exists());
        assert!(dir.join("old-used.png").exists() && dir.join("fresh.png").exists());
    }

    #[test]
    fn image_is_copied_once_and_checked() {
        let images = tmp("imgs");
        let png = [&b"\x89PNG\r\n\x1a\n"[..], &[0u8; 32]].concat();
        let p = tmp("фото.png");
        std::fs::write(&p, &png).unwrap();
        let a = read(&p, &images, None).unwrap();
        assert_eq!((a.kind.as_str(), a.name.as_str()), ("image", "фото.png"));
        let stored = a.path.clone().unwrap();
        assert!(stored.starts_with(&images) && stored.exists());
        // Та же картинка под другим именем — та же копия.
        let p2 = tmp("копия.PNG");
        std::fs::write(&p2, &png).unwrap();
        assert_eq!(read(&p2, &images, None).unwrap().path, Some(stored.clone()));
        assert!(image_data_url(&stored).unwrap().starts_with("data:image/png;base64,iVBORw0KGgo"));

        let fake = tmp("fake.jpg");
        std::fs::write(&fake, b"not an image").unwrap();
        assert!(read(&fake, &images, None).unwrap_err().contains("не картинка"));
        // HEIC без ffmpeg не открыть; MP4, названный .heic, — не картинка.
        let heic = tmp("pic.heic");
        std::fs::write(&heic, b"\0\0\0\x18ftypheic\0\0\0\0mif1heic").unwrap();
        assert!(read(&heic, &images, None).unwrap_err().contains("докачать"));
        std::fs::write(&heic, b"\0\0\0\x18ftypisom\0\0\0\0").unwrap();
        assert!(read(&heic, &images, None).unwrap_err().contains("не картинка"));
    }

    fn encoded(img: &image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn stored(name: &str, bytes: &[u8]) -> image::DynamicImage {
        let p = tmp(name);
        std::fs::write(&p, bytes).unwrap();
        let a = read(&p, &tmp("conv"), None).unwrap();
        assert_eq!(a.name, name);
        let path = a.path.unwrap();
        assert!(matches!(sniff(&std::fs::read(&path).unwrap()), Some("jpg" | "png")), "{path:?}");
        image::open(&path).unwrap()
    }

    /// WebP и TIFF движок не открывает — они перекодируются; прозрачность сохраняется.
    #[test]
    fn webp_and_tiff_are_converted() {
        let photo = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 48, |x, y| {
            image::Rgb([x as u8 * 4, y as u8 * 5, 90])
        }));
        let mut logo = image::RgbaImage::from_pixel(32, 32, image::Rgba([200, 30, 30, 255]));
        logo.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let logo = image::DynamicImage::ImageRgba8(logo);

        let a = stored("фото.webp", &encoded(&photo, image::ImageFormat::WebP));
        assert_eq!((a.width(), a.height(), a.color().has_alpha()), (64, 48, false));
        let b = stored("лого.webp", &encoded(&logo, image::ImageFormat::WebP));
        assert_eq!((b.width(), b.to_rgba8().get_pixel(0, 0).0[3]), (32, 0));
        let c = stored("скан.tiff", &encoded(&photo, image::ImageFormat::Tiff));
        assert_eq!((c.width(), c.height()), (64, 48));
        // Цвета после JPG — почти те же.
        let (was, now) = (photo.to_rgb8().get_pixel(40, 30).0, c.to_rgb8().get_pixel(40, 30).0);
        assert!(was.iter().zip(now).all(|(a, b)| a.abs_diff(b) < 12), "{was:?} {now:?}");

        let broken = tmp("битая.webp");
        std::fs::write(&broken, b"RIFF\0\0\0\0WEBPVP8 garbage").unwrap();
        assert!(read(&broken, &tmp("conv"), None).unwrap_err().contains("повреждена"));
    }

    /// Фото с телефона, снятое «портретом»: кадр 40×20 и пометка EXIF «повернуть на 90°».
    #[test]
    fn phone_photo_is_turned_upright() {
        let wide = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(40, 20, image::Rgb([10, 120, 200])));
        let jpg = encoded(&wide, image::ImageFormat::Jpeg);
        // APP1 с EXIF: один тег Orientation (0x0112) = 6, сразу после начала файла.
        let exif: &[u8] = b"Exif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
        let len = (exif.len() + 2) as u16;
        let rotated = [&jpg[..2], &[0xFF, 0xE1], &len.to_be_bytes(), exif, &jpg[2..]].concat();
        let img = stored("портрет.jpg", &rotated);
        assert_eq!((img.width(), img.height()), (20, 40));
        // Без пометки — файл уходит как есть, байт в байт.
        let p = tmp("ровно.jpg");
        std::fs::write(&p, &jpg).unwrap();
        assert_eq!(std::fs::read(read(&p, &tmp("conv"), None).unwrap().path.unwrap()).unwrap(), jpg);
    }
}
