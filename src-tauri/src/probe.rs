//! Что за модель лежит в файле и пойдёт ли она на этом ПК.
//!
//! Заголовок файла (GGUF / safetensors / ggml) → тип, движок, размеры → «светофор».
//! Перенесено из прототипа фазы 0 `spikes/model-probe`, веса не читаются.

use crate::hardware::Hardware;
use crate::{gguf, safetensors as st};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Текстовая модель (чат).
    Llm,
    /// Проектор зрения/звука для LLM (mmproj).
    Projector,
    Image,
    Video,
    SpeechToText,
    Vae,
    Lora,
    ControlNet,
    TextEncoder,
    Upscaler,
    Unknown,
}

impl Kind {
    pub fn ru(self) -> &'static str {
        match self {
            Kind::Llm => "текстовая модель (чат)",
            Kind::Projector => "дополнение «зрение» для текстовой модели (mmproj)",
            Kind::Image => "генерация картинок",
            Kind::Video => "генерация видео",
            Kind::SpeechToText => "распознавание речи",
            Kind::Vae => "VAE (часть модели картинок)",
            Kind::Lora => "LoRA (дополнение к модели)",
            Kind::ControlNet => "ControlNet (дополнение к модели картинок)",
            Kind::TextEncoder => "текстовый энкодер (часть модели картинок/видео)",
            Kind::Upscaler => "увеличение разрешения",
            Kind::Unknown => "неизвестно",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    LlamaCpp,
    ComfyUi,
    WhisperCpp,
    /// Формат не запускается нашими движками без конвертации.
    NeedsConversion,
    None,
}

/// Размеры текстовой модели — нужны для расчёта памяти.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDims {
    pub layers: u64,
    pub ctx_train: u64,
    pub embd: u64,
    pub heads: u64,
    pub heads_kv: u64,
    pub head_dim: u64,
    pub vocab: u64,
    /// Байт весов на один слой (в среднем).
    pub layer_bytes: u64,
    /// Веса вне слоёв (эмбеддинги, выходной слой).
    pub other_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub format: String,
    pub kind: Kind,
    pub engine: Engine,
    /// Семейство: «SDXL», «Flux.1 schnell», «llama» и т.п.
    pub family: String,
    pub name: Option<String>,
    pub license: Option<String>,
    pub params: u64,
    pub weights_bytes: u64,
    /// Сама диффузионная модель без текстовых энкодеров и VAE (для расчёта памяти).
    pub core_params: u64,
    pub core_bytes: u64,
    /// Основной тип весов: Q4_K, F16, FP8…
    pub precision: String,
    pub llm: Option<LlmDims>,
    /// Что лежит внутри файла (для моделей картинок).
    pub contains: Vec<String>,
    /// Что ещё нужно скачать, чтобы модель заработала.
    pub needs: Vec<String>,
    pub notes: Vec<String>,
}

impl ModelInfo {
    fn new(format: &'static str) -> Self {
        ModelInfo {
            format: format.into(),
            kind: Kind::Unknown,
            engine: Engine::None,
            family: String::new(),
            name: None,
            license: None,
            params: 0,
            weights_bytes: 0,
            core_params: 0,
            core_bytes: 0,
            precision: String::new(),
            llm: None,
            contains: vec![],
            needs: vec![],
            notes: vec![],
        }
    }
}

pub fn probe(path: &Path) -> Result<ModelInfo> {
    let mut magic = [0u8; 8];
    let n = File::open(path)?.read(&mut magic)?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if n >= 4 && &magic[..4] == b"GGUF" {
        return Ok(from_gguf(&gguf::read(path)?));
    }
    if n >= 4 && &magic[..4] == b"lmgg" {
        return from_whisper_ggml(path);
    }
    if n >= 2 && &magic[..2] == b"PK" || matches!(ext.as_str(), "ckpt" | "pt" | "pth") {
        let mut m = ModelInfo::new("pickle (PyTorch)");
        m.notes.push(
            "Формат pickle может содержать вредоносный код. Открывать не будем — \
             поищите эту модель в формате safetensors или GGUF."
                .into(),
        );
        return Ok(m);
    }
    if ext == "safetensors" || n == 8 {
        return Ok(from_safetensors(&st::read(path)?));
    }
    bail!("формат файла не распознан")
}

// ---------- GGUF ----------

