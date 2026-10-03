//! Картинки по описанию: какие модели умеем, параметры под модель, оценка времени до старта,
//! понятные ошибки и галерея.
//!
//! Окно знает только «форму кадра», «быстро ↔ качественно» и «сколько вариантов» — числа
//! (размер, шаги, CFG, сэмплер) решает ядро, как пресеты чата в `presets.rs`.
//! Пока умеем модели-чекпойнты Stable Diffusion (SD 1.5, SD 2.x, SDXL): в одном файле
//! и модель, и текстовый энкодер, и VAE, ComfyUI грузит их одним узлом.
//! Flux и SD 3.5 требуют отдельных файлов — следующий шаг фазы 4.

use crate::comfy;
use crate::hardware::Hardware;
use crate::probe::{Kind, ModelInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Sd15,
    Sd2,
    Sdxl,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Square,
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Fast,
    Normal,
    Best,
}

/// Больше вариантов за раз не даём: на GTX 1080 четыре картинки SDXL — уже четыре минуты.
pub const MAX_COUNT: u32 = 4;

/// Что за модель для картинок. `Err` — почему пока не умеем, словами для человека.
pub fn family(info: &ModelInfo) -> Result<Family, String> {
    if info.kind != Kind::Image {
        return Err(t!("это не модель для картинок", "this is not an image model").into());
    }
    let f = info.family.as_str();
    let fam = if f == "SD 1.5" {
        Family::Sd15
    } else if f == "SD 2.x" {
        Family::Sd2
    } else if f == "SDXL" {
        Family::Sdxl
    } else if f.starts_with("Flux") || f.starts_with("SD 3") {
        return Err(t!(
            "эту модель научимся запускать в одном из следующих обновлений",
            "we'll learn to run this model in one of the next updates"
        )
        .into());
    } else {
        return Err(t!("такую модель пока не умеем запускать", "we can't run this kind of model yet").into());
    };
    // Только модель без текстового энкодера и VAE: ComfyUI одним узлом её не загрузит.
    if !info.needs.is_empty() {
        return Err(t!(
            "в файле только часть модели — нужна полная версия (checkpoint)",
            "the file holds only part of the model — the full version (checkpoint) is needed"
        )
        .into());
    }
    if info.format != "safetensors" {
        return Err(t!("нужна версия в формате safetensors", "needs the safetensors version").into());
    }
    Ok(fam)
}

/// Решённые ядром параметры одной картинки.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub sampler: &'static str,
    pub scheduler: &'static str,
}

impl Plan {
    fn mpx(&self) -> f64 {
        (self.width * self.height) as f64 / 1e6
    }
}

/// Размеры — те, на которых модель училась: SD 1.5 на 512, SDXL на 1024 (портрет и пейзаж —
/// из списка разрешений обучения SDXL). На других размерах модели рисуют двух голов и повторы.
/// Шаги: dpmpp_2m + karras даёт чистую картинку уже к 15 шагам, после 30 почти не меняется.
pub fn plan(f: Family, shape: Shape, quality: Quality) -> Plan {
    let (square, long, short) = match f {
        Family::Sd15 => (512, 768, 512),
        Family::Sd2 => (768, 896, 640),
        Family::Sdxl => (1024, 1216, 832),
    };
    let (width, height) = match shape {
        Shape::Square => (square, square),
        Shape::Portrait => (short, long),
        Shape::Landscape => (long, short),
    };
    let steps = match quality {
        Quality::Fast => 15,
        Quality::Normal => 25,
        Quality::Best => 35,
    };
    // SDXL при CFG 7+ пережигает цвета; SD 1.5 и 2.x — привычные 7.
    let cfg = if f == Family::Sdxl { 6.0 } else { 7.0 };
    Plan { width, height, steps, cfg, sampler: "dpmpp_2m", scheduler: "karras" }
}

/// Что модель рисовать не должна. Общий список: человек его не видит и не настраивает.
pub const NEGATIVE: &str = "blurry, low quality, worst quality, jpeg artifacts, deformed, disfigured, extra limbs, watermark, text, signature";

/// Описание на кириллице: модели SD понимают только английский. Окно предупредит до старта.
pub fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
}

/// Секунды на шаг для мегапикселя вместе с декодированием — замер на GTX 1080 (фаза 0 и
/// `comfy::tests::txt2img_real`): SD 1.5 512², 20 шагов — 9,5 с; SDXL 1024², 20 шагов — 60 с.
/// SD 2.x не замеряли: UNet как у SD 1.5, но внимание шире — берём между.
fn base_rate(f: Family) -> f64 {
    match f {
        Family::Sd15 => 1.8,
        Family::Sd2 => 2.2,
        Family::Sdxl => 2.9,
    }
}

