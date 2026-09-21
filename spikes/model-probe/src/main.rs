//! Прототип фазы 0: определить тип модели по файлу и оценить, пойдёт ли она на этом ПК.
//!
//! model-probe [--json] [--vram ГБ] [--ram ГБ] <файл>...

mod classify;
mod fit;
mod gguf;
mod safetensors;

use std::path::PathBuf;

pub fn fmt_bytes(b: u64) -> String {
    let gb = b as f64 / (1u64 << 30) as f64;
    if gb >= 1.0 {
        format!("{gb:.1} ГБ").replace('.', ",")
    } else {
        format!("{} МБ", b >> 20)
    }
}

fn fmt_params(p: u64) -> String {
    if p >= 1_000_000_000 {
        format!("{:.1}B", p as f64 / 1e9)
    } else {
        format!("{}M", p / 1_000_000)
    }
}

fn main() {
    let mut json = false;
    let mut files = vec![];
    let mut hw = fit::detect();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let num = |args: &mut dyn Iterator<Item = String>| -> u64 {
            let gb: f64 = args.next().and_then(|s| s.parse().ok()).expect("нужно число ГБ");
            (gb * (1u64 << 30) as f64) as u64
        };
        match a.as_str() {
            "--json" => json = true,
            // Симуляция другого железа: --vram 8 --ram 16
            "--vram" => {
                let v = num(&mut args);
                hw.vram_total = v;
                hw.vram_free = v;
            }
            "--ram" => {
                let v = num(&mut args);
                hw.ram_total = v;
                hw.ram_avail = v;
            }
            _ => files.push(PathBuf::from(a)),
        }
    }

    if !json {
        println!(
            "ПК: {} — {} видеопамяти (свободно {}), CC {}.{}, драйвер {} (CUDA {}.{}); ОЗУ {} (свободно {})\n",
            hw.gpu.as_deref().unwrap_or("видеокарта NVIDIA не найдена"),
            fmt_bytes(hw.vram_total),
            fmt_bytes(hw.vram_free),
            hw.cc.0,
            hw.cc.1,
            hw.driver,
            hw.cuda_driver / 1000,
            hw.cuda_driver % 1000 / 10,
            fmt_bytes(hw.ram_total),
            fmt_bytes(hw.ram_avail),
        );
    }

    let mut out = vec![];
    for path in files {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        match classify::probe(&path) {
            Ok(m) => {
                let v = fit::assess(&m, &hw);
                if json {
                    out.push(serde_json::json!({ "file": name, "model": m, "verdict": v }));
                    continue;
                }
                let light = match v.light {
                    fit::Light::Green => "🟢",
                    fit::Light::Yellow => "🟡",
                    fit::Light::Red => "🔴",
                    fit::Light::None => "⚪",
                };
                println!("{name}");
                println!("  {} · {} · {:?}", m.kind.ru(), m.family, m.engine);
                let params = if m.params > 0 { format!(" · {} параметров", fmt_params(m.params)) } else { String::new() };
                println!("  {}{params} · {} · {}", m.format, m.precision, fmt_bytes(m.weights_bytes));
                if let Some(n) = &m.name {
                    println!("  название: {n}");
                }
                if let Some(l) = &m.license {
                    println!("  лицензия: {l}");
                }
                if let Some(d) = &m.llm {
                    println!(
                        "  слоёв {}, контекст до {}, KV-голов {} × {}, словарь {}",
                        d.layers, d.ctx_train, d.heads_kv, d.head_dim, d.vocab
                    );
                }
                if !m.contains.is_empty() {
                    println!("  внутри: {}", m.contains.join(", "));
                }
                if !m.needs.is_empty() {
                    println!("  ещё нужно: {}", m.needs.join(", "));
                }
                for n in &m.notes {
                    println!("  · {n}");
                }
                println!("  {light} {}", v.headline);
                for d in &v.details {
                    println!("     {d}");
                }
                if let (Some(l), Some(c)) = (v.gpu_layers, v.ctx) {
                    println!("     запуск: -ngl {l} -c {c}");
                }
                println!();
            }
            Err(e) => {
                if json {
                    out.push(serde_json::json!({ "file": name, "error": e.to_string() }));
                } else {
                    println!("{name}\n  ошибка: {e:#}\n");
                }
            }
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    }
}
