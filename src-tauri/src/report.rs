//! «Сообщить о проблеме»: отчёт для того, кто будет помогать.
//!
//! Сборки выкладываются на GitHub с 0.1, и тестеры — незнакомые люди: без отчёта из
//! программы «не работает» не разобрать. Отчёт собирает ядро, человек видит его целиком
//! до отправки и отправляет сам. Переписки и файлов в нём нет, пароли и токены лежат
//! в диспетчере учётных данных и сюда не попадают, имя пользователя в путях заменено.
//!
//! Отчёт — текстовый файл, а не zip: человек может открыть его Блокнотом и увидеть,
//! что уходит, а GitHub принимает `.txt` в форме issue так же, как архив.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Куда идут отчёты.
pub const REPO: &str = "https://github.com/WufCorp/Ollivo";
/// Группы поддержки Ollivo в Telegram и MAX: аккаунт GitHub есть не у всех,
/// а мессенджер — почти у каждого.
pub const TELEGRAM: &str = "https://t.me/ollivo_support";
pub const MAX: &str = "https://max.ru/join/vLWwbSCYaXnZYv-JFM3gVDuKnVC81BieJoCMCuZ8nVc";

/// Куда человек отправляет отчёт.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Github,
    Telegram,
    Max,
}
/// Сколько строк журнала движка брать: хватает, чтобы увидеть запуск и падение.
const LOG_LINES: usize = 120;
/// Сколько последних ошибок программы брать.
const PROBLEMS: usize = 30;
/// Длина строки журнала: длинные строки — это уже не служебные сообщения,
/// а куски текста (ответ модели, содержимое файла), им в отчёте не место.
const LINE_MAX: usize = 300;
/// Журнал ошибок больше этого — оставляем вторую половину.
const PROBLEMS_FILE_MAX: u64 = 256 << 10;

/// Что случилось, со слов человека: от этого зависит форма issue на GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Не ставится программа или движок.
    Install,
    /// Модель не запускается или падает.
    Model,
    Other,
}

impl Kind {
    fn template(self) -> &'static str {
        match self {
            Kind::Install => "install.yml",
            Kind::Model => "model.yml",
            Kind::Other => "other.yml",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Kind::Install => t!("Не ставится", "Won't install"),
            Kind::Model => t!("Модель не запускается", "Model won't start"),
            Kind::Other => t!("Другое", "Other"),
        }
    }
}

/// Всё, что ядро знает о программе и компьютере. Собирает `lib.rs`: здесь только текст.
#[derive(Debug, Default)]
pub struct Facts {
    pub version: String,
    pub channel: String,
    pub windows: String,
    pub cpu: String,
    pub hardware: Option<crate::hardware::Hardware>,
    pub vulkan: bool,
    pub vc_runtime: bool,
    pub data_dir: PathBuf,
    /// Движки: id, и что стоит (`None` — не установлен).
    pub engines: Vec<(String, Option<crate::engines::Installed>)>,
    /// Запущенная модель: файл, память разговора, слоёв на видеокарте, зрение, инструменты.
    pub model: Option<String>,
    /// `settings.json` как есть; логин прокси заменяется здесь.
    pub settings: serde_json::Value,
    /// Журналы движков: название и путь.
    pub logs: Vec<(String, PathBuf)>,
    /// Журнал ошибок программы (`note`).
    pub problems: PathBuf,
}

/// Готовый отчёт: коротко — в форму issue, целиком — в файл.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Report {
    pub summary: String,
    pub full: String,
}

