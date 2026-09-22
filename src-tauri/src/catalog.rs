//! Каталог моделей: проверенная подборка и поиск по HuggingFace.
//!
//! Подборка встроена в программу (`manifest/catalog.json`) — она открывается без сети,
//! с точными размерами и SHA256, как манифест движков; свежая приходит из S3.
//! Поиск идёт мимо подборки: имена и размеры файлов берём из API репозитория HF,
//! SHA256 — оттуда же (`lfs.oid`), так что скачанное всё равно проверяется.

use crate::hardware::Hardware;
use crate::probe::{self, Verdict};
use serde::{Deserialize, Serialize};

const BUNDLED: &str = include_str!("../../manifest/catalog.json");
const SCHEMA: u32 = 1;
/// Сколько репозиториев показываем в поиске.
const SEARCH_LIMIT: u32 = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub revision: u32,
    pub models: Vec<Pick>,
}

/// Модель из подборки: репозиторий HF и отобранные варианты файлов.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pick {
    pub id: String,
    pub title: String,
    /// Кто сделал модель — не тот, кто выложил её в GGUF.
    pub vendor: String,
    pub about: String,
    pub tags: Vec<String>,
    pub license: Option<String>,
    pub repo: String,
    pub params: u64,
    /// Работающая часть у моделей «из частей» (MoE): по ней считается скорость.
    #[serde(default)]
    pub active_params: Option<u64>,
    pub files: Vec<PickFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickFile {
    pub quant: String,
    /// Имя файла в репозитории.
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

impl Catalog {
    pub fn bundled() -> Self {
        Self::parse(BUNDLED).expect("встроенный каталог")
    }

    pub fn parse(json: &str) -> Result<Self, String> {
        let c: Catalog = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if c.schema != SCHEMA {
            return Err(format!("каталог схемы {}, программа понимает {SCHEMA}", c.schema));
        }
        Ok(c)
    }
}

impl Pick {
    /// Сколько весов читается на каждый токен: у MoE — только работающая часть.
    fn active_bytes(&self, size: u64) -> u64 {
        match self.active_params {
            // В байтах модель на 35 миллиардов не помещается в u64 при умножении.
            Some(active) if self.params > 0 => (size as u128 * active as u128 / self.params as u128) as u64,
            _ => size,
        }
    }

    /// Варианты подборки со «светофором» на сегодняшнем железе.
    pub fn variants(&self, hw: &Hardware) -> Vec<Variant> {
        let mut v: Vec<Variant> = self
            .files
            .iter()
            .map(|f| Variant {
                quant: f.quant.clone(),
                name: f.name.clone(),
                size: f.size,
                sha256: Some(f.sha256.clone()),
                quality: quality(&f.quant),
                verdict: probe::rough(f.size, self.active_bytes(f.size), hw),
            })
            .collect();
        v.sort_by_key(|x| x.size);
        v
    }
}

/// Файл модели, который можно скачать: что за сжатие, сколько весит, пойдёт ли.
#[derive(Debug, Clone, Serialize)]
pub struct Variant {
    pub quant: String,
    pub name: String,
    pub size: u64,
    /// Из подборки или из `lfs.oid` HuggingFace; `None` — сверим по заголовку ответа.
    pub sha256: Option<String>,
    /// Что значит это сжатие человеческими словами.
    pub quality: &'static str,
    pub verdict: Verdict,
}

/// Сколько бит на вес: `Q4_K_M` — 4, `UD-Q2_K_XL` — 2, `F16` — 16.
fn bits(quant: &str) -> u8 {
    let q = quant.to_ascii_uppercase();
    let tail = q.rsplit('-').next().unwrap_or(&q).to_string();
    if tail.starts_with('F') || tail.starts_with("BF") {
        return 16;
    }
    tail.chars().find(char::is_ascii_digit).and_then(|c| c.to_digit(10)).unwrap_or(0) as u8
}

/// Что значит сжатие для человека, который впервые видит слово «квантизация».
pub fn quality(quant: &str) -> &'static str {
    match bits(quant) {
        0 => "",
        1..=2 => "сжата сильнее некуда: влезет куда угодно, но отвечает заметно хуже",
        3 => "сильно сжата: экономит память, иногда путается",
        4 => "обычный выбор: почти как оригинал, а места вдвое меньше",
        5 => "чуть лучше обычной и чуть больше",
        6 => "почти неотличима от оригинала",
        7..=8 => "без потерь качества, но большая",
        _ => "оригинал без сжатия — для переписки такой не нужен",
    }
}

/// Служебные файлы рядом с моделью: зрение (`mmproj`), черновая модель для ускорения
/// (`draft`, `dflash`), предсказание нескольких токенов (`mtp`). Сами по себе они
/// не запускаются — скачивать их вместо модели нельзя.
fn is_aux(name: &str) -> bool {
    let first = name.split(['-', '_', '.']).next().unwrap_or("").to_ascii_lowercase();
    matches!(first.as_str(), "mmproj" | "mtp" | "dflash" | "draft")
}

