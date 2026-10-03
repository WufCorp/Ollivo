//! ComfyUI без окна: запуск, задача через `/prompt`, ход по websocket, итог — файлы на диске.
//!
//! - Python запускается с `-E -s`: переменные `PYTHON*` и пакеты из профиля человека
//!   (`%APPDATA%\Python`) не попадают в наш Python — у кого-то там стоит свой torch.
//! - Всё, что ComfyUI пишет сам (настройки, база, временное), — в `engines\comfyui\data`
//!   (`--base-directory`), а не в папку сборки: сборку сверяет «Починить» и заменяет
//!   обновление, а `data` без метки они не трогают. Рядом с движком — чтобы переезжала
//!   вместе с папкой программы.
//! - Веб-интерфейс не ставится: `--front-end-root` смотрит на пустую страницу.
//! - `--disable-api-nodes`: платные облачные узлы ComfyUI не грузим — программа
//!   не должна ходить в сеть без спроса, а запуск быстрее.
//! - Модели не копируются: папки с ними перечислены в `model-paths.yaml`.

use crate::process::{self, Handle, Supervisor};
use crate::pyenv::Env;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

/// Первый запуск после установки дольше: ComfyUI проверяет узлы и видеокарту.
/// Фаза 0: 73 с без .pyc, 9 с с ними; на медленном диске — больше.
const START_TIMEOUT: Duration = Duration::from_secs(240);

pub struct Comfy {
    pub handle: Handle,
    pub port: u16,
    pub output: PathBuf,
    /// Сколько запускался: из этого замера — оценка времени до первой картинки.
    pub started_in: Duration,
    client: reqwest::Client,
}

#[derive(Debug)]
pub enum Error {
    Start(String),
    Rejected(String),
    /// Ошибка при генерации: узел и текст исключения из ComfyUI.
    Failed(String),
    Cancelled,
    Link(String),
}

// Вручную, а не `thiserror`: текст зависит от языка программы.
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            Error::Start(e) => tf!("движок картинок не запустился: {e}", "the image engine did not start: {e}"),
            Error::Rejected(e) => tf!("движок картинок не принял задачу: {e}", "the image engine rejected the task: {e}"),
            Error::Failed(e) => e.clone(),
            Error::Cancelled => t!("генерация остановлена", "generation stopped").to_string(),
            Error::Link(e) => tf!("связь с движком картинок: {e}", "link to the image engine: {e}"),
        })
    }
}

impl std::error::Error for Error {}

/// Ход генерации для окна.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Progress {
    /// Шаг и сколько всего (у KSampler — шаги, у VAE — плитки). 0 из 0 — идёт загрузка.
    pub value: u32,
    pub max: u32,
    /// Узел, который работает сейчас (id из workflow).
    pub node: Option<String>,
}

/// Где ComfyUI держит своё: `engines\comfyui\data`.
pub fn base_dir(root: &Path) -> PathBuf {
    root.join("engines").join(crate::pyenv::COMFY).join("data")
}

/// Аргументы запуска. `main.py` — абсолютным путём, рабочая папка — папка Python.
fn args(env: &Env, root: &Path, port: u16, output: &Path) -> Vec<String> {
    let base = base_dir(root);
    let s = |p: &Path| p.to_string_lossy().into_owned();
    [
        "-E", "-s", "-X", "utf8", "-u",
    ]
    .into_iter()
    .map(String::from)
    .chain([
        s(&env.comfy.join("main.py")),
        "--listen".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--disable-auto-launch".into(),
        "--disable-api-nodes".into(),
        "--base-directory".into(),
        s(&base),
        "--front-end-root".into(),
        s(&base.join("noui")),
        "--extra-model-paths-config".into(),
        s(&base.join("model-paths.yaml")),
        "--output-directory".into(),
        s(output),
    ])
    .collect()
}