/// Архитектуры диффузионных моделей в GGUF (сборки city96 для ComfyUI-GGUF).
fn gguf_diffusion(arch: &str) -> Option<(Kind, &'static str)> {
    Some(match arch {
        "flux" => (Kind::Image, "Flux.1"),
        "sd1" => (Kind::Image, "SD 1.5"),
        "sdxl" => (Kind::Image, "SDXL"),
        "sd3" => (Kind::Image, "SD 3.x"),
        "aura" => (Kind::Image, "AuraFlow"),
        "hidream" => (Kind::Image, "HiDream"),
        "qwen_image" => (Kind::Image, "Qwen-Image"),
        "lumina2" => (Kind::Image, "Lumina 2"),
        "wan" => (Kind::Video, "Wan"),
        "ltxv" => (Kind::Video, "LTX-Video"),
        "hyvid" => (Kind::Video, "HunyuanVideo"),
        "cosmos" => (Kind::Video, "Cosmos"),
        _ => return None,
    })
}

fn from_gguf(g: &gguf::Gguf) -> ModelInfo {
    let mut m = ModelInfo::new("GGUF");
    let arch = g.str("general.architecture").unwrap_or("?").to_string();
    m.name = g.str("general.name").map(String::from);
    m.license = g.str("general.license").map(String::from);
    m.params = g.tensors.iter().map(|t| t.elements).sum();
    m.weights_bytes = g.tensors.iter().map(|t| t.bytes).sum();
    m.precision = dominant_ggml_type(g);
    if g.tensors.iter().any(|t| gguf::ggml_block(t.ggml_type).is_none()) {
        m.notes.push("есть тензоры незнакомого типа — размер посчитан не полностью".into());
    }

    if arch == "clip" || g.str("general.type") == Some("mmproj") {
        m.kind = Kind::Projector;
        m.engine = Engine::LlamaCpp;
        let mut what = vec![];
        if g.get("clip.has_vision_encoder").is_some() {
            what.push("зрение");
        }
        if g.get("clip.has_audio_encoder").is_some() {
            what.push("звук");
        }
        m.family = format!(
            "{} ({})",
            g.str("clip.projector_type")
                .or(g.str("clip.vision.projector_type"))
                .unwrap_or("clip"),
            what.join(", ")
        );
        m.notes.push("подключается к своей текстовой модели, отдельно не запускается".into());
        return m;
    }

    if let Some((kind, fam)) = gguf_diffusion(&arch) {
        m.kind = kind;
        m.engine = Engine::ComfyUi;
        m.family = fam.to_string();
        m.contains.push(if kind == Kind::Video { "видеомодель" } else { "UNet/DiT" }.into());
        m.needs = diffusion_needs(fam, false, false);
        m.core_params = m.params;
        m.core_bytes = m.weights_bytes;
        m.notes.push("нужен узел ComfyUI-GGUF".into());
        return m;
    }

    if g.get("tokenizer.ggml.model").is_some() || g.arch_int("block_count").is_some() {
        m.kind = Kind::Llm;
        m.engine = Engine::LlamaCpp;
        m.family = arch.clone();
        let layers = g.arch_int("block_count").unwrap_or(0);
        let embd = g.arch_int("embedding_length").unwrap_or(0);
        let heads = g.arch_int("attention.head_count").unwrap_or(0);
        let heads_kv = g.arch_int("attention.head_count_kv").unwrap_or(heads);
        let head_dim = g
            .arch_int("attention.key_length")
            .unwrap_or(if heads > 0 { embd / heads } else { 0 });
        let layer_total: u64 =
            g.tensors.iter().filter(|t| t.name.starts_with("blk.")).map(|t| t.bytes).sum();
        let vocab = match g.get("tokenizer.ggml.tokens") {
            Some(gguf::Value::Arr { len, .. }) => *len,
            _ => g.arch_int("vocab_size").unwrap_or(0),
        };
        m.llm = Some(LlmDims {
            layers,
            ctx_train: g.arch_int("context_length").unwrap_or(4096),
            embd,
            heads,
            heads_kv,
            head_dim,
            vocab,
            layer_bytes: if layers > 0 { layer_total / layers } else { 0 },
            other_bytes: m.weights_bytes - layer_total,
        });
        if let Some(label) = g.str("general.size_label") {
            m.family = format!("{arch} {label}");
        }
        if g.int("split.count").unwrap_or(1) > 1 {
            m.notes.push("модель разбита на несколько файлов — нужны все части".into());
        }
        return m;
    }

    m.family = arch;
    m
}