/// Кусок имени, похожий на сжатие: `Q4_K_M`, `IQ4_XS`, `UD`, `MXFP4_MOE`, `BF16`.
fn is_quant_part(p: &str) -> bool {
    let p = p.to_ascii_uppercase();
    if matches!(p.as_str(), "UD" | "BF16" | "F16" | "F32") || p.starts_with("MXFP4") {
        return true;
    }
    let rest = p.strip_prefix("IQ").or_else(|| p.strip_prefix('Q')).unwrap_or("");
    rest.starts_with(|c: char| c.is_ascii_digit())
        && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Сжатие из имени файла: подряд идущие куски в конце имени.
/// `Qwen3.5-9B-Q4_K_M.gguf` → `Q4_K_M`, `…-UD-Q4_K_XL.gguf` → `UD-Q4_K_XL`.
pub fn quant_of(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".gguf").or_else(|| name.strip_suffix(".GGUF"))?;
    let parts: Vec<&str> = stem.split('-').collect();
    let take = parts.iter().rev().take_while(|p| is_quant_part(p)).count();
    (take > 0).then(|| parts[parts.len() - take..].join("-"))
}

/// Репозиторий HuggingFace в выдаче поиска.
#[derive(Debug, Clone, Serialize)]
pub struct Repo {
    pub repo: String,
    /// Имя без автора — его показываем крупно.
    pub name: String,
    pub author: String,
    pub downloads: u64,
    pub likes: u64,
    /// Закрытая модель: качается только с токеном и после согласия на её странице.
    pub gated: bool,
    pub license: Option<String>,
}

#[derive(Deserialize)]
struct ApiModel {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    /// `false`, `"auto"` или `"manual"`.
    #[serde(default)]
    gated: serde_json::Value,
    #[serde(default)]
    tags: Vec<String>,
}

