//! Текстовая модель через llama-server: запуск, готовность, прогрев, вопрос.
//!
//! llama-server слушает только 127.0.0.1 на свободном порту, веб-интерфейс выключен.
//! После загрузки модели делаем прогревочный запрос: у Vulkan первые 1–2 ответа
//! медленные из-за компиляции шейдеров (замер фазы 0: ~1,4 с до первого токена).

use crate::engines::Installed;
use crate::process::{self, Handle, Supervisor};
use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Большая модель с медленного диска грузится долго (SDXL в фазе 0 — до 150 с).
const LOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Настройки запуска. Подбираются заранее под свободную видеопамять
/// (`plan` в `lib.rs`), здесь уже готовые числа.
#[derive(Debug, Clone)]
pub struct Config {
    pub model: PathBuf,
    /// Память разговора в токенах: сколько модель помнит.
    pub ctx: u32,
    /// Сколько слоёв считает видеокарта; 0 — всё на процессоре.
    pub gpu_layers: u32,
}

pub struct Llm {
    pub handle: Handle,
    pub port: u16,
    pub model: PathBuf,
    /// С чем запустили: окно показывает это в карточке модели.
    pub ctx: u32,
    pub gpu_layers: u32,
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
    // Сколько слоёв уйдёт на видеокарту, посчитано заранее по свободной памяти.
    a.extend(["-ngl".into(), cfg.gpu_layers.to_string()]);
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
    Ok(Llm {
        handle,
        port,
        model: cfg.model.clone(),
        ctx: cfg.ctx,
        gpu_layers: cfg.gpu_layers,
        started_in: started.elapsed(),
    })
}

/// Реплика разговора. Роли как у OpenAI: `system`, `user`, `assistant`.
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
pub struct Msg {
    pub role: String,
    pub content: String,
}

/// Чем закончился ответ: сколько токенов и как быстро.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub tokens: u64,
    /// Токенов в секунду на генерации.
    pub speed: f64,
    /// Время до первого токена, мс.
    pub prompt_ms: f64,
}

/// Ответ по кускам: каждый кусок текста уходит в `on_delta` сразу, как пришёл.
/// `cancel` — кнопка «Остановить»: обрываем соединение, движок прекращает считать.
pub async fn chat(
    port: u16,
    messages: &[Msg],
    cancel: &CancellationToken,
    on_delta: impl Fn(&str),
) -> Result<Stats, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        // Ограничения по времени нет: длинный ответ на медленном ПК идёт долго,
        // а обрыв — дело кнопки «Остановить».
        .build()
        .map_err(|e| e.to_string())?;
    let body = serde_json::json!({
        "messages": messages,
        "stream": true,
        // Просим итоговые числа в последнем куске.
        "stream_options": {"include_usage": true},
        "timings_per_token": true,
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

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut stats = Stats::default();
    loop {
        let chunk = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => return Ok(stats), // ответ обрываем, что успели — уже показано
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|e| format!("связь с движком оборвалась: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        // Server-sent events: события разделены пустой строкой, данные — в строках `data: `.
        while let Some(end) = buf.find("\n\n") {
            let event: String = buf.drain(..end + 2).collect();
            for line in event.lines() {
                let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
                if data == "[DONE]" {
                    return Ok(stats);
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
                if let Some(err) = v["error"]["message"].as_str() {
                    return Err(err.to_string());
                }
                if let Some(text) = v["choices"][0]["delta"]["content"].as_str() {
                    if !text.is_empty() {
                        on_delta(text);
                    }
                }
                if let Some(t) = v.get("timings").filter(|t| !t.is_null()) {
                    stats.speed = t["predicted_per_second"].as_f64().unwrap_or(stats.speed);
                    stats.prompt_ms = t["prompt_ms"].as_f64().unwrap_or(stats.prompt_ms);
                    stats.tokens = t["predicted_n"].as_u64().unwrap_or(stats.tokens);
                }
                if let Some(n) = v["usage"]["completion_tokens"].as_u64() {
                    stats.tokens = n;
                }
            }
        }
    }
    Ok(stats)
}

/// Вопрос без стриминга — для проверки движка.
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
        let cfg = Config { model: PathBuf::from(r"D:\Ollivo\models\m.gguf"), ctx: 8192, gpu_layers: 21 };
        let a = args(&cfg, 5000).join(" ");
        assert!(a.contains("--host 127.0.0.1 --port 5000"));
        assert!(a.contains("--no-webui"));
        assert!(a.contains(r"-m D:\Ollivo\models\m.gguf"));
        // Подобранные настройки доходят до движка.
        assert!(a.contains("-c 8192") && a.contains("-ngl 21"), "{a}");
    }

    /// Настоящий llama-server из D:\Ollivo с моделью 0.5B: запуск, вопрос, остановка.
    /// `cargo test llm::tests::real_start_ask_stop -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_start_ask_stop() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let cfg = Config { model: root.join(r"models\qwen2.5-0.5b-instruct-q4_k_m.gguf"), ctx: 4096, gpu_layers: 999 };
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

    /// Стриминг на настоящем движке: куски приходят по ходу, «Остановить» обрывает ответ.
    /// `cargo test llm::tests::real_chat_stream -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_chat_stream() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let cfg = Config { model: root.join(r"models\qwen2.5-0.5b-instruct-q4_k_m.gguf"), ctx: 4096, gpu_layers: 999 };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &std::env::temp_dir().join("ollivo-chat-test"), &CancellationToken::new())
            .await
            .unwrap();

        let msgs = vec![Msg { role: "user".into(), content: "Посчитай вслух от 1 до 20.".into() }];
        let chunks = std::sync::Mutex::new(Vec::<String>::new());
        let stats = chat(llm.port, &msgs, &CancellationToken::new(), |t| {
            chunks.lock().unwrap().push(t.to_string())
        })
        .await
        .unwrap();
        let chunks = chunks.into_inner().unwrap();
        println!("кусков {}, {:?}", chunks.len(), stats);
        println!("{}", chunks.concat());
        assert!(chunks.len() > 5, "ответ должен приходить кусками");
        assert!(stats.tokens > 0 && stats.speed > 0.0);

        // «Остановить» через полсекунды: ждём не дольше, чем ответ целиком.
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            c.cancel();
        });
        let t = Instant::now();
        let long = vec![Msg { role: "user".into(), content: "Напиши рассказ на 2000 слов.".into() }];
        chat(llm.port, &long, &cancel, |_| {}).await.unwrap();
        println!("остановлено за {:.1} с", t.elapsed().as_secs_f64());
        assert!(t.elapsed() < Duration::from_secs(5));
        llm.handle.stop().await;
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
        let cfg = Config { model: PathBuf::from(r"Z:\нет.gguf"), ctx: 4096, gpu_layers: 999 };
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
        let cfg = Config { model, ctx: 4096, gpu_layers: 999 };
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
