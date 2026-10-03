//! Каталог моделей: проверенная подборка и поиск по HuggingFace.
//!
//! Подборка встроена в программу (`manifest/catalog.json`) — она открывается без сети,
//! с точными размерами и SHA256, как манифест движков; свежая приходит из S3.
//! Поиск идёт мимо подборки: имена и размеры файлов берём из API репозитория HF,
//! SHA256 — оттуда же (`lfs.oid`), так что скачанное всё равно проверяется.

use crate::hardware::Hardware;
use crate::probe::{self, Light, Verdict};
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
    /// То же по-английски. В каталоге из S3 старой схемы их может не быть — тогда русские.
    #[serde(default)]
    pub about_en: Option<String>,
    #[serde(default)]
    pub tags_en: Vec<String>,
    /// Видит картинки: в репозитории есть `mmproj`. Полем, а не меткой — по нему
    /// программа выбирает, что предложить, когда модель не видит приложенное фото.
    #[serde(default)]
    pub vision: bool,
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

impl Pick {
    /// Описание на языке окна.
    pub fn about(&self) -> &str {
        match &self.about_en {
            Some(en) if crate::i18n::en() => en,
            _ => &self.about,
        }
    }

    /// Метки на языке окна.
    pub fn tags(&self) -> &[String] {
        if crate::i18n::en() && self.tags_en.len() == self.tags.len() { &self.tags_en } else { &self.tags }
    }
}

impl Catalog {
    pub fn bundled() -> Self {
        Self::parse(BUNDLED).expect("встроенный каталог")
    }