/// Во сколько раз видеокарта быстрее GTX 1080. Грубо: по пропускной способности памяти
/// (320 ГБ/с у 1080) и вдвое — за тензорные ядра у RTX (у 1080 их нет, fp16 медленный).
/// Ошибается в сторону «дольше»: RTX 4090 на деле ~10×, а выйдет ~6×. Первый же прогон
/// заменит оценку замером (`Speeds`).
fn gpu_factor(hw: &Hardware) -> f64 {
    let Some(g) = &hw.gpu else { return 1.0 };
    let bw = if g.vram_bw > 0 { g.vram_bw as f64 / 320e9 } else { 1.0 };
    let tensor = if g.cc >= (7, 0) { 2.0 } else { 1.0 };
    (bw * tensor).max(0.3)
}

/// Запуск ComfyUI, пока на этом ПК не замеряли. На GTX 1080 с NVMe — 25–39 с, если файлы
/// движка уже в кэше Windows, и ~2 мин первым запуском после перезагрузки (2026-10-03):
/// torch читается с диска. Берём середину — дальше заменит замер.
const START_SECS: f64 = 60.0;
/// Чтение модели с диска в видеокарту, пока не замеряли: ~100 МБ/с. Замер 2026-10-03 на GTX 1080:
/// SD 1.5 (2,1 ГБ) с холодного диска — ~25 с; из кэша Windows — секунды.
const LOAD_BPS: f64 = 100e6;

/// Что замерено на этом ПК. Лежит рядом с данными ComfyUI — переезжает вместе с папкой программы.
/// Испорчен или пропал — оценка по железу.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Measured {
    /// Секунды на шаг для мегапикселя, по семействам.
    #[serde(default)]
    pub rates: HashMap<Family, f64>,
    /// Сколько запускался ComfyUI в последний раз.
    #[serde(default)]
    pub start: Option<f64>,
    /// С какой скоростью читалась модель в последний раз, байт/с.
    #[serde(default)]
    pub load_bps: Option<f64>,
}

impl Measured {
    pub fn rate(&self, f: Family) -> Option<f64> {
        self.rates.get(&f).copied().filter(|r| r.is_finite() && *r > 0.0)
    }
}

/// Сколько секунд ждать всего. `cold` — ComfyUI ещё не запущен;
/// `load` — сколько байт модели читать с диска (0 — уже в памяти).
pub fn estimate(f: Family, p: &Plan, count: u32, hw: &Hardware, m: &Measured, cold: bool, load: u64) -> u64 {
    let rate = m.rate(f).unwrap_or_else(|| base_rate(f) / gpu_factor(hw));
    let draw = rate * p.steps as f64 * p.mpx() * count as f64;
    let start = if cold { m.start.unwrap_or(START_SECS) } else { 0.0 };
    let load = if load > 0 { load as f64 / m.load_bps.unwrap_or(LOAD_BPS) } else { 0.0 };
    (draw + start + load).round().max(1.0) as u64
}

/// Запуск и чтение модели зависят от того, в кэше ли Windows файлы: после перезагрузки в разы
/// дольше. Последний замер был бы то слишком бодрым, то слишком мрачным — усредняем с прошлым.
fn smooth(old: Option<f64>, new: f64) -> f64 {
    old.map_or(new, |o| (o + new) / 2.0)
}

pub struct Speeds {
    path: PathBuf,
}

impl Speeds {
    pub fn new(comfy_data: &Path) -> Self {
        Self { path: comfy_data.join("ollivo-speed.json") }
    }

    pub fn read(&self) -> Measured {
        std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn write(&self, m: &Measured) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, serde_json::to_vec(m).unwrap_or_default());
    }

    /// Картинки, где модель уже была в памяти: `secs` на `count` картинок по плану `p`.
    pub fn record(&self, f: Family, p: &Plan, count: u32, secs: f64) {
        let work = p.steps as f64 * p.mpx() * count as f64;
        if work <= 0.0 || secs <= 0.0 {
            return;
        }
        let mut m = self.read();
        m.rates.insert(f, secs / work);
        self.write(&m);
    }

    pub fn record_start(&self, secs: f64) {
        let mut m = self.read();
        m.start = Some(smooth(m.start, secs));
        self.write(&m);
    }

    /// Картинка вместе с чтением модели: `secs` всего, `bytes` — размер модели. Чтение — это
    /// всё сверх рисования по замеренной скорости; без замера скорости не считаем.
    pub fn record_load(&self, f: Family, p: &Plan, bytes: u64, secs: f64) {
        let mut m = self.read();
        let Some(rate) = m.rate(f) else { return };
        let load = secs - rate * p.steps as f64 * p.mpx();
        // Меньше полсекунды — модель была в кэше Windows: скорость диска из этого не узнать.
        if load > 0.5 {
            m.load_bps = Some(smooth(m.load_bps, bytes as f64 / load));
            self.write(&m);
        }
    }
}

