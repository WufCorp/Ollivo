//! Текстовая модель через llama-server: запуск, готовность, прогрев, вопрос.
//!
//! llama-server слушает только 127.0.0.1 на свободном порту, веб-интерфейс выключен.
//! После загрузки модели делаем прогревочный запрос: у Vulkan первые 1–2 ответа
//! медленные из-за компиляции шейдеров (замер фазы 0: ~1,4 с до первого токена).

use crate::engines::Installed;
use crate::process::{self, Handle, Supervisor};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Большая модель с медленного диска грузится долго (SDXL в фазе 0 — до 150 с).
const LOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    pub model: PathBuf,
    /// Контекст в токенах. Простой режим — 8192 (решение фазы 0).
    #[serde(default = "default_ctx")]
    pub ctx: u32,
}

fn default_ctx() -> u32 {
    8192
}

pub struct Llm {
    pub handle: Handle,
    pub port: u16,
    pub model: PathBuf,
    pub started_in: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    pub text: String,
    pub tokens: u64,
    /// Токенов в секунду на генерации (из `timings` llama-server).
    pub speed: f64,
    /// Время до первого токена, мс (чтение запроса).
    pub prompt_ms: f64,
}

fn args(cfg: &Config, port: u16) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        cfg.model.display().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        cfg.ctx.to_string(),
        "--no-webui".into(),
    ];
    // Все слои на видеокарту; если не влезет, llama.cpp подберёт сам (fit).
    a.extend(["-ngl".into(), "999".into()]);
    a
}

/// Ошибка `start`, когда загрузку отменили (`cancel`): движок уже остановлен.
pub const CANCELLED: &str = "отменено";

/// Запускает llama-server и ждёт загрузки модели. `cancel` прерывает ожидание
/// в любой момент — большая модель грузится минутами, и «Остановить» не должно ждать.
pub async fn start(
    sup: &Supervisor,
    engine: &Installed,
    cfg: &Config,
    logs: &Path,
    cancel: &CancellationToken,
) -> Result<Llm, String> {
    if !cfg.model.is_file() {
        return Err(format!("файл модели не найден: {}", cfg.model.display()));
    }
    let port = process::free_port().map_err(|e| e.to_string())?;
    let spec = process::Spec {
        exe: engine.exe.clone(),
        args: args(cfg, port),
        log: logs.join("llama-server.log"),
    };
    let started = Instant::now();
    let handle = sup.spawn(&spec).map_err(|e| format!("движок не запустился: {e}"))?;
    let health = format!("http://127.0.0.1:{port}/health");
    let ready = tokio::select! {
        r = process::wait_ready(&handle, &health, LOAD_TIMEOUT) => r.map_err(|e| e.to_string()),
        _ = cancel.cancelled() => Err(CANCELLED.to_string()),
    };
    if let Err(e) = ready {
        handle.stop().await;
        return Err(e);
    }
    // Прогрев: ошибка здесь не критична — модель уже загружена.
    tokio::select! {
        _ = ask(port, "Hi", 1) => {}
        _ = cancel.cancelled() => {
            handle.stop().await;
            return Err(CANCELLED.to_string());
        }
    }
    Ok(Llm { handle, port, model: cfg.model.clone(), started_in: started.elapsed() })
}

/// Вопрос без стриминга — для проверки движка. Чат со стримингом — фаза 2.
pub async fn ask(port: u16, prompt: &str, max_tokens: u32) -> Result<Answer, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let body = serde_json::json!({
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
    });
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .body(body.to_string())
        .header("content-type", "application/json")
        .send()
        .await
        .map_err(|e| format!("движок не отвечает: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("движок ответил ошибкой {}", resp.status().as_u16()));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let text = v["choices"][0]["message"]["content"].as_str().unwrap_or_default().trim().to_string();
    Ok(Answer {
        text,
        tokens: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        speed: v["timings"]["predicted_per_second"].as_f64().unwrap_or(0.0),
        prompt_ms: v["timings"]["prompt_ms"].as_f64().unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_bind_localhost_without_webui() {
        let cfg = Config { model: PathBuf::from(r"D:\Ollivo\models\m.gguf"), ctx: 8192 };
        let a = args(&cfg, 5000).join(" ");
        assert!(a.contains("--host 127.0.0.1 --port 5000"));
        assert!(a.contains("--no-webui"));
        assert!(a.contains(r"-m D:\Ollivo\models\m.gguf"));
    }

    /// Настоящий llama-server из D:\Ollivo с моделью 0.5B: запуск, вопрос, остановка.
    /// `cargo test llm::tests::real_start_ask_stop -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_start_ask_stop() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let cfg = Config { model: root.join(r"models\qwen2.5-0.5b-instruct-q4_k_m.gguf"), ctx: 4096 };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &std::env::temp_dir().join("ollivo-llm-test"), &CancellationToken::new())
            .await
            .unwrap();
        println!("готов за {:.1} с, порт {}", llm.started_in.as_secs_f64(), llm.port);
        let a = ask(llm.port, "Столица Франции? Одно слово.", 16).await.unwrap();
        println!("{a:?}");
        assert!(!a.text.is_empty() && a.speed > 0.0);
        let e = llm.handle.stop().await;
        assert!(e.by_us);
    }

    #[tokio::test]
    async fn missing_model_is_clear_error() {
        let engine = Installed {
            id: "llama.cpp".into(),
            version: "b1".into(),
            build: crate::hardware::Build::Vulkan,
            dir: PathBuf::new(),
            exe: PathBuf::from("llama-server.exe"),
        };
        let cfg = Config { model: PathBuf::from(r"Z:\нет.gguf"), ctx: 4096 };
        let err = start(&Supervisor::new(), &engine, &cfg, &std::env::temp_dir(), &CancellationToken::new())
            .await
            .err()
            .unwrap();
        assert!(err.contains("не найден"));
    }

    /// Отмена во время загрузки: не ждём таймаута, процесс остановлен.
    /// Вместо llama-server — ping, который никогда не ответит на /health.
    #[tokio::test]
    async fn cancel_during_load_stops_engine() {
        let dir = std::env::temp_dir().join(format!("ollivo-llm-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("m.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        let engine = Installed {
            id: "llama.cpp".into(),
            version: "b1".into(),
            build: crate::hardware::Build::Vulkan,
            dir: PathBuf::new(),
            exe: PathBuf::from(r"C:\Windows\System32\PING.EXE"),
        };
        let cfg = Config { model, ctx: 4096 };
        let sup = Supervisor::new();
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            c.cancel();
        });
        let t = Instant::now();
        let err = start(&sup, &engine, &cfg, &dir.join("logs"), &cancel).await.err().unwrap();
        assert_eq!(err, CANCELLED);
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