pub fn build(f: &Facts, now: SystemTime) -> Report {
    let gb = |b: u64| crate::i18n::decimal(format!("{:.1} {}", b as f64 / 1e9, t!("ГБ", "GB")));
    let yes = |b: bool| if b { t!("есть", "yes") } else { t!("нет", "no") };
    let free = t!("свободно", "free");

    let mut gpu_line = t!("видеокарта NVIDIA не найдена", "no NVIDIA graphics card found").to_string();
    let mut driver_line = String::new();
    if let Some(hw) = &f.hardware {
        if let Some(g) = &hw.gpu {
            gpu_line = format!(
                "{}, {} ({free} {}), CC {}.{}",
                g.name,
                gb(g.vram_total),
                gb(g.vram_free),
                g.cc.0,
                g.cc.1
            );
            // Без этой пометки отчёт с Tesla выглядел как обычный — а Vulkan карту не видел.
            if g.compute_only {
                gpu_line += t!(", без графики (TCC) — Vulkan её не видит", ", no graphics (TCC) — Vulkan can't see it");
            }
        }
        driver_line = tf!(
            "{}, CUDA {}.{} (по драйверу подходит сборка {})",
            "{}, CUDA {}.{} (the driver fits build {})",
            if hw.driver.is_empty() { "?" } else { &hw.driver },
            hw.cuda_driver / 1000,
            hw.cuda_driver % 1000 / 10,
            build_name(hw.cuda_build)
        );
    }
    let ram = f.hardware.as_ref().map_or(String::new(), |hw| format!("{} ({free} {})", gb(hw.ram_total), gb(hw.ram_avail)));
    let last_problem = tail_lines(&f.problems, 1);

    let mut summary = format!("Ollivo {} ({}), {}\n", f.version, f.channel, f.windows);
    let (l_gpu, l_driver, l_ram) = (t!("Видеокарта", "GPU"), t!("Драйвер", "Driver"), t!("Память", "Memory"));
    summary += &format!("{l_gpu}: {gpu_line}\n");
    if !driver_line.is_empty() {
        summary += &format!("{l_driver}: {driver_line}\n");
    }
    summary += &format!("{l_ram}: {ram}\n");
    for (id, e) in &f.engines {
        if let Some(e) = e {
            summary += &format!("{id} {} ({})\n", e.version, build_name(e.build));
        }
    }
    if !last_problem.is_empty() {
        summary += &tf!("Последняя ошибка: {last_problem}\n", "Last error: {last_problem}\n");
    }

    let mut t = String::new();
    let _ = writeln!(t, "{}", tf!("Отчёт Ollivo — {} UTC", "Ollivo report — {} UTC", stamp(now)));
    t += t!(
        "Этот файл собрала программа. Ваших разговоров, файлов и паролей в нём нет,\n\
         имя пользователя в путях заменено на %USERNAME%.\n",
        "This file was put together by the program. Your conversations, files and passwords are not in it,\n\
         the user name in paths is replaced with %USERNAME%.\n"
    );

    let _ = writeln!(t, "\n== {} ==", t!("Программа", "Program"));
    let _ = writeln!(t, "{}", tf!("Ollivo {}, канал обновлений {}", "Ollivo {}, update channel {}", f.version, f.channel));
    let _ = writeln!(t, "Windows: {}", f.windows);
    let _ = writeln!(t, "{}", tf!("Папка программы: {}", "Program folder: {}", f.data_dir.display()));

    let _ = writeln!(t, "\n== {} ==", t!("Компьютер", "Computer"));
    let _ = writeln!(t, "{l_gpu}: {gpu_line}");
    if !driver_line.is_empty() {
        let _ = writeln!(t, "{l_driver}: {driver_line}");
    }
    let _ = writeln!(t, "{}: {}", t!("Процессор", "CPU"), f.cpu);
    let _ = writeln!(t, "{l_ram}: {ram}");
    if let Some(hw) = &f.hardware {
        let disks: Vec<String> = hw.disks.iter().map(|d| format!("{} {} ({free} {})", d.mount, gb(d.total), gb(d.free))).collect();
        let _ = writeln!(t, "{}: {}", t!("Диски", "Disks"), disks.join(", "));
        let risky = if hw.profile_risky { t!("да", "yes") } else { t!("нет", "no") };
        let _ = writeln!(t, "{}: {risky}", t!("Кириллица или пробел в профиле", "Cyrillic or space in the profile"));
    }
    let _ = writeln!(t, "{}", tf!("Vulkan в системе: {}, VC++ Runtime: {}", "Vulkan in the system: {}, VC++ Runtime: {}", yes(f.vulkan), yes(f.vc_runtime)));

    let _ = writeln!(t, "\n== {} ==", t!("Движки", "Engines"));
    for (id, e) in &f.engines {
        match e {
            Some(e) => {
                let _ = writeln!(t, "{id} {} ({}) — {}", e.version, build_name(e.build), e.dir.display());
            }
            None => {
                let _ = writeln!(t, "{id} — {}", t!("не установлен", "not installed"));
            }
        }
    }

    let _ = writeln!(t, "\n== {} ==", t!("Модель", "Model"));
    let _ = writeln!(t, "{}", f.model.as_deref().unwrap_or(t!("не запущена", "not running")));

    let _ = writeln!(t, "\n== {} ==", t!("Настройки", "Settings"));
    let _ = writeln!(t, "{}", serde_json::to_string_pretty(&without_secrets(f.settings.clone())).unwrap_or_default());

    let _ = writeln!(t, "\n== {} ==", tf!("Последние ошибки программы (до {PROBLEMS})", "Latest program errors (up to {PROBLEMS})"));
    let problems = tail_lines(&f.problems, PROBLEMS);
    let _ = writeln!(t, "{}", if problems.is_empty() { t!("нет", "none") } else { &problems });

    for (name, path) in &f.logs {
        let _ = writeln!(t, "\n== {} ==", tf!("Журнал {name} (последние {LOG_LINES} строк)", "Log {name} (last {LOG_LINES} lines)"));
        let log = tail_lines(path, LOG_LINES);
        let _ = writeln!(t, "{}", if log.is_empty() { t!("пусто", "empty") } else { &log });
    }

    Report { summary: scrub_env(&summary), full: scrub_env(&t) }
}