/// Модель для картинок из подборки: проверенная, с хешем. Подборка обновляется
/// только с выпуском программы — как и каталог чата.
pub struct Pick {
    pub id: &'static str,
    pub repo: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub family: Family,
    pub license: &'static str,
}

/// Дообученные модели, а не базовые SD 1.5 и SDXL: те же размеры и скорость, но картинки
/// заметно лучше без длинных описаний — новичок пишет «кот в шляпе», а не
/// «masterpiece, best quality, 8k». Хеши — `lfs.oid` из API HuggingFace (2026-10-03).
pub const PICKS: &[Pick] = &[
    Pick {
        id: "dreamshaper-8",
        repo: "Lykon/DreamShaper",
        file: "DreamShaper_8_pruned.safetensors",
        size: 2_132_625_894,
        sha256: "879db523c30d3b9017143d56705015e15a2cb5628762c11d086fed9538abd7fd",
        family: Family::Sd15,
        license: "creativeml-openrail-m",
    },
    Pick {
        id: "juggernaut-xl-9",
        repo: "RunDiffusion/Juggernaut-XL-v9",
        file: "Juggernaut-XL_v9_RunDiffusionPhoto_v2.safetensors",
        size: 7_105_348_188,
        sha256: "c9e3e68f89b8e38689e1097d4be4573cf308de4e3fd044c64ca697bdb4aa8bca",
        family: Family::Sdxl,
        license: "creativeml-openrail-m",
    },
];

impl Pick {
    pub fn title(&self) -> &'static str {
        match self.id {
            "dreamshaper-8" => "DreamShaper 8",
            _ => "Juggernaut XL v9",
        }
    }

    /// Одна фраза «чем хороша», без технических слов.
    pub fn why(&self) -> &'static str {
        match self.family {
            Family::Sd15 | Family::Sd2 => t!(
                "Лёгкая и быстрая: рисунки, арт, фэнтези. Пойдёт и на скромной видеокарте.",
                "Light and fast: drawings, art, fantasy. Runs even on a modest graphics card."
            ),
            Family::Sdxl => t!(
                "Крупные, похожие на фото картинки. Нужна видеокарта посильнее, рисует дольше.",
                "Large, photo-like pictures. Needs a stronger graphics card and takes longer."
            ),
        }
    }
}

/// Пойдёт ли модель на этой видеокарте. Замеры GTX 1080 8 ГБ (фаза 0): SD 1.5 512² — пик 3,1 ГБ,
/// SDXL 1024² — 6,5 ГБ из 8. Меньше — ComfyUI сам грузит модель частями: работает, но медленнее.
pub fn fits(f: Family, hw: &Hardware) -> (crate::probe::Light, &'static str) {
    use crate::probe::Light;
    // NVML показывает чуть меньше цифры на коробке: «8 ГБ» — это 7,9.
    let vram = hw.gpu.as_ref().map_or(0, |g| g.vram_total + (256 << 20));
    let (good, slow): (u64, u64) = match f {
        Family::Sd15 | Family::Sd2 => (4 << 30, 2 << 30),
        Family::Sdxl => (8 << 30, 6 << 30),
    };
    if vram >= good {
        (Light::Green, t!("пойдёт на этой видеокарте", "will run on this graphics card"))
    } else if vram >= slow {
        (Light::Yellow, t!("пойдёт, но медленнее: видеопамяти впритык", "will run, but slower: video memory is tight"))
    } else {
        (Light::Red, t!("видеопамяти не хватит — возьмите модель полегче", "not enough video memory — take a lighter model"))
    }
}

/// Ошибка генерации человеческими словами: что случилось и что сделать.
pub struct Trouble {
    pub text: String,
    pub hint: Option<String>,
    /// Исходный текст ошибки — под «Подробности» и в отчёт.
    pub details: String,
}