/// `model-paths.yaml`: каждая папка с моделями — во все виды моделей сразу. ComfyUI ищет
/// файл по имени внутри папки вида, поэтому одна и та же папка может быть и `checkpoints`,
/// и `vae`: лишнего он не загрузит. Пути — строками JSON: это правильный YAML и никаких
/// сюрпризов с `\` и кириллицей.
fn model_paths_yaml(dirs: &[PathBuf]) -> String {
    const KINDS: &[&str] = &[
        "checkpoints", "diffusion_models", "unet", "vae", "text_encoders", "clip", "loras",
        "upscale_models", "controlnet", "embeddings",
    ];
    let mut out = String::new();
    for (i, d) in dirs.iter().enumerate() {
        let base = serde_json::to_string(&d.to_string_lossy().replace('\\', "/")).unwrap();
        out += &format!("ollivo{i}:\n  base_path: {base}\n");
        for k in KINDS {
            out += &format!("  {k}: \".\"\n");
        }
    }
    out
}

/// Запускает ComfyUI и ждёт, пока он ответит. `model_dirs` — папки, где лежат модели картинок.
pub async fn start(
    sup: &Supervisor,
    env: &Env,
    root: &Path,
    model_dirs: &[PathBuf],
    output: &Path,
    cancel: &CancellationToken,
) -> Result<Comfy, Error> {
    let base = base_dir(root);
    let io = |e: std::io::Error| Error::Start(e.to_string());
    std::fs::create_dir_all(base.join("noui")).map_err(io)?;
    // С `--base-directory` ComfyUI сам её не создаёт и падает при старте.
    std::fs::create_dir_all(base.join("custom_nodes")).map_err(io)?;
    std::fs::create_dir_all(output).map_err(io)?;
    std::fs::write(base.join("noui").join("index.html"), "Ollivo").map_err(io)?;
    std::fs::write(base.join("model-paths.yaml"), model_paths_yaml(model_dirs)).map_err(io)?;

    let port = process::free_port().map_err(io)?;
    let spec = process::Spec {
        exe: env.python.clone(),
        args: args(env, root, port, output),
        log: root.join("logs").join("comfyui.log"),
    };
    let started = Instant::now();
    let handle = sup.spawn(&spec).map_err(io)?;
    let stats = format!("http://127.0.0.1:{port}/system_stats");
    let ready = tokio::select! {
        r = process::wait_ready(&handle, &stats, START_TIMEOUT) => r,
        _ = cancel.cancelled() => {
            handle.stop().await;
            return Err(Error::Cancelled);
        }
    };
    if let Err(e) = ready {
        handle.stop().await;
        return Err(Error::Start(e.to_string()));
    }
    let client = reqwest::Client::builder().no_proxy().build().map_err(|e| Error::Start(e.to_string()))?;
    Ok(Comfy { handle, port, output: output.to_path_buf(), started_in: started.elapsed(), client })
}

impl Comfy {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Выгружает модели из видеокарты: её просит модель чата. Сам ComfyUI остаётся
    /// запущенным (~0,3 ГБ на контекст CUDA) — следующая картинка не ждёт 30 с запуска.
    pub async fn free(&self) {
        let body = json!({ "unload_models": true, "free_memory": true });
        let _ = self.client.post(self.url("/free")).json(&body).send().await;
    }