fn build_name(b: crate::hardware::Build) -> String {
    serde_json::to_value(b).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

/// Настройки без того, что может выдать человека: логин прокси. Пароль прокси и токен
/// HF в `settings.json` не лежат вовсе.
fn without_secrets(mut s: serde_json::Value) -> serde_json::Value {
    if let Some(user) = s.pointer_mut("/proxy/username") {
        if user.as_str().is_some_and(|u| !u.is_empty()) {
            *user = "(скрыт)".into();
        }
    }
    s
}

/// Последние строки файла, длинные — обрезаны.
fn tail_lines(path: &Path, lines: usize) -> String {
    crate::process::log_tail(path, lines)
        .lines()
        .map(|l| match l.char_indices().nth(LINE_MAX) {
            Some((i, _)) => format!("{}…", &l[..i]),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Имя пользователя из путей: папку профиля — на `%USERPROFILE%`, само имя — на `%USERNAME%`.
fn scrub_env(text: &str) -> String {
    scrub(
        text,
        std::env::var("USERPROFILE").ok().as_deref(),
        std::env::var("USERNAME").ok().as_deref(),
    )
}

pub fn scrub(text: &str, profile: Option<&str>, user: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(p) = profile.filter(|p| p.len() > 3) {
        out = replace_ci(&out, p, "%USERPROFILE%");
        out = replace_ci(&out, &p.replace('\\', "/"), "%USERPROFILE%");
    }
    // Короткое имя вроде «ai» совпало бы с половиной слов в журнале.
    if let Some(u) = user.filter(|u| u.chars().count() >= 3) {
        out = replace_ci(&out, u, "%USERNAME%");
    }
    out
}

/// Замена без учёта регистра: Windows не различает `C:\Users\Ivan` и `c:\users\ivan`.
fn replace_ci(text: &str, needle: &str, with: &str) -> String {
    let (low, pat) = (text.to_lowercase(), needle.to_lowercase());
    // Строчные буквы бывают другой длины в байтах (редкие символы) — тогда только точное совпадение.
    if low.len() != text.len() || pat.len() != needle.len() || pat.is_empty() {
        return text.replace(needle, with);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (i, _) in low.match_indices(&pat) {
        if i < last || !text.is_char_boundary(i) {
            continue;
        }
        out += &text[last..i];
        out += with;
        last = i + pat.len();
    }
    out + &text[last..]
}

/// Адрес формы issue на GitHub с заполненными полями. Слова человека — в поле «Что
/// случилось», коротко о компьютере — в «Сводка». Полный отчёт человек прикладывает файлом:
/// в адрес он не влезет (у GitHub предел около 8 КБ, а кириллица в адресе весит вшестеро).
pub fn issue_url(kind: Kind, what: &str, summary: &str, version: &str, gpu: &str) -> String {
    let what: String = what.trim().chars().take(600).collect();
    let first = what.lines().next().unwrap_or("").chars().take(70).collect::<String>();
    let title = if first.is_empty() { kind.title().to_string() } else { format!("{}: {first}", kind.title()) };
    let url = url::Url::parse_with_params(
        &format!("{REPO}/issues/new"),
        &[
            ("template", kind.template()),
            ("title", &title),
            ("what", &what),
            ("gpu", gpu),
            ("version", version),
            ("summary", summary),
        ],
    )
    .expect("адрес issue");
    url.into()
}

/// Сообщение для мессенджера: что случилось словами человека и сводка о компьютере.
/// Ссылку с готовым текстом в группу ни Telegram, ни MAX не передают — текст человек
/// вставляет сам, окно кладёт его в буфер обмена. Полный отчёт — файлом, как на GitHub.
/// Видеокарта, Windows и версия уже есть в сводке — отдельно не повторяем.
pub fn message(kind: Kind, what: &str, summary: &str) -> String {
    let what: String = what.trim().chars().take(1500).collect();
    format!("Ollivo — {}\n\n{what}\n\n{}\n", kind.title(), summary.trim())
}

/// Имя файла отчёта: по времени, чтобы второй отчёт не затёр первый.
pub fn file_name(now: SystemTime) -> String {
    format!("Ollivo-{}-{}.txt", t!("отчёт", "report"), stamp(now).replace([' ', ':'], "-"))
}

/// Запоминает ошибку, которую человек видел, — для отчёта. Файл, а не память: человек
/// может сначала перезапустить программу, а потом решить сообщить.
pub fn note(file: &Path, what: &str, text: &str, details: &str) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(file).is_ok_and(|m| m.len() > PROBLEMS_FILE_MAX) {
        if let Ok(old) = std::fs::read_to_string(file) {
            let half = old.len() / 2;
            let cut = old[half..].find('\n').map_or(half, |i| half + i + 1);
            let _ = std::fs::write(file, &old[cut..]);
        }
    }
    // Одна строка на ошибку: подробности — первыми строками, через « | ».
    let details: Vec<&str> = details.lines().map(str::trim).filter(|l| !l.is_empty()).take(6).collect();
    let mut line = format!("{} · {what} · {}", stamp(SystemTime::now()), text.trim());
    if !details.is_empty() {
        line += &format!(" | {}", details.join(" | "));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{}", line.replace('\n', " "));
    }
}

/// «2026-09-26 18:03» по UTC. Своя арифметика дат вместо крейта: нужна одна строка.
fn stamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Дни от 1970-01-01 → дата (алгоритм Говарда Хиннанта, civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dates() {
        assert_eq!(stamp(UNIX_EPOCH), "1970-01-01 00:00");
        // 2026-09-26 18:03 UTC
        assert_eq!(stamp(UNIX_EPOCH + Duration::from_secs(1_790_445_780)), "2026-09-26 18:03");
        assert_eq!(stamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00");
        assert_eq!(file_name(UNIX_EPOCH), "Ollivo-отчёт-1970-01-01-00-00.txt");
    }

    #[test]
    fn user_name_is_hidden() {
        let log = r"loading model 'C:\Users\Иван Петров\.lmstudio\m.gguf'
c:/users/иван петров/AppData | owner: Иван Петров";
        let s = scrub(log, Some(r"C:\Users\Иван Петров"), Some("Иван Петров"));
        assert!(!s.to_lowercase().contains("иван"), "{s}");
        assert!(s.contains(r"'%USERPROFILE%\.lmstudio\m.gguf'") && s.contains("%USERPROFILE%/AppData"), "{s}");
        assert!(s.contains("owner: %USERNAME%"), "{s}");
        // Короткое имя не трогаем — иначе испортим журнал.
        assert_eq!(scrub("main loop", None, Some("ai")), "main loop");
    }

    #[test]
    fn proxy_login_is_hidden() {
        let s = serde_json::json!({"proxy": {"enabled": true, "host": "10.0.0.1", "username": "ivan"}, "hf": {}});
        let clean = without_secrets(s);
        assert_eq!(clean["proxy"]["username"], "(скрыт)");
        assert_eq!(clean["proxy"]["host"], "10.0.0.1");
    }

    #[test]
    fn problems_are_one_line_each_and_file_stays_small() {
        let file = crate::testserver::tmp().join(format!("ollivo-report-{}", std::process::id())).join("problems.log");
        let _ = std::fs::remove_file(&file);
        note(&file, "Модель не запустилась", "Не хватило видеопамяти.", "line one\n\nline two\n");
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("Модель не запустилась · Не хватило видеопамяти. | line one | line two"), "{text}");
        for _ in 0..3000 {
            note(&file, "Чат", &"x".repeat(100), "");
        }
        assert!(std::fs::metadata(&file).unwrap().len() < PROBLEMS_FILE_MAX + 200);
    }

    #[test]
    fn report_has_no_secrets_and_long_lines_are_cut() {
        let dir = crate::testserver::tmp().join(format!("ollivo-report-b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("llama-server.log");
        std::fs::write(&log, format!("I srv  load_model: loading\nW parse: {}\n", "y".repeat(2000))).unwrap();
        let f = Facts {
            version: "0.1.0".into(),
            channel: "stable".into(),
            windows: "Windows 11 (26200)".into(),
            settings: serde_json::json!({"proxy": {"username": "ivan"}}),
            engines: vec![("ffmpeg".into(), None)],
            logs: vec![("llama-server".into(), log)],
            problems: dir.join("none.log"),
            ..Default::default()
        };
        let r = build(&f, UNIX_EPOCH);
        assert!(r.full.contains("ffmpeg — не установлен") && r.full.contains("load_model"), "{}", r.full);
        assert!(!r.full.contains("ivan") && !r.full.contains(&"y".repeat(LINE_MAX + 1)));
        assert!(r.summary.starts_with("Ollivo 0.1.0 (stable), Windows 11"));
    }

    /// Отчёт на этом ПК: `cargo test report::tests::real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real() {
        let root = Path::new(r"D:\Ollivo");
        let f = Facts {
            version: "0.0.2".into(),
            channel: "stable".into(),
            windows: sysinfo::System::long_os_version().unwrap_or_default(),
            hardware: Some(crate::hardware::detect()),
            vulkan: crate::setup::has_vulkan(),
            vc_runtime: crate::setup::has_vc_runtime(),
            engines: ["llama.cpp", "whisper.cpp", "ffmpeg"]
                .iter()
                .map(|id| (id.to_string(), crate::engines::installed(root, id).pop()))
                .collect(),
            logs: vec![("llama-server".into(), root.join(r"logs\llama-server.log"))],
            problems: crate::testserver::tmp().join("none.log"),
            data_dir: root.into(),
            ..Default::default()
        };
        let r = build(&f, SystemTime::now());
        println!("{}
-----
{}", r.summary, r.full);
        let url = issue_url(Kind::Install, "Не ставится движок, пишет «не удалось»", &r.summary, "0.0.2", "GTX 1080");
        println!("{} знаков: {url}", url.len());
    }

    #[test]
    fn issue_link_prefills_form() {
        let u = issue_url(Kind::Model, "Не запускается Qwen\nпишет про память", "Ollivo 0.1.0", "0.1.0", "GTX 1080");
        assert!(u.starts_with("https://github.com/WufCorp/Ollivo/issues/new?template=model.yml&title="), "{u}");
        let parsed = url::Url::parse(&u).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(q["title"], "Модель не запускается: Не запускается Qwen");
        assert_eq!(q["gpu"], "GTX 1080");
        assert!(u.len() < 8000);
    }

    #[test]
    fn messenger_text_has_words_and_summary() {
        let m = message(Kind::Model, "  Модель молчит  ", "Ollivo 0.4.0, GTX 1080, ОЗУ 32 ГБ");
        assert!(m.starts_with("Ollivo — "), "{m}");
        assert!(m.contains("\n\nМодель молчит\n\n") && m.ends_with("ОЗУ 32 ГБ\n"), "{m}");
        // Длинное описание обрезается: сообщение Telegram — до 4096 знаков.
        assert!(message(Kind::Other, &"а".repeat(9000), "").chars().count() < 1600);
    }
}