fn dominant_ggml_type(g: &gguf::Gguf) -> String {
    let mut by_type: Vec<(u32, u64)> = vec![];
    for t in &g.tensors {
        match by_type.iter_mut().find(|(ty, _)| *ty == t.ggml_type) {
            Some(e) => e.1 += t.bytes,
            None => by_type.push((t.ggml_type, t.bytes)),
        }
    }
    by_type.sort_by_key(|e| std::cmp::Reverse(e.1));
    by_type.first().map(|(ty, _)| gguf::ggml_type_name(*ty).to_string()).unwrap_or_default()
}

// ---------- Whisper (старый формат ggml у whisper.cpp) ----------

fn from_whisper_ggml(path: &Path) -> Result<ModelInfo> {
    let mut f = File::open(path)?;
    let mut h = [0u8; 4 + 11 * 4];
    f.read_exact(&mut h)?;
    let field = |i: usize| i32::from_le_bytes(h[4 + i * 4..8 + i * 4].try_into().unwrap());
    let (vocab, audio_layers, ftype) = (field(0), field(4), field(10));
    let mut m = ModelInfo::new("ggml (whisper.cpp)");
    m.kind = Kind::SpeechToText;
    m.engine = Engine::WhisperCpp;
    let (size, bytes) = match audio_layers {
        4 => ("tiny", 75u64 << 20),
        6 => ("base", 142 << 20),
        12 => ("small", 466 << 20),
        24 => ("medium", 1500 << 20),
        32 => ("large", 3000 << 20),
        _ => ("?", 0),
    };
    let en = if vocab == 51864 { ".en (только английский)" } else { "" };
    m.family = format!("Whisper {size}{en}");
    m.precision = match ftype % 1000 {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        6 => "Q5_0",
        7 => "Q5_1",
        8 => "Q8_0",
        _ => "?",
    }
    .into();
    // Настоящий размер — по файлу; для обрезанных сэмплов — типичный для fp16.
    let real = std::fs::metadata(path)?.len();
    m.weights_bytes = if real > 16 << 20 { real } else { bytes };
    Ok(m)
}

// ---------- safetensors ----------

