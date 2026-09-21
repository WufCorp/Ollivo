//! Определение типа модели по заголовку файла: что это, какой движок нужен, что ещё докачать.

use crate::{gguf, safetensors as st};
use anyhow::{Result, bail};
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Engine {
    LlamaCpp,
    ComfyUi,
    WhisperCpp,
    /// Формат не запускается нашими движками без конвертации.
    NeedsConversion,
    None,
}

/// Размеры текстовой модели — нужны для расчёта памяти.
#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub format: &'static str,
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
    pub contains: Vec<&'static str>,
    /// Что ещё нужно скачать, чтобы модель заработала.
    pub needs: Vec<String>,
    pub notes: Vec<String>,
}

impl ModelInfo {
    fn new(format: &'static str) -> Self {
        ModelInfo {
            format,
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
        m.contains.push(if kind == Kind::Video { "видеомодель" } else { "UNet/DiT" });
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
    m.contains.push(if m.kind == Kind::Video { "видеомодель" } else { "UNet/DiT" });
    if has_te {
        m.contains.push("текстовый энкодер");
    }
    if has_vae {
        m.contains.push("VAE");
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
        m.notes.push(format!("из них сама модель: {}", crate::fmt_bytes(m.core_bytes)));
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