/// Поиск моделей в формате GGUF: их запускает llama.cpp, остальное в фазе 2 не нужно.
pub async fn search(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    query: &str,
) -> Result<Vec<Repo>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(vec![]);
    }
    let url = format!(
        "{base}/api/models?filter=gguf&search={}&sort=downloads&direction=-1&limit={SEARCH_LIMIT}",
        urlencode(query)
    );
    let models: Vec<ApiModel> = get_json(client, &url, token).await?;
    Ok(models
        .into_iter()
        .map(|m| {
            let (author, name) = m.id.split_once('/').unwrap_or(("", m.id.as_str()));
            Repo {
                name: name.to_string(),
                author: author.to_string(),
                downloads: m.downloads,
                likes: m.likes,
                gated: !matches!(m.gated, serde_json::Value::Bool(false)),
                license: m.tags.iter().find_map(|t| t.strip_prefix("license:")).map(str::to_string),
                repo: m.id,
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct ApiFile {
    path: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    lfs: Option<ApiLfs>,
}

#[derive(Deserialize)]
struct ApiLfs {
    /// У больших файлов HuggingFace это SHA256 — им и проверяем скачанное.
    oid: String,
}

/// Что есть в репозитории: варианты модели и сколько файлов пришлось пропустить.
#[derive(Debug, Clone, Serialize)]
pub struct Files {
    pub variants: Vec<Variant>,
    /// Модели, разрезанные на части (`-00001-of-00002`): такие пока не качаем.
    pub split: usize,
}

/// Файлы репозитория со «светофором». Сколько в модели параметров, мы не знаем,
/// поэтому скорость считается по размеру файла — для MoE она выйдет заниженной.
pub async fn files(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    repo: &str,
    hw: &Hardware,
) -> Result<Files, String> {
    let url = format!("{base}/api/models/{repo}/tree/main?recursive=true");
    let all: Vec<ApiFile> = get_json(client, &url, token).await?;
    let mut split = 0;
    let mut variants: Vec<Variant> = vec![];
    for f in all {
        let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
        if !name.to_ascii_lowercase().ends_with(".gguf") || is_aux(&name) {
            continue;
        }
        // Часть большой модели: llama.cpp такие собирает сам, но наш менеджер загрузок
        // качает по одному файлу — показывать половину модели нечестно.
        if name.contains("-of-") {
            split += 1;
            continue;
        }
        let Some(quant) = quant_of(&name) else { continue };
        // Оригинал без сжатия для переписки бесполезен, а весит втрое больше.
        if bits(&quant) > 8 {
            continue;
        }
        variants.push(Variant {
            quality: quality(&quant),
            verdict: probe::rough(f.size, f.size, hw),
            quant,
            name: f.path,
            size: f.size,
            sha256: f.lfs.map(|l| l.oid).filter(|o| o.len() == 64),
        });
    }
    // Одно и то же сжатие иногда лежит в нескольких видах (`QAD-Q4_0` рядом с `Q4_0`) —
    // человеку хватит одного, берём первый по имени.
    variants.sort_by(|a, b| a.quant.cmp(&b.quant).then(a.name.cmp(&b.name)));
    variants.dedup_by(|a, b| a.quant == b.quant);
    variants.sort_by_key(|v| v.size);
    Ok(Files { variants, split })
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<T, String> {
    let mut req = client.get(url);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            "HuggingFace не ответил — нужен прокси или зеркало?".to_string()
        } else {
            "HuggingFace не открывается — нужен прокси или зеркало?".to_string()
        }
    })?;
    match resp.status().as_u16() {
        200 => resp.json().await.map_err(|_| "HuggingFace ответил непонятным".to_string()),
        401 | 403 => Err("модель закрытая: нужен токен HuggingFace и согласие на её странице".into()),
        404 => Err("такого репозитория нет".into()),
        429 => Err("слишком много запросов к HuggingFace — подождите минуту".into()),
        s => Err(format!("HuggingFace ответил ошибкой {s}")),
    }
}

/// Только для строки поиска: percent-кодировку `reqwest` наружу не отдаёт.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_is_valid() {
        let c = Catalog::bundled();
        assert!(c.models.len() >= 5);
        for m in &c.models {
            assert!(m.repo.contains('/'), "{}", m.id);
            assert!(m.params > 0 && !m.about.is_empty(), "{}", m.id);
            assert!(!m.files.is_empty(), "{}", m.id);
            for f in &m.files {
                assert_eq!(f.sha256.len(), 64, "{} {}", m.id, f.name);
                assert!(f.size > 0 && f.name.ends_with(".gguf"), "{} {}", m.id, f.name);
                // Имя файла и заявленное сжатие должны сходиться.
                assert_eq!(quant_of(&f.name).as_deref(), Some(f.quant.as_str()), "{}", m.id);
            }
        }
        assert!(Catalog::parse(r#"{"schema":9,"revision":1,"models":[]}"#).is_err());
    }

    #[test]
    fn quants_are_read_from_names() {
        assert_eq!(quant_of("Qwen3.5-9B-Q4_K_M.gguf").as_deref(), Some("Q4_K_M"));
        assert_eq!(quant_of("Qwen3.5-35B-A3B-UD-Q4_K_XL.gguf").as_deref(), Some("UD-Q4_K_XL"));
        assert_eq!(quant_of("Qwen3.5-35B-A3B-MXFP4_MOE.gguf").as_deref(), Some("MXFP4_MOE"));
        assert_eq!(quant_of("LFM2.5-2.6B-IQ4_XS.gguf").as_deref(), Some("IQ4_XS"));
        assert_eq!(quant_of("gemma-4-E4B-it-BF16.gguf").as_deref(), Some("BF16"));
        // Размер модели за сжатие не принимаем.
        assert_eq!(quant_of("model-7B.gguf"), None);
        assert_eq!(quant_of("readme.md"), None);
    }

    #[test]
    fn bits_and_words() {
        assert_eq!(bits("Q4_K_M"), 4);
        assert_eq!(bits("UD-Q2_K_XL"), 2);
        assert_eq!(bits("IQ4_XS"), 4);
        assert_eq!(bits("BF16"), 16);
        assert!(quality("Q4_K_M").contains("обычный выбор"));
        assert!(quality("BF16").contains("без сжатия"));
    }

    #[test]
    fn aux_files_are_not_models() {
        assert!(is_aux("mmproj-F16.gguf"));
        assert!(is_aux("mtp-gemma-4-E4B-it-Q8_0.gguf"));
        assert!(is_aux("dflash-Qwen3.8-27B-Q4_0.gguf"));
        assert!(!is_aux("Qwen3.5-9B-Q4_K_M.gguf"));
    }

    /// «Светофор» подборки на сегодняшнем железе.
    /// `cargo test catalog::tests::real_picks -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_picks() {
        let hw = crate::hardware::detect();
        for m in &Catalog::bundled().models {
            println!("{} ({})", m.title, m.repo);
            for v in m.variants(&hw) {
                println!(
                    "  {:>10} {:>6.1} ГБ  {:?} {} — {}",
                    v.quant,
                    v.size as f64 / (1u64 << 30) as f64,
                    v.verdict.light,
                    v.verdict.headline,
                    v.verdict.details.join("; ")
                );
            }
        }
    }

    /// Поиск и разбор настоящего репозитория.
    /// `cargo test catalog::tests::real_search -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_search() {
        let hw = crate::hardware::detect();
        let client = reqwest::Client::new();
        let found = search(&client, crate::hf::OFFICIAL, "", "Qwen3.5-9B").await.unwrap();
        assert!(!found.is_empty());
        println!("нашлось {} репозиториев, первый — {}", found.len(), found[0].repo);
        let f = files(&client, crate::hf::OFFICIAL, "", &found[0].repo, &hw).await.unwrap();
        for v in &f.variants {
            println!("  {:>12} {:>6.1} ГБ {}", v.quant, v.size as f64 / (1u64 << 30) as f64, v.name);
        }
        assert!(f.variants.iter().any(|v| v.quant.contains("Q4")));
        assert!(f.variants.iter().all(|v| v.sha256.is_some()));
    }
}