    pub fn parse(json: &str) -> Result<Self, String> {
        let c: Catalog = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if c.schema != SCHEMA {
            return Err(tf!("каталог схемы {}, программа понимает {SCHEMA}", "catalog schema {}, the program understands {SCHEMA}", c.schema));
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

/// Модель подборки со зрением для этого ПК, когда текущая картинку не видит.
/// Берём самую крупную из тех, что целиком помещаются в видеокарту: она и умнее,
/// и отвечает быстро. Если в видеокарту не лезет ни одна — самую лёгкую из тех, что
/// пойдут хоть как-то. Красный «светофор» не предлагаем: скачать 3 ГБ ради «не хватит
/// памяти» — хуже, чем честно сказать «в каталоге».
/// `reserve` — видеопамять под само зрение: дополнение с буфером на картинку.
pub fn seeing_pick<'a>(picks: &'a [Pick], hw: &Hardware, reserve: u64) -> Option<(&'a Pick, Variant)> {
    let mut hw = hw.clone();
    if let Some(g) = hw.gpu.as_mut() {
        g.vram_free = g.vram_free.saturating_sub(reserve);
    }
    let light = |v: &Variant| v.verdict.light;
    let chosen = picks.iter().filter(|p| p.vision).filter_map(|p| {
        let vs = p.variants(&hw);
        // Среди вариантов, которые лезут в видеокарту, — самый близкий к «обычному выбору»
        // (4 бита); меньше четырёх — только если больше ничего не лезет.
        let green = vs
            .iter()
            .filter(|v| light(v) == Light::Green)
            .min_by_key(|v| (bits(&v.quant) < 4, bits(&v.quant).abs_diff(4)))
            .cloned();
        green.or_else(|| vs.into_iter().find(|v| light(v) == Light::Yellow)).map(|v| (p, v))
    });
    let (green, yellow): (Vec<_>, Vec<_>) = chosen.partition(|(_, v)| light(v) == Light::Green);
    // Сжатая до двух-трёх бит большая модель отвечает хуже обычной средней — она
    // только если иначе никак. Одинаковые по размеру (4B у Qwen и Gemma) — берём файл
    // полегче: быстрее качается и отвечает.
    let best_green =
        green.into_iter().max_by_key(|(p, v)| (bits(&v.quant) >= 4, p.params, std::cmp::Reverse(v.size)));
    best_green.or_else(|| yellow.into_iter().min_by_key(|(_, v)| v.size))
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
        1..=2 => t!(
            "сжата сильнее некуда: влезет куда угодно, но отвечает заметно хуже",
            "squeezed to the limit: fits anywhere, but answers noticeably worse"
        ),
        3 => t!("сильно сжата: экономит память, иногда путается", "strongly squeezed: saves memory, sometimes gets confused"),
        4 => t!(
            "обычный выбор: почти как оригинал, а места вдвое меньше",
            "the usual choice: almost like the original, at half the size"
        ),
        5 => t!("чуть лучше обычной и чуть больше", "a bit better than usual and a bit bigger"),
        6 => t!("почти неотличима от оригинала", "almost indistinguishable from the original"),
        7..=8 => t!("без потерь качества, но большая", "no quality loss, but big"),
        _ => t!("оригинал без сжатия — для переписки такой не нужен", "the uncompressed original — not needed for chatting"),
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

/// Закрыта, только если HF прямо так сказал (`true`, `"auto"`, `"manual"`).
/// Нет поля — не значит «закрыта»: раньше из-за этого закрытыми выглядели все.
fn is_gated(v: &serde_json::Value) -> bool {
    matches!(v, serde_json::Value::Bool(true) | serde_json::Value::String(_))
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
        // `gated` в выдаче поиска по умолчанию нет — просим явно, вместе с остальным,
        // что показываем: `expand` заменяет набор полей целиком.
        "{base}/api/models?filter=gguf&search={}&sort=downloads&direction=-1&limit={SEARCH_LIMIT}&expand[]=gated&expand[]=downloads&expand[]=likes&expand[]=tags",
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
                gated: is_gated(&m.gated),
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

/// Файл зрения (`mmproj`) в репозитории модели: имя, размер и SHA256.
#[derive(Debug, Clone, Serialize)]
pub struct Projector {
    pub name: String,
    pub size: u64,
    pub sha256: Option<String>,
}

/// Есть ли у модели из этого репозитория зрение, и какой файл качать.
pub async fn projector(client: &reqwest::Client, base: &str, token: &str, repo: &str) -> Result<Option<Projector>, String> {
    let url = format!("{base}/api/models/{repo}/tree/main?recursive=true");
    let all: Vec<ApiFile> = get_json(client, &url, token).await?;
    let Some(name) = crate::vision::pick_from_repo(all.iter().map(|f| f.path.as_str())) else {
        return Ok(None);
    };
    let f = all.iter().find(|f| f.path == name).unwrap();
    Ok(Some(Projector {
        name: f.path.clone(),
        size: f.size,
        sha256: f.lfs.as_ref().map(|l| l.oid.clone()).filter(|o| o.len() == 64),
    }))
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
            t!("HuggingFace не ответил — нужен прокси или зеркало?", "HuggingFace did not respond — do you need a proxy or a mirror?").to_string()
        } else {
            t!("HuggingFace не открывается — нужен прокси или зеркало?", "HuggingFace won't open — do you need a proxy or a mirror?").to_string()
        }
    })?;
    match resp.status().as_u16() {
        200 => resp.json().await.map_err(|_| t!("HuggingFace ответил непонятным", "HuggingFace returned something unreadable").to_string()),
        401 | 403 => Err(t!(
            "модель закрытая: нужен токен HuggingFace и согласие на её странице",
            "the model is gated: you need a HuggingFace token and to accept the terms on its page"
        )
        .into()),
        404 => Err(t!("такого репозитория нет", "no such repository").into()),
        429 => Err(t!("слишком много запросов к HuggingFace — подождите минуту", "too many requests to HuggingFace — wait a minute").into()),
        s => Err(tf!("HuggingFace ответил ошибкой {s}", "HuggingFace returned error {s}")),
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
    fn gated_only_when_said_so() {
        use serde_json::json;
        assert!(is_gated(&json!(true)) && is_gated(&json!("manual")) && is_gated(&json!("auto")));
        assert!(!is_gated(&json!(false)) && !is_gated(&serde_json::Value::Null));
    }

    #[test]
    fn bundled_is_valid() {
        let c = Catalog::bundled();
        assert!(c.models.len() >= 5);
        for m in &c.models {
            assert!(m.repo.contains('/'), "{}", m.id);
            // Английское — у каждой модели и метка в метку: иначе окно покажет смесь языков.
            assert!(m.about_en.as_ref().is_some_and(|a| !a.is_empty()), "{}", m.id);
            assert_eq!(m.tags_en.len(), m.tags.len(), "{}", m.id);
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

    fn hw(vram_gib: Option<u64>, ram_gib: u64) -> Hardware {
        const GIB: u64 = 1 << 30;
        Hardware {
            gpu: vram_gib.map(|v| crate::hardware::Gpu {
                name: "test".into(),
                vram_total: v * GIB,
                vram_free: v * GIB - GIB / 2,
                cc: (6, 1),
                vram_bw: 320_000_000_000,
                compute_only: false,
            }),
            driver: String::new(),
            cuda_driver: 0,
            cuda_build: crate::hardware::Build::Vulkan,
            ram_total: ram_gib * 2 * GIB,
            ram_avail: ram_gib * GIB,
            disks: vec![],
            profile_risky: false,
        }
    }

    /// Что предложить вместо модели без зрения — по встроенной подборке.
    #[test]
    fn seeing_pick_fits_hardware() {
        let c = Catalog::bundled();
        let pick = |hw: &Hardware| seeing_pick(&c.models, hw, 1 << 30).map(|(p, v)| (p.id.clone(), v.quant));
        // GTX 1080: 9B со зрением уже не лезет, из двух 4B — та, что легче.
        assert_eq!(pick(&hw(Some(8), 16)), Some(("qwen3.5-4b".into(), "Q4_K_M".into())));
        // 24 ГБ: 27B в обычном сжатии, а не 35B, сжатая до двух бит.
        assert_eq!(pick(&hw(Some(24), 32)), Some(("qwen3.5-27b".into(), "Q4_K_M".into())));
        // Без видеокарты — самая лёгкая, честно жёлтая.
        let (p, v) = seeing_pick(&c.models, &hw(None, 8), 1 << 30).unwrap();
        assert_eq!((p.id.as_str(), v.verdict.light), ("qwen3.5-2b", Light::Yellow));
        // Памяти нет ни на что — не предлагаем ничего.
        assert_eq!(pick(&hw(None, 1)), None);
        // Модели без зрения не предлагаются никогда.
        assert!(c.models.iter().any(|m| !m.vision));
    }

    /// Шкала «влезет ли» в окне не должна спорить со «светофором»: зелёная помещается
    /// в свободное, не помещающаяся — не зелёная, без видеокарты шкалы нет.
    #[test]
    fn gauge_agrees_with_light() {
        const GIB: u64 = 1 << 30;
        let gtx1080 = hw(Some(8), 16);
        let small = probe::rough(GIB, GIB, &gtx1080);
        assert_eq!(small.light, Light::Green);
        assert!(small.need.unwrap() <= small.room.unwrap());
        let big = probe::rough(20 * GIB, 20 * GIB, &gtx1080);
        assert_ne!(big.light, Light::Green);
        assert!(big.need.unwrap() > big.room.unwrap());
        let no_gpu = probe::rough(GIB, GIB, &hw(None, 16));
        assert_eq!((no_gpu.need, no_gpu.room), (None, None));
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