fn from_safetensors(s: &st::Safetensors) -> ModelInfo {
    let mut m = ModelInfo::new("safetensors");
    m.params = s.tensors.iter().map(|t| t.shape.iter().product::<u64>()).sum();
    m.weights_bytes = s.tensors.iter().map(|t| t.bytes).sum();
    m.precision = dominant_dtype(s);
    m.license = s
        .meta("modelspec.license")
        .or(s.meta("license"))
        .map(String::from);
    m.name = s.meta("modelspec.title").map(String::from);

    // Префикс, под которым лежит сама диффузионная модель в «полном» чекпойнте.
    let dm = if s.has_prefix("model.diffusion_model.") { "model.diffusion_model." } else { "" };
    let has_te = s.has_prefix("cond_stage_model.")
        || s.has_prefix("conditioner.")
        || s.has_prefix("text_encoders.");
    let has_vae = s.has_prefix("first_stage_model.") || s.has_prefix("vae.");

    // LoRA — раньше всего: у неё имена тензоров повторяют базовую модель.
    if s.has("lora_up") || s.has("lora_down") || s.has(".lora_A") || s.has(".lora_B") || s.has(".lora.") {
        m.kind = Kind::Lora;
        m.engine = Engine::ComfyUi;
        let base = s.meta("ss_base_model_version").unwrap_or("").to_lowercase()
            + &s.meta("modelspec.architecture").unwrap_or("").to_lowercase();
        m.family = if base.contains("flux") || s.has("double_blocks") {
            "LoRA для Flux".into()
        } else if base.contains("xl") || s.has("lora_te2_") || s.has("lora_unet_input_blocks_4_1_transformer_blocks_1") {
            "LoRA для SDXL".into()
        } else if s.has("base_model.model.model.layers") || s.has("q_proj") && !s.has("lora_unet") {
            m.engine = Engine::NeedsConversion;
            "LoRA для текстовой модели".into()
        } else if s.has("lora_unet_") {
            "LoRA для SD 1.5".into()
        } else {
            "LoRA (база не определена)".into()
        };
        return m;
    }

    if s.has_prefix("control_model.") || s.has("input_hint_block") || s.has("controlnet_") {
        m.kind = Kind::ControlNet;
        m.engine = Engine::ComfyUi;
        m.family = "ControlNet".into();
        return m;
    }

    if s.has("double_blocks.") && s.has("single_blocks.") {
        let hunyuan = s.has("txt_in.individual_token_refiner");
        m.kind = if hunyuan { Kind::Video } else { Kind::Image };
        m.engine = Engine::ComfyUi;
        m.family = if hunyuan {
            "HunyuanVideo".into()
        } else if s.has("guidance_in.") {
            "Flux.1 dev".into()
        } else {
            "Flux.1 schnell".into()
        };
        finish_diffusion(&mut m, s, dm, has_te, has_vae);
        return m;
    }

    if s.has("joint_blocks.") {
        m.kind = Kind::Image;
        m.engine = Engine::ComfyUi;
        m.family = "SD 3.x".into();
        finish_diffusion(&mut m, s, dm, has_te, has_vae);
        return m;
    }

    if s.has("patch_embedding") && s.has("blocks.0.self_attn.q") {
        m.kind = Kind::Video;
        m.engine = Engine::ComfyUi;
        let dim = s.find("patch_embedding.weight").map(|t| t.shape[0]).unwrap_or(0);
        let size = match dim {
            1536 => "1.3B",
            3072 => "5B",
            5120 => "14B",
            _ => "?",
        };
        let mode = if s.has("img_emb.") { "картинка→видео" } else { "текст→видео" };
        m.family = format!("Wan {size} ({mode})");
        finish_diffusion(&mut m, s, dm, has_te, has_vae);
        return m;
    }

    if s.has("patchify_proj") && s.has("scale_shift_table") {
        m.kind = Kind::Video;
        m.engine = Engine::ComfyUi;
        m.family = "LTX-Video".into();
        finish_diffusion(&mut m, s, dm, has_te, s.has_prefix("vae."));
        return m;
    }

    // UNet семейства Stable Diffusion: по размерности кросс-внимания.
    if s.has("input_blocks.") || s.has("down_blocks.0.attentions") {
        let ctx = s
            .tensors
            .iter()
            .filter(|t| t.name.contains("attn2.to_k.weight") && t.shape.len() == 2)
            .map(|t| t.shape[1])
            .max()
            .unwrap_or(0);
        m.kind = Kind::Image;
        m.engine = Engine::ComfyUi;
        m.family = match ctx {
            768 => "SD 1.5",
            1024 => "SD 2.x",
            2048 => "SDXL",
            1280 => "SDXL Refiner",
            _ => "Stable Diffusion (версия не определена)",
        }
        .into();
        if s.has("down_blocks.0.attentions") {
            m.notes.push("формат diffusers (папка из нескольких файлов) — это только UNet".into());
        }
        finish_diffusion(&mut m, s, dm, has_te, has_vae);
        return m;
    }

    if s.has_prefix("decoder.") && s.has_prefix("encoder.") || s.has_prefix("first_stage_model.") {
        m.kind = Kind::Vae;
        m.engine = Engine::ComfyUi;
        let conv_in = s.find("decoder.conv_in.weight").or(s.find("decoder.conv1.weight"));
        m.family = match conv_in.map(|t| (t.shape.get(1).copied(), t.shape.len())) {
            Some((_, 5)) => "VAE видео (Wan)".into(),
            Some((Some(4), _)) => "VAE для SD 1.5 / SDXL".into(),
            Some((Some(16), _)) => "VAE для Flux / SD 3".into(),
            _ => "VAE".into(),
        };
        return m;
    }

    if s.has("text_model.encoder.layers") {
        m.kind = Kind::TextEncoder;
        m.engine = Engine::ComfyUi;
        let hidden = s
            .find("text_model.final_layer_norm.weight")
            .map(|t| t.shape[0])
            .unwrap_or(0);
        m.family = match hidden {
            768 => "CLIP-L",
            1280 => "CLIP-G",
            _ => "CLIP",
        }
        .into();
        return m;
    }
    if s.has("encoder.block.0.layer.0.SelfAttention") {
        m.kind = Kind::TextEncoder;
        m.engine = Engine::ComfyUi;
        m.family = "T5".into();
        return m;
    }

    if s.has("conv_first.weight") || s.has("RRDB_trunk") || s.has("body.0.rdb1") {
        m.kind = Kind::Upscaler;
        m.engine = Engine::ComfyUi;
        m.family = "ESRGAN".into();
        return m;
    }

    if s.has("model.encoder.conv1") && s.has("model.decoder.") {
        m.kind = Kind::SpeechToText;
        m.engine = Engine::NeedsConversion;
        m.family = "Whisper (формат HuggingFace)".into();
        m.notes.push("для whisper.cpp нужна версия в формате ggml — найдём её в каталоге".into());
        return m;
    }

    if s.has("model.embed_tokens.weight") || s.has("model.layers.0.") || s.has("lm_head.weight") {
        m.kind = Kind::Llm;
        m.engine = Engine::NeedsConversion;
        m.family = "текстовая модель (формат HuggingFace)".into();
        m.notes.push(
            "llama.cpp работает с GGUF. Найдём готовую GGUF-версию или сконвертируем (нужен Python)".into(),
        );
        if s.has("model.layers.0.") && !s.has("lm_head.weight") && !s.has("model.norm.weight") {
            m.notes.push("похоже на одну часть из нескольких (model-0000X-of-0000N)".into());
        }
        return m;
    }

    if let Some(a) = s.meta("modelspec.architecture") {
        m.family = format!("по метаданным: {a}");
    }
    m
}