pub fn explain(e: &comfy::Error) -> Trouble {
    let details = e.to_string();
    let low = details.to_lowercase();
    let (text, hint) = if low.contains("out of memory") || low.contains("outofmemory") || low.contains("allocate") {
        (
            t!("Видеокарте не хватило памяти.", "The graphics card ran out of memory."),
            Some(t!(
                "Выберите «Быстро» или квадрат, закройте игры и другие программы на видеокарте.",
                "Choose “Fast” or a square, close games and other programs using the graphics card."
            )),
        )
    } else if matches!(e, comfy::Error::Start(_)) {
        (
            t!("Движок картинок не запустился.", "The image engine did not start."),
            Some(t!(
                "Попробуйте «Починить» в разделе «Картинки». Не поможет — отправьте отчёт.",
                "Try “Repair” in the Images section. If that doesn't help, send a report."
            )),
        )
    } else if low.contains("safetensor") || low.contains("header") || low.contains("checkpoint") {
        (
            t!("Модель не открылась — файл, похоже, повреждён.", "The model didn't open — the file seems damaged."),
            Some(t!("Скачайте модель заново.", "Download the model again.")),
        )
    } else {
        (t!("Картинка не получилась.", "The picture didn't work out."), None)
    };
    Trouble { text: text.into(), hint: hint.map(Into::into), details }
}

/// Картинка из галереи для окна.
#[derive(Debug, Clone, Serialize)]
pub struct Picture {
    pub path: PathBuf,
    /// Когда нарисована, unix-секунды.
    pub mtime: u64,
}

/// Готовые картинки в папке, новые первыми. Только PNG верхнего уровня — это наши.
pub fn gallery(dir: &Path, limit: usize) -> Vec<Picture> {
    let mut out: Vec<Picture> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            m.is_file().then(|| Picture { path: e.path(), mtime })
        })
        .collect();
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| b.path.cmp(&a.path)));
    out.truncate(limit);
    out
}

/// Файл лежит прямо в папке галереи: окно не должно открывать и читать что попало.
pub fn inside(dir: &Path, path: &Path) -> bool {
    path.parent().is_some_and(|p| p == dir) && path.is_file()
}