    /// Выполняет workflow (формат API ComfyUI) и возвращает пути к готовым картинкам.
    pub async fn run(
        &self,
        workflow: &Value,
        cancel: &CancellationToken,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<Vec<PathBuf>, Error> {
        let link = |e: &dyn std::fmt::Display| Error::Link(e.to_string());
        let client_id = format!("ollivo-{}-{}", std::process::id(), rand_suffix());
        // Подписываемся до отправки задачи: короткая задача может закончиться раньше, чем
        // мы бы успели подключиться, и сообщение о конце потерялось бы.
        let ws_url = format!("ws://127.0.0.1:{}/ws?clientId={client_id}", self.port);
        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.map_err(|e| link(&e))?;

        let body = json!({ "prompt": workflow, "client_id": client_id });
        let resp = self.client.post(self.url("/prompt")).json(&body).send().await.map_err(|e| link(&e))?;
        let status = resp.status();
        let answer: Value = resp.json().await.map_err(|e| link(&e))?;
        if !status.is_success() {
            return Err(Error::Rejected(rejection(&answer)));
        }
        let prompt_id = answer["prompt_id"].as_str().ok_or_else(|| Error::Rejected(answer.to_string()))?.to_string();

        loop {
            let msg = tokio::select! {
                m = ws.next() => m,
                _ = cancel.cancelled() => {
                    // Прерывает текущую задачу; наша — единственная в очереди.
                    let _ = self.client.post(self.url("/interrupt")).send().await;
                    let _ = ws.close(None).await;
                    return Err(Error::Cancelled);
                }
            };
            let text = match msg {
                Some(Ok(Message::Text(t))) => t,
                Some(Ok(Message::Ping(p))) => {
                    let _ = ws.send(Message::Pong(p)).await;
                    continue;
                }
                // Двоичные сообщения — превью шагов, их не заказываем.
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(link(&e)),
                None => return Err(Error::Link(t!("движок закрыл соединение", "the engine closed the connection").into())),
            };
            let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
            let data = &event["data"];
            if data.get("prompt_id").and_then(Value::as_str).is_some_and(|p| p != prompt_id) {
                continue;
            }
            match event["type"].as_str().unwrap_or("") {
                "progress" => on_progress(Progress {
                    value: data["value"].as_u64().unwrap_or(0) as u32,
                    max: data["max"].as_u64().unwrap_or(0) as u32,
                    node: data["node"].as_str().map(String::from),
                }),
                "executing" if !data["node"].is_null() => {
                    on_progress(Progress { value: 0, max: 0, node: data["node"].as_str().map(String::from) })
                }
                "execution_error" => return Err(Error::Failed(failure(data))),
                "execution_interrupted" => return Err(Error::Cancelled),
                // Конец задачи: у новых версий — `execution_success`, у старых — `executing` с node = null.
                "execution_success" | "executing" => break,
                _ => {}
            }
        }
        let _ = ws.close(None).await;

        // «Готово» по websocket приходит раньше, чем ComfyUI записывает итог в историю
        // (`task_done` идёт после `execute`): сразу спросить — бывает пусто. Ждём запись.
        let started = Instant::now();
        let history = loop {
            let h: Value = self
                .client
                .get(self.url(&format!("/history/{prompt_id}")))
                .send()
                .await
                .map_err(|e| link(&e))?
                .json()
                .await
                .map_err(|e| link(&e))?;
            if h.get(&prompt_id).is_some() {
                break h;
            }
            if started.elapsed() > Duration::from_secs(30) {
                return Err(Error::Link(t!("движок не записал итог задачи", "the engine did not record the task result").into()));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        Ok(output_files(&history[&prompt_id], &self.output))
    }
}

/// Картинки из ответа `/history`: только `type: output` (не временные превью)
/// и только внутри папки вывода — имена пришли от процесса, но проверяем всё равно.
fn output_files(entry: &Value, output: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let Some(nodes) = entry["outputs"].as_object() else { return out };
    for node in nodes.values() {
        for img in node["images"].as_array().into_iter().flatten() {
            if img["type"] != "output" {
                continue;
            }
            let (Some(name), sub) = (img["filename"].as_str(), img["subfolder"].as_str().unwrap_or("")) else { continue };
            let rel = Path::new(sub).join(name);
            if rel.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
                out.push(output.join(rel));
            }
        }
    }
    out
}

/// Почему ComfyUI не принял задачу: общая ошибка и ошибки узлов.
fn rejection(answer: &Value) -> String {
    let mut parts = vec![];
    if let Some(m) = answer["error"]["message"].as_str() {
        parts.push(m.to_string());
    }
    if let Some(d) = answer["error"]["details"].as_str().filter(|d| !d.is_empty()) {
        parts.push(d.to_string());
    }
    for (node, e) in answer["node_errors"].as_object().into_iter().flatten() {
        for err in e["errors"].as_array().into_iter().flatten() {
            let class = e["class_type"].as_str().unwrap_or("");
            parts.push(format!(
                "{node} {class}: {} {}",
                err["message"].as_str().unwrap_or(""),
                err["details"].as_str().unwrap_or("")
            ));
        }
    }
    if parts.is_empty() {
        answer.to_string()
    } else {
        parts.join("; ")
    }
}

fn failure(data: &Value) -> String {
    format!(
        "{} ({}): {}",
        data["node_type"].as_str().unwrap_or("?"),
        data["exception_type"].as_str().unwrap_or(""),
        data["exception_message"].as_str().unwrap_or("").trim()
    )
}

/// Уникальная часть id клиента: время в наносекундах, криптостойкость не нужна.
fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// Картинка по описанию для моделей-чекпойнтов (SD 1.5, SDXL): модель, два описания,
/// пустой латент, семплер, декодер, сохранение. Параметры — уже решённые ядром.
pub struct Txt2Img<'a> {
    pub checkpoint: &'a str,
    pub prompt: &'a str,
    pub negative: &'a str,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub seed: u64,
    pub batch: u32,
    pub sampler: &'a str,
    pub scheduler: &'a str,
    pub prefix: &'a str,
}

impl Txt2Img<'_> {
    pub fn workflow(&self) -> Value {
        json!({
            "1": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": self.checkpoint}},
            "2": {"class_type": "CLIPTextEncode", "inputs": {"clip": ["1", 1], "text": self.prompt}},
            "3": {"class_type": "CLIPTextEncode", "inputs": {"clip": ["1", 1], "text": self.negative}},
            "4": {"class_type": "EmptyLatentImage", "inputs": {"width": self.width, "height": self.height, "batch_size": self.batch}},
            "5": {"class_type": "KSampler", "inputs": {
                "model": ["1", 0], "positive": ["2", 0], "negative": ["3", 0], "latent_image": ["4", 0],
                "seed": self.seed, "steps": self.steps, "cfg": self.cfg,
                "sampler_name": self.sampler, "scheduler": self.scheduler, "denoise": 1.0}},
            "6": {"class_type": "VAEDecode", "inputs": {"samples": ["5", 0], "vae": ["1", 2]}},
            "7": {"class_type": "SaveImage", "inputs": {"images": ["6", 0], "filename_prefix": self.prefix}},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_paths_survive_backslashes_and_cyrillic() {
        let y = model_paths_yaml(&[PathBuf::from(r"D:\Иван Петров\models"), PathBuf::from(r"E:\ComfyUI\models\checkpoints")]);
        assert!(y.contains("ollivo0:\n  base_path: \"D:/Иван Петров/models\"\n  checkpoints: \".\"\n"), "{y}");
        assert!(y.contains("ollivo1:\n  base_path: \"E:/ComfyUI/models/checkpoints\"\n"));
        assert!(y.contains("  vae: \".\"\n"));
    }

    #[test]
    fn args_keep_comfy_away_from_network_and_its_own_folder() {
        let env = Env {
            python: PathBuf::from(r"D:\Ollivo\engines\python\3.12.14-cuda12\python.exe"),
            comfy: PathBuf::from(r"D:\Ollivo\engines\comfyui\0.37.0-cpu\ComfyUI-0.37.0"),
            build: crate::hardware::Build::Cuda12,
        };
        let a = args(&env, Path::new(r"D:\Ollivo"), 8200, Path::new(r"D:\Ollivo\images"));
        assert_eq!(&a[..5], ["-E", "-s", "-X", "utf8", "-u"]);
        assert!(a[5].ends_with("main.py"));
        let after = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].clone());
        assert_eq!(after("--listen").as_deref(), Some("127.0.0.1"));
        assert_eq!(after("--base-directory").as_deref(), Some(r"D:\Ollivo\engines\comfyui\data"));
        assert!(a.contains(&"--disable-api-nodes".to_string()));
    }

    #[test]
    fn outputs_only_final_images_inside_folder() {
        let entry = json!({"outputs": {
            "7": {"images": [
                {"filename": "a_00001_.png", "subfolder": "", "type": "output"},
                {"filename": "b.png", "subfolder": "day", "type": "output"},
                {"filename": "p.png", "subfolder": "", "type": "temp"},
                {"filename": "x.png", "subfolder": "../..", "type": "output"}
            ]}
        }});
        let out = Path::new(r"D:\Ollivo\images");
        assert_eq!(output_files(&entry, out), vec![out.join("a_00001_.png"), out.join("day").join("b.png")]);
    }

    #[test]
    fn rejection_names_node_and_reason() {
        let answer = json!({
            "error": {"type": "prompt_outputs_failed_validation", "message": "Prompt outputs failed validation", "details": ""},
            "node_errors": {"1": {"class_type": "CheckpointLoaderSimple", "errors": [
                {"message": "Value not in list", "details": "ckpt_name: 'nope.safetensors' not in []"}
            ]}}
        });
        let r = rejection(&answer);
        assert!(r.contains("failed validation") && r.contains("1 CheckpointLoaderSimple") && r.contains("nope.safetensors"), "{r}");
    }

    /// Настоящий ComfyUI из `D:\Ollivo` (после `pyenv::tests::install_real`) и SD 1.5
    /// из `D:\Ollivo\models\checkpoints`: картинка 512×512.
    /// `cargo test comfy::tests::txt2img_real -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn txt2img_real() {
        let root = Path::new(r"D:\Ollivo");
        let m = crate::manifest::Manifest::bundled();
        let env = crate::pyenv::ready(root, &m, crate::hardware::detect().cuda_build).expect("окружение не поставлено");
        let sup = Supervisor::new();
        let out = root.join("lab").join("images");
        let c = CancellationToken::new();
        let comfy = start(&sup, &env, root, &[root.join("models").join("checkpoints")], &out, &c).await.unwrap();
        println!("запуск: {:.1} с", comfy.started_in.as_secs_f64());
        // Второй прогон — другой seed: одинаковую задачу ComfyUI не считает, а отдаёт из кэша.
        for run in 1..=2u64 {
            let wf = Txt2Img {
                checkpoint: "v1-5-pruned-emaonly-fp16.safetensors",
                prompt: "a cozy wooden cabin in a snowy forest at sunset, detailed, warm light",
                negative: "blurry, low quality",
                width: 512,
                height: 512,
                steps: 20,
                cfg: 7.0,
                seed: rand_suffix() as u64 + run,
                batch: 1,
                sampler: "euler",
                scheduler: "normal",
                prefix: "Ollivo",
            }
            .workflow();
            let started = Instant::now();
            let steps = std::sync::Mutex::new(0);
            let files = comfy
                .run(&wf, &c, &|p| {
                    if p.max == 20 {
                        *steps.lock().unwrap() = p.value;
                    }
                })
                .await
                .unwrap();
            println!("прогон {run}: {:.1} с, шагов {}, {files:?}", started.elapsed().as_secs_f64(), steps.lock().unwrap());
            assert_eq!(files.len(), 1);
            assert!(files[0].is_file());
            assert_eq!(*steps.lock().unwrap(), 20);
        }

        // Ошибка в задаче — понятный текст, а не зависание.
        let bad = Txt2Img { checkpoint: "нет-такой.safetensors", ..Txt2Img {
            checkpoint: "", prompt: "x", negative: "", width: 64, height: 64, steps: 1, cfg: 1.0, seed: 1, batch: 1,
            sampler: "euler", scheduler: "normal", prefix: "x" } }
        .workflow();
        let err = comfy.run(&bad, &c, &|_| {}).await.unwrap_err();
        println!("{err}");
        assert!(matches!(err, Error::Rejected(_)));
        comfy.handle.stop().await;
    }
}