fn finish_diffusion(m: &mut ModelInfo, s: &st::Safetensors, dm: &str, has_te: bool, has_vae: bool) {
    m.contains.push(if m.kind == Kind::Video { "видеомодель" } else { "UNet/DiT" }.into());
    if has_te {
        m.contains.push("текстовый энкодер".into());
    }
    if has_vae {
        m.contains.push("VAE".into());
    }
    // Для расчёта памяти важен размер самой диффузионной части.
    let core = |n: &str| {
        if dm.is_empty() {
            !["cond_stage_model.", "conditioner.", "first_stage_model.", "text_encoders.", "vae."]
                .iter()
                .any(|p| n.starts_with(p))
        } else {
            n.starts_with(dm)
        }
    };
    m.core_bytes = s.bytes_where(core);
    m.core_params = s
        .tensors
        .iter()
        .filter(|t| core(&t.name))
        .map(|t| t.shape.iter().product::<u64>())
        .sum();
    if m.core_bytes < m.weights_bytes {
        m.notes.push(format!("из них сама модель: {}", fmt_bytes(m.core_bytes)));
    }
    let fam = m.family.clone();
    m.needs = diffusion_needs(&fam, has_te, has_vae);
}

/// Какие компоненты ещё нужны, если в файле только сама модель.
fn diffusion_needs(family: &str, has_te: bool, has_vae: bool) -> Vec<String> {
    let mut v = vec![];
    let (te, vae): (&[&str], &str) = if family.starts_with("Flux") {
        (&["T5-XXL (fp8, ~4,9 ГБ)", "CLIP-L (~0,25 ГБ)"], "VAE Flux (~0,3 ГБ)")
    } else if family.starts_with("SD 3") {
        (&["T5-XXL (fp8, ~4,9 ГБ)", "CLIP-L", "CLIP-G"], "VAE SD3")
    } else if family.starts_with("Wan") {
        (&["UMT5-XXL (fp8, ~6,7 ГБ)"], "VAE Wan (~0,25 ГБ)")
    } else if family.starts_with("LTX") {
        (&["T5-XXL (fp8, ~4,9 ГБ)"], "VAE LTX")
    } else if family.starts_with("HunyuanVideo") {
        (&["LLaVA-Llama3 (fp8, ~9 ГБ)", "CLIP-L"], "VAE HunyuanVideo")
    } else if family.starts_with("SDXL") {
        (&["CLIP-L", "CLIP-G"], "VAE SDXL")
    } else {
        (&["CLIP-L"], "VAE SD 1.5")
    };
    if !has_te {
        v.extend(te.iter().map(|s| s.to_string()));
    }
    if !has_vae {
        v.push(vae.to_string());
    }
    v
}

fn dominant_dtype(s: &st::Safetensors) -> String {
    let mut by: Vec<(&str, u64)> = vec![];
    for t in &s.tensors {
        match by.iter_mut().find(|(d, _)| *d == t.dtype) {
            Some(e) => e.1 += t.bytes,
            None => by.push((&t.dtype, t.bytes)),
        }
    }
    by.sort_by_key(|e| std::cmp::Reverse(e.1));
    by.first().map(|e| e.0.to_string()).unwrap_or_default()
}

