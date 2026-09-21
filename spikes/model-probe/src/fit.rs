//! «Светофор»: пойдёт ли модель на этом ПК и с какими настройками.

use crate::classify::{Kind, ModelInfo};
use crate::fmt_bytes;
use serde::Serialize;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

#[derive(Debug, Clone, Serialize)]
pub struct Hardware {
    pub gpu: Option<String>,
    pub vram_total: u64,
    pub vram_free: u64,
    /// Compute capability, например (6, 1) у GTX 10xx.
    pub cc: (u32, u32),
    pub driver: String,
    /// Версия CUDA, которую поддерживает драйвер, например 12090.
    pub cuda_driver: i32,
    /// Пропускная способность видеопамяти, байт/с (оценка).
    pub vram_bw: u64,
    pub ram_total: u64,
    pub ram_avail: u64,
}

pub fn detect() -> Hardware {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let mut hw = Hardware {
        gpu: None,
        vram_total: 0,
        vram_free: 0,
        cc: (0, 0),
        driver: String::new(),
        cuda_driver: 0,
        vram_bw: 0,
        ram_total: sys.total_memory(),
        ram_avail: sys.available_memory(),
    };
    let Ok(nvml) = nvml_wrapper::Nvml::init() else { return hw };
    hw.driver = nvml.sys_driver_version().unwrap_or_default();
    hw.cuda_driver = nvml.sys_cuda_driver_version().unwrap_or(0);
    let Ok(dev) = nvml.device_by_index(0) else { return hw };
    hw.gpu = dev.name().ok();
    if let Ok(m) = dev.memory_info() {
        hw.vram_total = m.total;
        hw.vram_free = m.free;
    }
    if let Ok(c) = dev.cuda_compute_capability() {
        hw.cc = (c.major as u32, c.minor as u32);
    }
    let bus = dev.memory_bus_width().unwrap_or(0) as u64;
    let clk = dev
        .max_clock_info(nvml_wrapper::enum_wrappers::device::Clock::Memory)
        .unwrap_or(0) as u64;
    hw.vram_bw = clk * 1_000_000 * 2 * bus / 8;
    hw
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
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
    let mut v = match m.kind {
        Kind::Llm if m.llm.is_some() => llm(m, hw),
        Kind::Llm => verdict(Light::Yellow, "нужна версия в формате GGUF"),
        Kind::Image | Kind::Video => diffusion(m, hw),
        Kind::SpeechToText => {
            if hw.vram_free > m.weights_bytes * 2 + 512 * MIB {
                verdict(Light::Green, "пойдёт на видеокарте")
            } else {
                verdict(Light::Yellow, "пойдёт на процессоре, медленнее")
            }
        }
        _ => verdict(Light::None, "дополнение — подключается к основной модели"),
    };
    if hw.gpu.is_some() && hw.cc < (5, 0) {
        v.light = Light::Red;
        v.details.push("видеокарта слишком старая для CUDA-сборок".into());
    }
    if hw.gpu.is_none() && v.light == Light::Green {
        v.light = Light::Yellow;
        v.details.push("видеокарта NVIDIA не найдена — всё будет на процессоре".into());
    }
    v
}

/// Видеопамять, которую можно занять: свободная минус запас под рабочий стол Windows.
fn vram_budget(hw: &Hardware) -> u64 {
    hw.vram_free.saturating_sub(300 * MIB)
}

fn llm(m: &ModelInfo, hw: &Hardware) -> Verdict {
    let d = m.llm.as_ref().unwrap();
    let budget = vram_budget(hw);
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
        v.details.push(format!(
            "занято будет ~{} из {} свободных; память разговора до {} токенов",
            fmt_bytes(full(want_ctx)),
            fmt_bytes(hw.vram_free),
            max_ctx
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

/// Грубая оценка скорости генерации: на каждый токен читаются все веса.
/// Эффективность ~60% от пиковой пропускной способности; ОЗУ считаем 40 ГБ/с.
fn speed(gpu_bytes: u64, cpu_bytes: u64, hw: &Hardware) -> Option<f64> {
    let gpu_bw = hw.vram_bw as f64 * 0.6;
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
    let pascal = hw.gpu.is_some() && hw.cc < (7, 0);
    let mut notes = vec![];
    let model = if m.format == "GGUF" || m.precision.starts_with("F8") {
        // GGUF и fp8 остаются как есть и переводятся в рабочий тип послойно.
        m.core_bytes
    } else {
        if pascal {
            // GTX 10xx: fp16 медленный, bf16 нет. ComfyUI под Windows, по нашим данным, хранит веса
            // в fp16 и считает в fp32 послойно — память та же, скорость ниже. Проверить в фазе 0.
            notes.push("на этой видеокарте картинки считаются медленнее (нет быстрого fp16)".to_string());
        }
        m.core_params * 2
    };
    let budget = vram_budget(hw);
    let need = model + activ;
    let mut v = if hw.gpu.is_some() && need <= budget {
        verdict(Light::Green, "поместится в видеокарту")
    } else if hw.gpu.is_some() && need <= budget + GIB {
        // ComfyUI (DynamicVRAM) сам подгружает недостающие веса по ходу — скорость почти не страдает.
        verdict(Light::Green, "поместится впритык")
    } else if model + hw.vram_total.min(activ) <= hw.ram_avail + budget {
        verdict(Light::Yellow, "поместится частично — генерация будет заметно медленнее")
    } else {
        verdict(Light::Red, "не хватит памяти — возьмите версию поменьше (GGUF Q4 / fp8)")
    };
    v.details.push(format!("нужно ~{}, свободно {}", fmt_bytes(need), fmt_bytes(budget)));
    v.details.extend(notes);
    v
}