/// Уменьшенная копия для галереи: PNG 1024² весит ~1,5 МБ, в окно их уходят десятки.
pub fn thumb(path: &Path, side: u32) -> Result<String, String> {
    use base64::Engine;
    let img = image::open(path).map_err(|e| e.to_string())?;
    let small = img.thumbnail(side, side).to_rgb8();
    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85)
        .encode_image(&small)
        .map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(buf.into_inner())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{Build, Gpu};

    fn info(family: &str) -> ModelInfo {
        let mut m: ModelInfo = serde_json::from_value(serde_json::json!({
            "format": "safetensors", "kind": "image", "engine": "comfy_ui", "family": family,
            "name": null, "license": null, "params": 0, "weights_bytes": 0, "core_params": 0,
            "core_bytes": 0, "precision": "F16", "llm": null, "contains": [], "needs": [], "notes": []
        }))
        .unwrap();
        m.family = family.into();
        m
    }

    #[test]
    fn families_we_can_draw() {
        assert_eq!(family(&info("SD 1.5")), Ok(Family::Sd15));
        assert_eq!(family(&info("SDXL")), Ok(Family::Sdxl));
        assert!(family(&info("Flux.1 dev")).is_err());
        assert!(family(&info("SDXL Refiner")).is_err());
        // Только UNet без энкодера и VAE — одним узлом не загрузить.
        let mut part = info("SDXL");
        part.needs = vec!["CLIP".into()];
        assert!(family(&part).is_err());
        let mut lora = info("SD 1.5");
        lora.kind = Kind::Lora;
        assert!(family(&lora).is_err());
    }

    #[test]
    fn sizes_are_native_for_each_model() {
        let p = plan(Family::Sd15, Shape::Portrait, Quality::Fast);
        assert_eq!((p.width, p.height, p.steps), (512, 768, 15));
        let p = plan(Family::Sdxl, Shape::Landscape, Quality::Best);
        assert_eq!((p.width, p.height, p.steps), (1216, 832, 35));
        assert!(p.cfg < 7.0);
        for f in [Family::Sd15, Family::Sd2, Family::Sdxl] {
            for s in [Shape::Square, Shape::Portrait, Shape::Landscape] {
                let p = plan(f, s, Quality::Normal);
                assert!(p.width % 64 == 0 && p.height % 64 == 0, "{f:?} {s:?}");
            }
        }
    }

    fn gtx1080() -> Hardware {
        Hardware {
            gpu: Some(Gpu {
                name: "GTX 1080".into(),
                vram_total: 8 << 30,
                vram_free: 7 << 30,
                cc: (6, 1),
                vram_bw: 320_000_000_000,
                compute_only: false,
            }),
            driver: String::new(),
            cuda_driver: 0,
            cuda_build: Build::Cuda12,
            ram_total: 32 << 30,
            ram_avail: 20 << 30,
            disks: vec![],
            profile_risky: false,
        }
    }

    /// Оценка без замера сходится с замерами на GTX 1080.
    #[test]
    fn estimate_matches_gtx1080_measurements() {
        let hw = gtx1080();
        let sd = Plan { steps: 20, ..plan(Family::Sd15, Shape::Square, Quality::Normal) };
        let none = Measured::default();
        let secs = estimate(Family::Sd15, &sd, 1, &hw, &none, false, 0);
        assert!((8..=12).contains(&secs), "{secs}");
        let xl = Plan { steps: 20, ..plan(Family::Sdxl, Shape::Square, Quality::Normal) };
        let secs = estimate(Family::Sdxl, &xl, 1, &hw, &none, false, 0);
        assert!((55..=65).contains(&secs), "{secs}");
        // Первый запуск: плюс запуск движка и чтение 2 ГБ модели.
        let cold = estimate(Family::Sd15, &sd, 1, &hw, &none, true, 2_000_000_000);
        assert!(cold > 60, "{cold}");
    }

    #[test]
    fn measured_speed_replaces_guess() {
        let dir = crate::testserver::tmp().join("ollivo-images-speed");
        let _ = std::fs::remove_dir_all(&dir);
        let s = Speeds::new(&dir);
        assert_eq!(s.read().rate(Family::Sd15), None);
        let p = plan(Family::Sd15, Shape::Square, Quality::Normal);
        // Чтение модели без замера скорости рисования не посчитать.
        s.record_load(Family::Sd15, &p, 2_000_000_000, 30.0);
        assert_eq!(s.read().load_bps, None);
        s.record(Family::Sd15, &p, 2, 10.0);
        let m = s.read();
        assert_eq!(estimate(Family::Sd15, &p, 2, &gtx1080(), &m, false, 0), 10);
        assert_eq!(m.rate(Family::Sdxl), None);
        // Первая картинка шла 25 с вместо 5 — 20 с на 2 ГБ: 100 МБ/с.
        s.record_load(Family::Sd15, &p, 2_000_000_000, 25.0);
        s.record_start(40.0);
        s.record_start(20.0);
        let m = s.read();
        assert_eq!(m.load_bps.map(|b| b.round()), Some(1e8));
        // Запуск усредняется с прошлым: после перезагрузки он в разы дольше.
        assert_eq!(m.start, Some(30.0));
        assert_eq!(estimate(Family::Sd15, &p, 1, &gtx1080(), &m, true, 2_000_000_000), 55);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn picks_are_checkpoints_with_full_hashes() {
        for p in PICKS {
            assert_eq!(p.sha256.len(), 64, "{}", p.id);
            assert!(p.file.ends_with(".safetensors"), "{}", p.id);
            assert!(p.repo.contains('/'), "{}", p.id);
        }
        use crate::probe::Light;
        assert_eq!(fits(Family::Sdxl, &gtx1080()).0, Light::Green);
        let mut small = gtx1080();
        small.gpu.as_mut().unwrap().vram_total = 4 << 30;
        assert_eq!(fits(Family::Sdxl, &small).0, Light::Red);
        assert_eq!(fits(Family::Sd15, &small).0, Light::Green);
    }

    #[test]
    fn cyrillic_prompt_is_noticed() {
        assert!(has_cyrillic("кот в шляпе"));
        assert!(has_cyrillic("a cat, ёлка"));
        assert!(!has_cyrillic("a cat in a hat, 4k"));
    }

    #[test]
    fn out_of_memory_is_explained() {
        let e = comfy::Error::Failed("KSampler (OutOfMemoryError): CUDA out of memory. Tried to allocate 2.00 GiB".into());
        let t = explain(&e);
        assert!(t.hint.is_some());
        assert!(t.details.contains("Tried to allocate"));
    }

    #[test]
    fn gallery_lists_only_own_pngs_newest_first() {
        let dir = crate::testserver::tmp().join("ollivo-gallery Иван");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for n in ["a.png", "b.PNG", "notes.txt", "sub/c.png"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let g = gallery(&dir, 10);
        let names: Vec<_> = g.iter().map(|p| p.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(inside(&dir, &dir.join("a.png")));
        assert!(!inside(&dir, &dir.join("sub").join("c.png")));
        assert!(!inside(&dir, &dir.join("..").join("a.png")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