// ---------- «Светофор»: пойдёт ли модель на этом ПК ----------

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

fn fmt_bytes(b: u64) -> String {
    let gb = b as f64 / GIB as f64;
    if gb >= 1.0 {
        format!("{gb:.1} ГБ").replace('.', ",")
    } else {
        format!("{} МБ", b >> 20)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Light {
    Green,
    Yellow,
    Red,
    /// Не модель, а дополнение — оценивать отдельно нечего.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub light: Light,
    pub headline: String,
    pub details: Vec<String>,
    /// Для llama.cpp: слоёв на видеокарту и размер контекста.
    pub gpu_layers: Option<u64>,
    pub ctx: Option<u64>,
}

fn verdict(light: Light, headline: impl Into<String>) -> Verdict {
    Verdict { light, headline: headline.into(), details: vec![], gpu_layers: None, ctx: None }
}

pub fn assess(m: &ModelInfo, hw: &Hardware) -> Verdict {
    let gpu = hw.gpu.as_ref();
    let mut v = match m.kind {
        Kind::Llm if m.llm.is_some() => llm(m, hw),
        Kind::Llm => verdict(Light::Yellow, "нужна версия в формате GGUF"),
        Kind::Image | Kind::Video => diffusion(m, hw),
        Kind::SpeechToText => {
            if budget(hw) > m.weights_bytes * 2 + 512 * MIB {
                verdict(Light::Green, "пойдёт на видеокарте")
            } else {
                verdict(Light::Yellow, "пойдёт на процессоре, медленнее")
            }
        }
        _ => verdict(Light::None, "дополнение — подключается к основной модели"),
    };
    if gpu.is_some_and(|g| g.cc < (5, 0)) {
        v.light = Light::Red;
        v.details.push("видеокарта слишком старая для CUDA-сборок".into());
    }
    if gpu.is_none() && v.light == Light::Green {
        v.light = Light::Yellow;
        v.details.push("видеокарта NVIDIA не найдена — всё будет на процессоре".into());
    }
    v
}

/// Видеопамять, которую можно занять: свободная минус запас под рабочий стол Windows.
fn budget(hw: &Hardware) -> u64 {
    hw.gpu.as_ref().map_or(0, |g| g.vram_free.saturating_sub(300 * MIB))
}

fn llm(m: &ModelInfo, hw: &Hardware) -> Verdict {
    let d = m.llm.as_ref().unwrap();
    let budget = budget(hw);
    let vram_free = hw.gpu.as_ref().map_or(0, |g| g.vram_free);
    // KV-кэш в f16: K и V на каждый слой.
    let kv_per_tok = 2 * d.layers * d.heads_kv * d.head_dim * 2;
    // Контекст CUDA и буфер вычислений (логиты на батч 512 — основная часть).
    let overhead = 400 * MIB + (d.vocab * 512 * 4).max(256 * MIB);
    let weights = m.weights_bytes;
    let want_ctx = d.ctx_train.min(8192);

    let full = |ctx: u64| weights + kv_per_tok * ctx + overhead;

    if hw.gpu.is_some() && full(want_ctx) <= budget {
        let max_ctx = ((budget - weights - overhead) / kv_per_tok.max(1)).min(d.ctx_train);
        let max_ctx = max_ctx / 1024 * 1024;
        let mut v = verdict(Light::Green, "поместится в видеокарту целиком");
        v.gpu_layers = Some(d.layers + 1);
        v.ctx = Some(want_ctx);
        // Запускаем с `want_ctx`, а не с максимумом: так меньше памяти и быстрее ответ
        // на длинный вопрос. Пишем оба числа — иначе после запуска «до 8192» выглядит
        // как обман после обещанных «до 32768» (найдено в окне).
        let memory = if max_ctx > want_ctx {
            format!("память разговора {want_ctx} токенов, можно до {max_ctx}")
        } else {
            format!("память разговора {want_ctx} токенов")
        };
        v.details.push(format!(
            "занято будет ~{} из {} свободных; {memory}",
            fmt_bytes(full(want_ctx)),
            fmt_bytes(vram_free)
        ));
        if let Some(tps) = speed(weights, 0, hw) {
            v.details.push(format!("скорость примерно {tps:.0} токенов/с"));
        }
        return v;
    }

    // Частичная выгрузка: сколько слоёв влезет при контексте 4096.
    let ctx = d.ctx_train.min(4096);
    let per_layer = d.layer_bytes + kv_per_tok / d.layers.max(1) * ctx;
    let fixed = overhead + d.other_bytes / 2; // выходной слой обычно тоже на видеокарте
    let gpu_layers = if hw.gpu.is_some() && budget > fixed {
        ((budget - fixed) / per_layer.max(1)).min(d.layers)
    } else {
        0
    };
    let total_need = weights + kv_per_tok * ctx + overhead;
    if total_need > hw.ram_avail + budget {
        let mut v = verdict(Light::Red, "не хватит памяти");
        v.details.push(format!(
            "нужно ~{}, доступно {} ОЗУ + {} видеопамяти. Возьмите версию поменьше (Q4 или меньше параметров)",
            fmt_bytes(total_need),
            fmt_bytes(hw.ram_avail),
            fmt_bytes(budget)
        ));
        return v;
    }
    let on_gpu = gpu_layers * d.layer_bytes;
    let mut v = if gpu_layers == 0 {
        verdict(Light::Yellow, "только на процессоре — ответы будут медленными")
    } else {
        verdict(
            Light::Yellow,
            format!("частично на видеокарте ({gpu_layers} из {} слоёв) — медленнее", d.layers),
        )
    };
    v.gpu_layers = Some(gpu_layers);
    v.ctx = Some(ctx);
    if let Some(tps) = speed(on_gpu, weights - on_gpu, hw) {
        v.details.push(format!("скорость примерно {tps:.1} токенов/с"));
    }
    v
}

/// Прикидка для файла, которого ещё нет на диске: каталог знает только размер.
/// Размеров модели (слои, головы) без заголовка не узнать, поэтому память разговора
/// и рабочие буферы считаем долей от весов — точный расчёт будет после скачивания.
/// `active_bytes` — сколько весов читается на каждый токен: у обычной модели это все
/// веса, у модели «из частей» (MoE) — только работающая часть.
pub fn rough(weights: u64, active_bytes: u64, hw: &Hardware) -> Verdict {
    let budget = budget(hw);
    let active = if active_bytes == 0 { weights } else { active_bytes.min(weights) };
    // Контекст 4–8 тысяч токенов плюс буферы: на 7B это около гигабайта.
    let overhead = 700 * MIB + weights / 8;
    let need = weights + overhead;
    let mut v = if hw.gpu.is_some() && need <= budget {
        let mut v = verdict(Light::Green, "поместится в видеокарту целиком");
        if let Some(tps) = speed(active, 0, hw) {
            v.details.push(format!("скорость примерно {tps:.0} токенов/с"));
        }
        v
    } else if need <= budget + hw.ram_avail {
        // Часть слоёв на видеокарте, остальное в ОЗУ: скорость между двумя пределами.
        let on_gpu = budget.saturating_sub(overhead).min(weights);
        let share = on_gpu as f64 / weights.max(1) as f64;
        let tps = speed((active as f64 * share) as u64, (active as f64 * (1.0 - share)) as u64, hw);
        // Меньше трёх токенов в секунду — это слово в секунду: человек должен узнать
        // об этом до того, как скачает десяток гигабайт.
        let mut v = match tps {
            Some(t) if t < 3.0 => verdict(Light::Yellow, "поместится, но отвечать будет очень медленно"),
            _ if on_gpu == 0 => verdict(Light::Yellow, "только на процессоре — ответы будут медленными"),
            _ => verdict(Light::Yellow, "поместится частично — будет медленнее"),
        };
        if let Some(t) = tps {
            v.details.push(format!("скорость примерно {t:.1} токенов/с"));
        }
        v
    } else {
        verdict(Light::Red, "не хватит памяти — возьмите версию поменьше")
    };
    v.details.push(format!(
        "нужно ~{}, свободно {} видеопамяти и {} оперативной",
        fmt_bytes(need),
        fmt_bytes(budget),
        fmt_bytes(hw.ram_avail)
    ));
    v
}

/// Грубая оценка скорости генерации: на каждый токен читаются все веса.
/// Эффективность ~60% от пиковой пропускной способности; ОЗУ считаем 40 ГБ/с.
fn speed(gpu_bytes: u64, cpu_bytes: u64, hw: &Hardware) -> Option<f64> {
    let gpu_bw = hw.gpu.as_ref().map_or(0.0, |g| g.vram_bw as f64) * 0.6;
    let cpu_bw = 40e9 * 0.6;
    if gpu_bytes > 0 && gpu_bw == 0.0 {
        return None;
    }
    // Плюс ~1,5 мс на токен независимо от размера (запуск ядер, сэмплинг).
    let t = if gpu_bytes > 0 { gpu_bytes as f64 / gpu_bw } else { 0.0 } + cpu_bytes as f64 / cpu_bw + 0.0015;
    Some(1.0 / t)
}

fn diffusion(m: &ModelInfo, hw: &Hardware) -> Verdict {
    let f = m.family.as_str();
    // Память на вычисления при типичном разрешении. Замер на GTX 1080 (фаза 0):
    // SD 1.5 512² — пик 3,1 ГБ при весах 1,6 ГБ; SDXL 1024² — пик 6,5 ГБ при весах 4,8 ГБ.
    let activ = if f.starts_with("SD 1") || f.starts_with("SD 2") {
        GIB + GIB / 2
    } else if m.kind == Kind::Video {
        5 * GIB
    } else if f.starts_with("SDXL") {
        GIB * 7 / 4
    } else {
        GIB * 5 / 2
    };
    // Текстовый энкодер и VAE ComfyUI грузит по очереди с моделью, поэтому пиково
    // в видеокарте — сама модель в том типе, в котором её держит ComfyUI.
    let pascal = hw.gpu.as_ref().is_some_and(|g| g.cc < (7, 0));
    let mut notes = vec![];
    let model = if m.format == "GGUF" || m.precision.starts_with("F8") {
        // GGUF и fp8 остаются как есть и переводятся в рабочий тип послойно.
        m.core_bytes
    } else {
        if pascal {
            // GTX 10xx: fp16 медленный, bf16 нет — ComfyUI считает в fp32 послойно (замер фазы 0).
            notes.push("на этой видеокарте картинки считаются медленнее (нет быстрого fp16)".to_string());
        }
        m.core_params * 2
    };
    let vram_total = hw.gpu.as_ref().map_or(0, |g| g.vram_total);
    let budget = budget(hw);
    let need = model + activ;
    let mut v = if hw.gpu.is_some() && need <= budget {
        verdict(Light::Green, "поместится в видеокарту")
    } else if hw.gpu.is_some() && need <= budget + GIB {
        // ComfyUI (DynamicVRAM) сам подгружает недостающие веса по ходу — скорость почти не страдает.
        verdict(Light::Green, "поместится впритык")
    } else if model + vram_total.min(activ) <= hw.ram_avail + budget {
        verdict(Light::Yellow, "поместится частично — генерация будет заметно медленнее")
    } else {
        verdict(Light::Red, "не хватит памяти — возьмите версию поменьше (GGUF Q4 / fp8)")
    };
    v.details.push(format!("нужно ~{}, свободно {}", fmt_bytes(need), fmt_bytes(budget)));
    v.details.extend(notes);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ollivo-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    use std::path::PathBuf;

    /// Формат pickle не разбираем: в нём может быть вредоносный код.
    #[test]
    fn pickle_is_refused_with_advice() {
        let p = tmp("model.ckpt");
        std::fs::write(&p, b"PK\x03\x04prochee").unwrap();
        let m = probe(&p).unwrap();
        assert_eq!(m.kind, Kind::Unknown);
        assert!(m.notes[0].contains("safetensors"));
    }

    #[test]
    fn junk_file_is_an_error() {
        let p = tmp("readme.md");
        std::fs::write(&p, b"# not a model").unwrap();
        assert!(probe(&p).is_err());
    }

    /// Настоящие файлы из D:\Ollivo: чат в GGUF и SDXL в safetensors.
    /// `cargo test probe::tests::real_models -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_models() {
        let hw = crate::hardware::detect();
        for (file, kind) in [
            (r"D:\Ollivo\models\qwen2.5-3b-instruct-q4_k_m.gguf", Kind::Llm),
            (r"D:\Ollivo\models\checkpoints\sd_xl_base_1.0.safetensors", Kind::Image),
        ] {
            let m = probe(Path::new(file)).unwrap();
            let v = assess(&m, &hw);
            println!(
                "{file}\n  {} · {} · {} · {}\n  {:?} {}\n  {}",
                m.kind.ru(),
                m.family,
                m.precision,
                fmt_bytes(m.weights_bytes),
                v.light,
                v.headline,
                v.details.join("\n  ")
            );
            assert_eq!(m.kind, kind);
        }
    }
}
