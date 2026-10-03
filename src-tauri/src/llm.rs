//! Текстовая модель через llama-server: запуск, готовность, прогрев, вопрос.
//! Или чужой OpenAI-совместимый сервер (`connect`): Ollivo тогда — только окно к нему.
//!
//! llama-server слушает только 127.0.0.1, веб-интерфейс выключен, и всегда с ключом API:
//! без него к модели мог бы обратиться любой процесс на этом ПК (отзыв, 2026-10-03).
//! Ключ — случайный на каждый запуск; открыли модель для других программ — постоянный,
//! на постоянном порту (`Config::key`, `Config::port`).
//! После загрузки модели делаем прогревочный запрос: у Vulkan первые 1–2 ответа
//! медленные из-за компиляции шейдеров (замер фазы 0: ~1,4 с до первого токена).

use crate::engines::Installed;
use crate::presets::{Role, Style};
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
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub model: PathBuf,
    /// Память разговора в токенах: сколько модель помнит.
    pub ctx: u32,
    /// Сколько слоёв считает видеокарта; 0 — всё на процессоре.
    pub gpu_layers: u32,
    /// Дополнение «зрение» (`vision::find_projector`); без него модель картинок не видит.
    pub mmproj: Option<PathBuf>,
    /// Ключ API; пусто — случайный на этот запуск.
    pub key: String,
    /// Постоянный порт (модель открыта для других программ); `None` — любой свободный.
    pub port: Option<u16>,
}

/// Куда слать запросы модели: свой llama-server или чужой сервер.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Адрес OpenAI-совместимого API без «/» в конце: `http://127.0.0.1:5000/v1`.
    pub api: String,
    /// Ключ — заголовком `Authorization: Bearer`; пусто — без ключа.
    pub key: String,
    /// Имя модели в запросе. llama-server его не смотрит, чужому серверу оно обязательно.
    pub model: String,
    /// Корень своего llama-server — для его собственных адресов (`/tokenize`, `/props`)
    /// и полей запроса, которых чужой сервер не знает. У чужого — `None`.
    pub llama: Option<String>,
}

impl Endpoint {
    fn local(port: u16, key: &str) -> Self {
        let root = format!("http://127.0.0.1:{port}");
        Endpoint { api: format!("{root}/v1"), key: key.into(), model: "ollivo".into(), llama: Some(root) }
    }

    fn post(&self, client: &reqwest::Client, url: String) -> reqwest::RequestBuilder {
        self.auth(client.post(url).header("content-type", "application/json"))
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.key.is_empty() { req } else { req.bearer_auth(&self.key) }
    }
}

/// Случайный ключ: 32 байта из генератора Windows, в hex.
pub fn new_key() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("генератор случайных чисел");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub struct Llm {
    /// Процесс llama-server; у чужого сервера — `None`.
    pub handle: Option<Handle>,
    pub port: u16,
    pub endpoint: Endpoint,
    /// Чужой сервер: его адрес — для окна.
    pub remote: Option<String>,
    pub model: PathBuf,
    /// С чем запустили: окно показывает это в карточке модели.
    pub ctx: u32,
    pub gpu_layers: u32,
    pub started_in: Duration,
    /// Видит картинки — так ответил сам движок (`/props`), а не наша догадка по файлам.
    pub vision: bool,
    /// Умеет вызывать инструменты — сама открывает файлы папки проекта. Тоже со слов
    /// движка: он знает, что поддерживает шаблон разговора модели.
    pub tools: bool,
    /// Сколько видеопамяти заняла модель — замер NVML до и после запуска, 0 — неизвестно.
    /// Без него любая другая модель, пока эта запущена, выглядит «не влезет».
    pub vram: u64,
    /// Ступень «экономнее», с которой запустили: проснётся после простоя с ней же.
    pub lighter: u8,
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

fn args(cfg: &Config, port: u16, key: &str) -> Vec<String> {
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
        "--api-key".into(),
        key.into(),
    ];
    // Сколько слоёв уйдёт на видеокарту, посчитано заранее по свободной памяти.
    a.extend(["-ngl".into(), cfg.gpu_layers.to_string()]);
    if let Some(mm) = &cfg.mmproj {
        a.extend(["--mmproj".into(), mm.display().to_string()]);
        // Qwen-VL с меньшим числом токенов на картинку ошибается: Qwen3.5 2B читала «42»
        // как «4» при 194 токенах и верно — при 1024 (docs/phase-3.md). Модели
        // с постоянным числом токенов на картинку (Gemma) флаг не трогает.
        a.extend(["--image-min-tokens".into(), "1024".into()]);
    }
    a
}

/// Ошибка `start`, когда загрузку отменили (`cancel`): движок уже остановлен.
pub const CANCELLED: &str = "отменено"; // служебное слово, человеку не показывается

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
        return Err(tf!("файл модели не найден: {}", "model file not found: {}", cfg.model.display()));
    }
    crate::vcrt::prepare(&engine.exe)?;
    let port = match cfg.port {
        // Постоянный порт занят другой программой — сказать сразу, а не ждать, пока движок упадёт.
        Some(p) => {
            std::net::TcpListener::bind(("127.0.0.1", p)).map_err(|_| tf!("порт {p} занят", "port {p} is busy"))?;
            p
        }
        None => process::free_port().map_err(|e| e.to_string())?,
    };
    let key = if cfg.key.is_empty() { new_key() } else { cfg.key.clone() };
    let endpoint = Endpoint::local(port, &key);
    let spec = process::Spec {
        exe: engine.exe.clone(),
        args: args(cfg, port, &key),
        log: logs.join("llama-server.log"),
    };
    let started = Instant::now();
    let handle = sup.spawn(&spec).map_err(|e| tf!("движок не запустился: {e}", "the engine did not start: {e}"))?;
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
        _ = ask(&endpoint, "Hi", 1) => {}
        _ = cancel.cancelled() => {
            handle.stop().await;
            return Err(CANCELLED.to_string());
        }
    }
    let (vision, tools) = abilities(&endpoint).await;
    Ok(Llm {
        handle: Some(handle),
        port,
        endpoint,
        remote: None,
        model: cfg.model.clone(),
        ctx: cfg.ctx,
        gpu_layers: cfg.gpu_layers,
        started_in: started.elapsed(),
        vision,
        tools,
        vram: 0,
        lighter: 0,
    })
}

/// Спрашивает у движка, подключилось ли зрение (дополнение могло не подойти модели)
/// и умеет ли шаблон модели вызывать инструменты.
async fn abilities(ep: &Endpoint) -> (bool, bool) {
    let props = async {
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().ok()?;
        let resp = ep.auth(client.get(format!("{}/props", ep.llama.as_ref()?))).send().await.ok()?;
        serde_json::from_slice::<serde_json::Value>(&resp.bytes().await.ok()?).ok()
    };
    let Some(v) = props.await else { return (false, false) };
    (v["modalities"]["vision"] == true, v["chat_template_caps"]["supports_tool_calls"] == true)
}

impl Llm {
    /// Останавливает свой движок; чужой сервер просто забываем.
    pub async fn stop(&self) {
        if let Some(h) = &self.handle {
            h.stop().await;
        }
    }
}

/// Сколько памяти разговора считать у чужого сервера: свою он не сообщает. От неё зависят
/// только кусок файла, который модель читает за раз, и предел длины ответа с файлами.
const REMOTE_CTX: u32 = 16384;

/// Адрес чужого сервера, как его пишут в настройках программ, — к адресу API.
/// Без пути — добавляем `/v1`: так пишут адрес Ollama и LM Studio («http://host:11434»).
/// Вставили адрес целиком, с `/chat/completions`, — отрезаем.
pub fn api_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    let mut url = url::Url::parse(&with_scheme)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        .ok_or_else(|| t!("Адрес сервера непонятен. Пример: http://192.168.1.5:11434/v1", "The server address isn't clear. Example: http://192.168.1.5:11434/v1").to_string())?;
    let path = url.path().trim_end_matches('/').trim_end_matches("/chat/completions").trim_end_matches("/models").to_string();
    url.set_path(if path.is_empty() { "/v1" } else { &path });
    url.set_query(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Модели на чужом сервере (`GET /models`). Ошибки — сразу человеческими словами:
/// это то, что человек увидит под кнопкой «Проверить».
pub async fn remote_models(api: &str, key: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let ep = Endpoint { api: api.into(), key: key.into(), model: String::new(), llama: None };
    let resp = ep.auth(client.get(format!("{api}/models"))).send().await.map_err(|e| {
        tf!(
            "Сервер не отвечает. Проверьте адрес и что сервер запущен. ({e})",
            "The server doesn't respond. Check the address and that the server is running. ({e})"
        )
    })?;
    match resp.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err(t!("Сервер не принял ключ.", "The server didn't accept the key.").into()),
        404 => {
            return Err(t!(
                "По этому адресу нет нужного API. Проверьте адрес — обычно он заканчивается на /v1.",
                "There is no suitable API at this address. Check it — it usually ends with /v1."
            )
            .into())
        }
        code => return Err(tf!("Сервер ответил ошибкой {code}.", "The server returned error {code}.")),
    }
    let v: serde_json::Value = serde_json::from_slice(&resp.bytes().await.map_err(|e| e.to_string())?)
        .map_err(|_| t!("Сервер ответил не так, как отвечают модели. Проверьте адрес.", "The server didn't answer the way model servers do. Check the address.").to_string())?;
    let models: Vec<String> = v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(String::from)).collect();
    if models.is_empty() {
        return Err(t!("На сервере нет ни одной модели.", "There are no models on the server.").into());
    }
    Ok(models)
}

/// Подключается к модели на чужом сервере. Проверяем тем же списком моделей: ключ
/// подошёл и модель на месте — значит, можно разговаривать.
pub async fn connect(api: &str, key: &str, model: &str) -> Result<Llm, String> {
    let started = Instant::now();
    if !remote_models(api, key).await?.iter().any(|m| m == model) {
        return Err(tf!("На сервере нет модели «{model}».", "There is no model “{model}” on the server."));
    }
    let host = url::Url::parse(api).ok().and_then(|u| Some(format!("{}{}", u.host_str()?, u.port().map(|p| format!(":{p}")).unwrap_or_default())));
    Ok(Llm {
        handle: None,
        port: 0,
        endpoint: Endpoint { api: api.into(), key: key.into(), model: model.into(), llama: None },
        remote: Some(host.unwrap_or_else(|| api.into())),
        model: PathBuf::from(model),
        ctx: REMOTE_CTX,
        gpu_layers: 0,
        started_in: started.elapsed(),
        // Что умеет модель, чужой сервер не говорит. Считаем, что умеет: не умеет — сервер
        // откажет, и человек увидит его ответ. Иначе картинку и папку было бы не дать вовсе.
        vision: true,
        tools: true,
        vram: 0,
        lighter: 0,
    })
}

/// Реплика разговора. Роли как у OpenAI: `system`, `user`, `assistant`.
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
pub struct Msg {
    pub role: String,
    pub content: String,
    /// Приложенные документы. Модель их видит перед вопросом (`attach::for_model`),
    /// а в окне и в истории они лежат отдельно — вопрос не тонет в тексте документа.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<crate::attach::Attachment>,
    /// Что модель делала с папкой проекта, пока отвечала: прочитала, нашла, сохранила.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<crate::project::Step>,
}

impl Msg {
    pub fn new(role: &str, content: String) -> Self {
        Msg { role: role.into(), content, files: vec![], steps: vec![] }
    }

    /// Реплика в том виде, в каком её ждёт llama-server. Картинки — отдельными частями
    /// сообщения; модели без зрения (разговор начали с другой) вместо картинки — пометка,
    /// иначе движок отказал бы во всём ответе.
    fn wire(&self, vision: bool) -> serde_json::Value {
        let text = crate::attach::for_model(&self.files, &self.content);
        let images: Vec<_> = self.files.iter().filter(|f| f.kind == "image").collect();
        if images.is_empty() {
            return serde_json::json!({"role": self.role, "content": text});
        }
        let mut notes = String::new();
        let mut parts = vec![];
        for img in images {
            match img.path.as_deref().map(crate::attach::image_data_url) {
                Some(Ok(url)) if vision => parts.push(serde_json::json!({"type": "image_url", "image_url": {"url": url}})),
                Some(Ok(_)) => notes.push_str(&tf!(
                    "[картинка «{}» — эта модель картинок не видит]\n",
                    "[image “{}” — this model can't see images]\n",
                    img.name
                )),
                _ => notes.push_str(&tf!("[картинка «{}» потерялась]\n", "[image “{}” is missing]\n", img.name)),
            }
        }
        parts.insert(0, serde_json::json!({"type": "text", "text": format!("{notes}{text}")}));
        serde_json::json!({"role": self.role, "content": parts})
    }
}

/// Чем закончился ответ: сколько токенов и как быстро.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub tokens: u64,
    /// Токенов в секунду на генерации.
    pub speed: f64,
    /// Время до первого токена, мс.
    pub prompt_ms: f64,
    /// Сколько токенов заняли разговор с вложениями — так видно, сколько весит картинка.
    pub prompt_tokens: u64,
}

//// Что приходит в окно по ходу ответа.
pub enum Event<'a> {
    /// Рассуждения думающей модели перед ответом.
    Thought(&'a str),
    /// Кусок ответа.
    Text(&'a str),
    /// Модель начала готовить вызов инструмента — его имя. Файл целиком модель
    /// пишет долго, и без этого окно молчало бы, пока она его «печатает».
    Calling(&'a str),
    /// Инструмент выполнен.
    Step(&'a crate::project::Step),
}

/// Сколько раз за ответ модель может обратиться к файлам. Маленькие модели бывает
/// читают один и тот же файл по кругу; после предела — отвечать без инструментов.
const MAX_ROUNDS: usize = 12;

/// Папка проекта для ответа: инструменты и умеет ли модель их вызывать сама.
pub struct Project<'a> {
    pub tools: &'a crate::project::Tools,
    pub can_call: bool,
}

/// Ответ по кускам: каждый кусок уходит в `on` сразу, как пришёл.
/// `cancel` — кнопка «Остановить»: обрываем соединение, движок прекращает считать.
/// С папкой проекта модель может по ходу ответа читать и записывать файлы —
/// тогда это несколько запросов подряд: модель → инструмент → снова модель.
#[allow(clippy::too_many_arguments)]
pub async fn chat(
    ep: &Endpoint,
    vision: bool,
    messages: &[Msg],
    role: &Role,
    style: &Style,
    project: Option<Project<'_>>,
    cancel: &CancellationToken,
    on: impl Fn(Event),
) -> Result<Stats, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        // Ограничения по времени нет: длинный ответ на медленном ПК идёт долго,
        // а обрыв — дело кнопки «Остановить».
        .build()
        .map_err(|e| e.to_string())?;
    // Промпт роли в историю разговора не пишется: сменили роль — следующий ответ
    // идёт уже по новой.
    let mut all = crate::presets::prepare(role, messages);
    // Переводчику папка не нужна: он переводит только последнее сообщение.
    let project = project.filter(|_| !role.prompt().contains("{target}"));
    if let Some(p) = &project {
        let note = p.tools.prompt(p.can_call);
        match all.first_mut() {
            Some(m) if m.role == "system" => m.content = format!("{}\n\n{note}", m.content),
            _ => all.insert(0, Msg::new("system", note)),
        }
    }
    crate::project::carry_steps(&mut all);
    let mut wire: Vec<serde_json::Value> = all.iter().map(|m| m.wire(vision)).collect();
    let tools = project.as_ref().filter(|p| p.can_call).map(|p| p.tools);
    let mut stats = Stats::default();
    // Всё, что ответ уже написал в окно, — чтобы склеить куски из разных запросов.
    let mut said = String::new();
    for round in 0..=MAX_ROUNDS {
        let mut body = serde_json::json!({
            "model": ep.model,
            "messages": wire,
            "temperature": style.temperature,
            "top_p": style.top_p,
            "stream": true,
            // Просим итоговые числа в последнем куске.
            "stream_options": {"include_usage": true},
        });
        // Поля llama-server. Чужой сервер может отказать во всём запросе из-за незнакомого
        // поля (так делает API OpenAI) — ему их не шлём.
        let llama = ep.llama.is_some();
        if llama {
            body["timings_per_token"] = true.into();
        }
        if role.no_cjk && llama {
            body["grammar"] = crate::presets::NO_CJK.into();
        }
        if let Some(t) = tools {
            body["tools"] = t.specs();
            // Маленькая модель может зациклиться внутри вызова: Qwen2.5 3B однажды
            // «писала файл» 1800 токенов и дальше. Половины памяти хватит на честный файл.
            body["max_tokens"] = (t.ctx() / 2).into();
            if round == MAX_ROUNDS {
                body["tool_choice"] = "none".into();
            }
        }
        // После ответа инструмента — без рассуждений. Qwen3.5 2B писала ответ («скидка 15%»)
        // внутри рассуждений и не закрывала их — ответ выходил пустым. И так быстрее: на тех же
        // четырёх вопросах 3,0 / 1,7 / 3,1 / 4,7 с против 7,8 / 2,4 / 3,9 / 7,8 с с рассуждениями,
        // ответы те же (docs/phase-3.md). Что открыть, модель успевает обдумать в первом запросе.
        // Модели без такого ключа его не заметят.
        if round > 0 && llama {
            body["chat_template_kwargs"] = serde_json::json!({"enable_thinking": false});
        }
        let mut r = round_trip(&client, ep, &body, cancel, &on, &mut said).await?;
        // Первый запрос идёт с рассуждениями. Закончила ни с чем — ни ответа, ни вызова, —
        // тот же запрос ещё раз без них: так ответ, спрятанный в рассуждениях, выйдет наружу.
        if round == 0 && llama && r.text.trim().is_empty() && r.calls.is_empty() && !cancel.is_cancelled() {
            body["chat_template_kwargs"] = serde_json::json!({"enable_thinking": false});
            let again = round_trip(&client, ep, &body, cancel, &on, &mut said).await?;
            r.stats.tokens += again.stats.tokens;
            r = Round { stats: Stats { tokens: r.stats.tokens, ..again.stats }, ..again };
        }
        // Скорость в окне — слова ответа на токены. Токены вызова (текст файла внутри)
        // в словах ответа не видны, и без этой поправки скорость выходила в 20 раз меньше.
        let args: usize = r.calls.iter().map(|c| c.arguments.len() + c.name.len()).sum();
        stats.tokens += r.stats.tokens * r.text.len() as u64 / (r.text.len() + args).max(1) as u64;
        stats.speed = r.stats.speed;
        stats.prompt_ms = r.stats.prompt_ms;
        stats.prompt_tokens = r.stats.prompt_tokens;
        let Some(tools) = tools else { break };
        if r.calls.is_empty() || cancel.is_cancelled() {
            break;
        }
        let calls: Vec<_> = r
            .calls
            .iter()
            .map(|c| {
                // Кривые аргументы движок не примет обратно в истории — отдаём пустые,
                // а модель узнает об ошибке из ответа инструмента.
                let args = if serde_json::from_str::<serde_json::Value>(&c.arguments).is_ok() {
                    c.arguments.as_str()
                } else {
                    "{}"
                };
                serde_json::json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": args}})
            })
            .collect();
        wire.push(serde_json::json!({"role": "assistant", "content": r.text, "tool_calls": calls}));
        for c in &r.calls {
            let (result, step) = tokio::select! {
                x = tools.call(&c.name, &c.arguments) => x,
                _ = cancel.cancelled() => return Ok(stats),
            };
            on(Event::Step(&step));
            wire.push(serde_json::json!({"role": "tool", "tool_call_id": c.id, "content": result}));
        }
    }
    Ok(stats)
}

/// Вызов инструмента, собранный из кусков стрима.
#[derive(Debug, Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
}

struct Round {
    text: String,
    calls: Vec<Call>,
    stats: Stats,
    /// Когда пришёл первый кусок — скорость для сервера, который её не сообщает.
    first: Option<Instant>,
}

impl Round {
    /// Чужой сервер чисел о скорости не шлёт — считаем сами: токены на время от первого куска.
    fn finish(mut self) -> Self {
        if let (0.0, Some(t)) = (self.stats.speed, self.first) {
            let secs = t.elapsed().as_secs_f64();
            if secs > 0.2 {
                self.stats.speed = self.stats.tokens as f64 / secs;
            }
        }
        self
    }
}

/// Один запрос к движку со стримингом. `said` — что ответ уже написал в прошлых
/// запросах: новый кусок отделяем пустой строкой, иначе «Посмотрю файл.Готово» слипнется.
async fn round_trip(
    client: &reqwest::Client,
    ep: &Endpoint,
    body: &serde_json::Value,
    cancel: &CancellationToken,
    on: &impl Fn(Event),
    said: &mut String,
) -> Result<Round, String> {
    let resp = ep
        .post(client, format!("{}/chat/completions", ep.api))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| tf!("движок не отвечает: {e}", "the engine is not responding: {e}"))?;
    if !resp.status().is_success() {
        // В теле — причина: например, `exceed_context_size_error`, когда разговор
        // перерос память модели. По ней `trouble::chat` подбирает понятный текст.
        let code = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(tf!(
            "движок ответил ошибкой {code}: {}",
            "the engine returned error {code}: {}",
            body.chars().take(500).collect::<String>()
        ));
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut round = Round { text: String::new(), calls: vec![], stats: Stats::default(), first: None };
    let mut fresh = true;
    loop {
        let chunk = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => return Ok(round.finish()), // ответ обрываем, что успели — уже показано
        };
        let Some(chunk) = chunk else { break };
        round.first.get_or_insert_with(Instant::now);
        let chunk = chunk.map_err(|e| tf!("связь с движком оборвалась: {e}", "connection to the engine was lost: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        // Server-sent events: события разделены пустой строкой, данные — в строках `data: `.
        while let Some(end) = buf.find("\n\n") {
            let event: String = buf.drain(..end + 2).collect();
            for line in event.lines() {
                let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
                if data == "[DONE]" {
                    return Ok(round.finish());
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
                if let Some(err) = v["error"]["message"].as_str() {
                    return Err(err.to_string());
                }
                let delta = &v["choices"][0]["delta"];
                // Думающие модели (Qwen3.5 и др.) сначала рассуждают — llama-server отдаёт это
                // отдельно от ответа. Без этого окно минуту показывало бы пустое «…».
                if let Some(text) = delta["reasoning_content"].as_str().filter(|t| !t.is_empty()) {
                    on(Event::Thought(text));
                }
                if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
                    if fresh && !said.is_empty() {
                        // Прошлый кусок мог оборваться внутри блока кода (модель начала
                        // «```json» и ушла в вызов) — закрываем, иначе весь ответ дальше станет кодом.
                        let gap = if said.matches("```").count() % 2 == 1 { "\n```\n\n" } else { "\n\n" };
                        said.push_str(gap);
                        on(Event::Text(gap));
                    }
                    fresh = false;
                    said.push_str(text);
                    round.text.push_str(text);
                    on(Event::Text(text));
                }
                // Вызов инструмента приходит кусками: имя сразу, аргументы — по частям.
                for part in delta["tool_calls"].as_array().into_iter().flatten() {
                    let i = part["index"].as_u64().unwrap_or(0) as usize;
                    if round.calls.len() <= i {
                        round.calls.resize_with(i + 1, Call::default);
                    }
                    let call = &mut round.calls[i];
                    if let Some(id) = part["id"].as_str() {
                        call.id = id.to_string();
                    }
                    if let Some(name) = part["function"]["name"].as_str().filter(|n| !n.is_empty()) {
                        call.name.push_str(name);
                        on(Event::Calling(&call.name));
                    }
                    if let Some(args) = part["function"]["arguments"].as_str() {
                        call.arguments.push_str(args);
                    }
                }
                if let Some(t) = v.get("timings").filter(|t| !t.is_null()) {
                    round.stats.speed = t["predicted_per_second"].as_f64().unwrap_or(round.stats.speed);
                    round.stats.prompt_ms = t["prompt_ms"].as_f64().unwrap_or(round.stats.prompt_ms);
                    round.stats.tokens = t["predicted_n"].as_u64().unwrap_or(round.stats.tokens);
                }
                if let Some(n) = v["usage"]["completion_tokens"].as_u64() {
                    round.stats.tokens = n;
                }
                if let Some(n) = v["usage"]["prompt_tokens"].as_u64() {
                    round.stats.prompt_tokens = n;
                }
            }
        }
    }
    Ok(round.finish())
}

// Сколько токенов займёт текст у этой модели. `None` — движок не ответил,
/// тогда обходимся прикидкой.
pub async fn count_tokens(ep: &Endpoint, text: &str) -> Option<u64> {
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(30)).build().ok()?;
    let resp = ep
        .post(&client, format!("{}/tokenize", ep.llama.as_ref()?))
        .body(serde_json::json!({"content": text}).to_string())
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&resp.bytes().await.ok()?).ok()?;
    v["tokens"].as_array().map(|t| t.len() as u64)
}

/// Вопрос без стриминга — для проверки движка.
pub async fn ask(ep: &Endpoint, prompt: &str, max_tokens: u32) -> Result<Answer, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let body = serde_json::json!({
        "model": ep.model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
    });
    let resp = ep
        .post(&client, format!("{}/chat/completions", ep.api))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| tf!("движок не отвечает: {e}", "the engine is not responding: {e}"))?;
    if !resp.status().is_success() {
        return Err(tf!("движок ответил ошибкой {}", "the engine returned error {}", resp.status().as_u16()));
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
    use crate::presets;

    #[test]
    fn remote_address() {
        assert_eq!(api_url("192.168.1.5:11434").unwrap(), "http://192.168.1.5:11434/v1");
        assert_eq!(api_url(" http://host:8080/v1/ ").unwrap(), "http://host:8080/v1");
        assert_eq!(api_url("https://api.example.com/v1/chat/completions").unwrap(), "https://api.example.com/v1");
        assert_eq!(api_url("http://host/openai/v1").unwrap(), "http://host/openai/v1");
        assert!(api_url("ftp://host").is_err() && api_url("").is_err());
    }

    /// Список моделей чужого сервера: ключ уходит заголовком, неверный ключ — понятными словами.
    #[tokio::test]
    async fn remote_models_and_key() {
        let srv = crate::testserver::serve(br#"{"data":[{"id":"qwen2.5:7b"},{"id":"llama3"}]}"#.to_vec(), false, 0);
        let api = srv.url.trim_end_matches("/file.bin").to_string() + "/v1";
        assert_eq!(remote_models(&api, "s3cret").await.unwrap(), ["qwen2.5:7b", "llama3"]);
        assert_eq!(srv.auth.lock().unwrap().last().unwrap(), "Bearer s3cret");
        let l = connect(&api, "", "llama3").await.unwrap();
        assert!(l.handle.is_none() && l.remote.is_some() && l.endpoint.llama.is_none());
        assert!(connect(&api, "", "нет такой").await.err().unwrap().contains("нет модели"));
        assert!(remote_models("http://127.0.0.1:1/v1", "").await.unwrap_err().contains("не отвечает"));
    }

    #[test]
    fn args_bind_localhost_without_webui() {
        let cfg = Config { model: PathBuf::from(r"D:\Ollivo\models\m.gguf"), ctx: 8192, gpu_layers: 21, mmproj: None, ..Default::default() };
        let a = args(&cfg, 5000, "k1").join(" ");
        assert!(a.contains("--host 127.0.0.1 --port 5000"));
        assert!(a.contains("--api-key k1"), "без ключа к модели обратится любой процесс");
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
        let cfg = Config { model: root.join(r"models\qwen2.5-0.5b-instruct-q4_k_m.gguf"), ctx: 4096, gpu_layers: 999, mmproj: None, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &crate::testserver::tmp().join("ollivo-llm-test"), &CancellationToken::new())
            .await
            .unwrap();
        println!("готов за {:.1} с, порт {}", llm.started_in.as_secs_f64(), llm.port);
        let a = ask(&llm.endpoint, "Столица Франции? Одно слово.", 16).await.unwrap();
        println!("{a:?}");
        assert!(!a.text.is_empty() && a.speed > 0.0);
        // Без ключа движок не отвечает: другой процесс на этом ПК модель не получит.
        let stranger = Endpoint { key: String::new(), ..llm.endpoint.clone() };
        let refused = ask(&stranger, "Hi", 1).await.unwrap_err();
        assert!(refused.contains("401"), "{refused}");
        let e = llm.handle.as_ref().unwrap().stop().await;
        assert!(e.by_us);
    }

    /// Стриминг на настоящем движке: куски приходят по ходу, «Остановить» обрывает ответ.
    /// ПК, где имя пользователя «Иван Петров» и VC++ никогда не ставили: движок и модель
    /// в папках с кириллицей и пробелами, библиотеки VC++ — только наши копии рядом с движком.
    /// `cargo test llm::tests::real_human_paths -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_human_paths() {
        let root = PathBuf::from(r"D:\Ollivo");
        let src = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let base = crate::testserver::human_dir("чат");
        let engine_dir = base.join("движок чата");
        crate::testserver::link_tree(&src.dir, &engine_dir);
        // Системы без VC++ нет под рукой — кладём копии как для неё (пустая «System32»), а метку
        // убираем, иначе `start` увидит новый VC++ в настоящей System32 и уберёт копии обратно.
        let no_system = base.join("пустая System32");
        std::fs::create_dir_all(&no_system).unwrap();
        crate::vcrt::ensure_in(&engine_dir, &no_system).unwrap();
        std::fs::remove_file(engine_dir.join(crate::vcrt::MARKER)).unwrap();

        let model = base.join("мои модели").join("Qwen маленькая.gguf");
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::hard_link(root.join(r"models\qwen2.5-0.5b-instruct-q4_k_m.gguf"), &model).unwrap();

        let engine = Installed { exe: engine_dir.join(src.exe.strip_prefix(&src.dir).unwrap()), dir: engine_dir.clone(), ..src };
        let cfg = Config { model, ctx: 2048, gpu_layers: 999, mmproj: None, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &base.join("журналы"), &CancellationToken::new()).await.unwrap();
        for dll in ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"] {
            let from = crate::testserver::loaded_from(llm.handle.as_ref().unwrap().pid, dll);
            println!("{dll}: {from}");
            assert!(from.contains("Иван Петров") && from.contains("движок чата"), "{dll} загружена из {from:?}");
        }
        let msgs = vec![Msg::new("user", "Напиши числа от 1 до 5 через запятую.".into())];
        let text = std::sync::Mutex::new(String::new());
        chat(&llm.endpoint, false, &msgs, presets::role(""), presets::style(""), None, &CancellationToken::new(), |e| {
            if let Event::Text(t) = e {
                text.lock().unwrap().push_str(t)
            }
        })
        .await
        .unwrap();
        let text = text.into_inner().unwrap();
        println!("ответ: {text}");
        assert!(text.contains('3'), "{text}");
        llm.stop().await;
    }

    /// `cargo test llm::tests::real_chat_stream -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_chat_stream() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let cfg = Config { model: root.join(r"models\qwen2.5-0.5b-instruct-q4_k_m.gguf"), ctx: 4096, gpu_layers: 999, mmproj: None, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &crate::testserver::tmp().join("ollivo-chat-test"), &CancellationToken::new())
            .await
            .unwrap();

        let msgs = vec![Msg::new("user", "Напиши числа от 1 до 20 словами, через запятую.".into())];
        let chunks = std::sync::Mutex::new(Vec::<String>::new());
        let stats = chat(&llm.endpoint, false, &msgs, presets::role(""), presets::style(""), None, &CancellationToken::new(), |e| if let Event::Text(t) = e {
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
        let long = vec![Msg::new("user", "Напиши рассказ на 2000 слов.".into())];
        chat(&llm.endpoint, false, &long, presets::role(""), presets::style(""), None, &cancel, |_| {}).await.unwrap();
        println!("остановлено за {:.1} с", t.elapsed().as_secs_f64());
        assert!(t.elapsed() < Duration::from_secs(5));
        llm.stop().await;
    }

    /// Разговор длиннее памяти модели: движок отвечает 400 с причиной в теле,
    /// и `trouble::chat` предлагает новый разговор.
    #[tokio::test]
    #[ignore]
    async fn real_context_overflow() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let cfg = Config { model: root.join(r"models\qwen2.5-0.5b-instruct-q4_k_m.gguf"), ctx: 512, gpu_layers: 999, mmproj: None, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &crate::testserver::tmp().join("ollivo-ctx-test"), &CancellationToken::new())
            .await
            .unwrap();
        let long = vec![Msg::new("user", "слово ".repeat(2000))];
        let err = chat(&llm.endpoint, false, &long, presets::role(""), presets::style(""), None, &CancellationToken::new(), |_| {})
            .await
            .err()
            .expect("должно не влезть");
        let p = crate::trouble::chat(&err);
        println!("{err}
→ {}", p.text);
        assert_eq!(p.actions, [crate::trouble::Action::NewChat]);
        llm.stop().await;
    }

    /// Роли на настоящей модели: переводчик переводит в обе стороны и не болтает.
    /// `cargo test llm::tests::real_roles -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_roles() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        // OLLIVO_MODEL — имя файла в D:\Ollivo\models, чтобы проверить и маленькую модель.
        let file = std::env::var("OLLIVO_MODEL").unwrap_or("qwen2.5-3b-instruct-q4_k_m.gguf".into());
        let cfg = Config { model: root.join("models").join(file), ctx: 4096, gpu_layers: 999, mmproj: None, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &crate::testserver::tmp().join("ollivo-roles-test"), &CancellationToken::new())
            .await
            .unwrap();
        let ask = |role: &'static str, style: &'static str, text: &'static str| {
            let port = llm.endpoint.clone();
            async move {
                let out = std::sync::Mutex::new(String::new());
                let msgs = vec![Msg::new("user", text.into())];
                chat(&port, false, &msgs, presets::role(role), presets::style(style), None, &CancellationToken::new(), |e| if let Event::Text(t) = e {
                    out.lock().unwrap().push_str(t)
                })
                .await
                .unwrap();
                let out = out.into_inner().unwrap();
                println!("[{role}/{style}] {text}
→ {out}
");
                out
            }
        };
        let cyrillic = |s: &str| s.chars().any(|c| ('а'..='я').contains(&c.to_lowercase().next().unwrap()));

        let en = ask("translator", "precise", "Сегодня хорошая погода, пойдём гулять в парк.").await;
        assert!(!cyrillic(&en), "с русского — на английский");
        let ru = ask("translator", "precise", "The cat is sleeping on the sofa.").await;
        assert!(cyrillic(&ru), "с английского — на русский");
        // В разговоре: прошлая пара «русский → английский» не должна сбить направление
        // (так 0.5B повторяла английский как есть, найдено в окне).
        let talk = vec![
            Msg::new("user", "Доброе утро! Как спалось?".into()),
            Msg::new("assistant", "Good morning! How did you sleep?".into()),
            Msg::new("user", "The weather is nice today, let's go for a walk.".into()),
        ];
        let out = std::sync::Mutex::new(String::new());
        chat(&llm.endpoint, false, &talk, presets::role("translator"), presets::style("precise"), None, &CancellationToken::new(), |e| if let Event::Text(t) = e {
            out.lock().unwrap().push_str(t)
        })
        .await
        .unwrap();
        let out = out.into_inner().unwrap();
        println!("[в разговоре] → {out}
");
        assert!(cyrillic(&out), "в разговоре — тоже на русский");
        ask("coder", "precise", "Как на Python прочитать файл построчно?").await;
        for style in ["precise", "creative"] {
            ask("helper", style, "Придумай название для кофейни.").await;
        }
        llm.stop().await;
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
        let cfg = Config { model: PathBuf::from(r"Z:\нет.gguf"), ctx: 4096, gpu_layers: 999, mmproj: None, ..Default::default() };
        let err = start(&Supervisor::new(), &engine, &cfg, &crate::testserver::tmp(), &CancellationToken::new())
            .await
            .err()
            .unwrap();
        assert!(err.contains("не найден"));
    }

    /// Отмена во время загрузки: не ждём таймаута, процесс остановлен.
    /// Вместо llama-server — ping, который никогда не ответит на /health.
    #[tokio::test]
    async fn cancel_during_load_stops_engine() {
        let dir = crate::testserver::tmp().join(format!("ollivo-llm-cancel-{}", std::process::id()));
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
        let cfg = Config { model, ctx: 4096, gpu_layers: 999, mmproj: None, ..Default::default() };
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

    /// Документ на настоящей модели: `/tokenize` считает, модель отвечает по тексту PDF.
    /// `cargo test llm::tests::real_document -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_document() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let cfg = Config { model: root.join(r"models\qwen2.5-3b-instruct-q4_k_m.gguf"), ctx: 4096, gpu_layers: 999, mmproj: None, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &crate::testserver::tmp().join("ollivo-doc-test"), &CancellationToken::new())
            .await
            .unwrap();
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/borsch.pdf");
        let text = crate::attach::pdf_text_here(&pdf).unwrap();
        let exact = count_tokens(&llm.endpoint, &text).await.unwrap();
        let guess = crate::attach::estimate_tokens(&text);
        println!("токенов: {exact}, прикидка по буквам {guess}");
        assert!(exact > 500 && guess >= exact * 9 / 10, "прикидка не должна сильно занижать");

        let mut q = Msg::new("user", "Сколько штук картофеля нужно по таблице? Ответь одним числом.".into());
        q.files.push(crate::attach::Attachment {
            name: "borsch.pdf".into(),
            kind: "document".into(),
            tokens: exact,
            text,
            trimmed: false,
            path: None,
        });
        let out = std::sync::Mutex::new(String::new());
        chat(&llm.endpoint, false, &[q], presets::role("helper"), presets::style("precise"), None, &CancellationToken::new(), |e| if let Event::Text(t) = e {
            out.lock().unwrap().push_str(t)
        })
        .await
        .unwrap();
        let out = out.into_inner().unwrap();
        println!("→ {out}");
        assert!(out.contains('3'), "{out}");
        llm.stop().await;
    }

    /// Зрение на настоящей модели из каталога: дополнение находится само, движок
    /// подтверждает зрение, модель читает число на картинке.
    /// `cargo test llm::tests::real_vision -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_vision() {
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        let model = root.join(r"models\unsloth\Qwen3.5-2B-GGUF\Qwen3.5-2B-Q4_K_M.gguf");
        let mmproj = crate::vision::find_projector(&model);
        assert!(mmproj.is_some(), "дополнение не нашлось");
        let cfg = Config { model, ctx: 4096, gpu_layers: 999, mmproj, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &crate::testserver::tmp().join("ollivo-vision-test"), &CancellationToken::new())
            .await
            .unwrap();
        println!("готов за {:.1} с, зрение: {}", llm.started_in.as_secs_f64(), llm.vision);
        assert!(llm.vision);

        let images = crate::testserver::tmp().join("ollivo-vision-test").join("images");
        let pic = crate::attach::read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/circle42.png"), &images, None).unwrap();
        let ask = |question: &'static str, files: Vec<crate::attach::Attachment>| {
            let port = llm.endpoint.clone();
            async move {
                let mut q = Msg::new("user", question.into());
                q.files = files;
                let out = std::sync::Mutex::new(String::new());
                let stats = chat(&port, true, &[q], presets::role("helper"), presets::style("precise"), None, &CancellationToken::new(), |e| if let Event::Text(t) = e {
                    out.lock().unwrap().push_str(t)
                })
                .await
                .unwrap();
                let out = out.into_inner().unwrap();
                println!("{question} → {out}\n  токенов вопроса: {}", stats.prompt_tokens);
                (out, stats.prompt_tokens)
            }
        };
        let (plain, base) = ask("Какое число написано на картинке? Ответь только числом.", vec![]).await;
        let (seen, with_image) = ask("Какое число написано на картинке? Ответь только числом.", vec![pic.clone()]).await;
        println!("картинка 512×384 заняла {} токенов", with_image - base);
        assert!(seen.contains("42"), "{seen}");
        assert!(!plain.contains("42"));
        let (color, _) = ask("Какого цвета круг? Одно слово.", vec![pic]).await;
        assert!(color.to_lowercase().contains("красн"), "{color}");
        llm.stop().await;
    }

    /// Папка проекта на настоящей модели: модель сама читает файл, находит текст
    /// и создаёт новый файл — после согласия человека.
    /// `cargo test llm::tests::real_project -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_project() {
        use crate::project::{self, Tools};
        use std::sync::{Arc, Mutex};
        let root = PathBuf::from(r"D:\Ollivo");
        let engine = crate::engines::installed(&root, "llama.cpp").pop().expect("llama.cpp не установлен");
        // OLLIVO_MODEL — путь от D:\Ollivo\models, чтобы сравнить модели.
        let file = std::env::var("OLLIVO_MODEL").unwrap_or(r"unsloth\Qwen3.5-2B-GGUF\Qwen3.5-2B-Q4_K_M.gguf".into());
        let cfg = Config { model: root.join("models").join(file), ctx: 8192, gpu_layers: 999, mmproj: None, ..Default::default() };
        let sup = Supervisor::new();
        let llm = start(&sup, &engine, &cfg, &crate::testserver::tmp().join("ollivo-project-test"), &CancellationToken::new())
            .await
            .unwrap();
        println!("готов за {:.1} с, инструменты: {}", llm.started_in.as_secs_f64(), llm.tools);
        assert!(llm.tools);

        let dir = crate::testserver::tmp().join(format!("ollivo-real-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shop")).unwrap();
        std::fs::write(dir.join("main.py"), "from shop.prices import total\n\nprint(total([120, 80, 45]))\n").unwrap();
        std::fs::write(
            dir.join("shop/prices.py"),
            "DISCOUNT = 0.15\n\n\ndef total(prices):\n    \"\"\"Сумма покупки со скидкой постоянного покупателя.\"\"\"\n    return round(sum(prices) * (1 - DISCOUNT), 2)\n",
        )
        .unwrap();
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let a = asked.clone();
        let ask: project::AskWrite = Arc::new(move |w| {
            println!("--- просит записать {} ({} строк):\n{}", w.path, w.new_lines, w.content);
            a.lock().unwrap().push(w.path);
            Box::pin(async { true })
        });
        let backups = dir.join("../ollivo-real-project-backups");
        // Как в программе: инструменты — заново на каждый ответ (там живёт «уже прочитан»).
        let tools = |mode| Tools::new(project::list(&dir).unwrap(), cfg.ctx, backups.clone(), mode, ask.clone());

        async fn talk(port: &Endpoint, tools: Tools, question: &str) -> (String, Vec<String>) {
            talk_in(port, tools, vec![Msg::new("user", question.into())]).await
        }
        async fn talk_in(port: &Endpoint, tools: Tools, messages: Vec<Msg>) -> (String, Vec<String>) {
            let tools = &tools;
            let question = messages.last().map(|m| m.content.clone()).unwrap_or_default();
            let out = Mutex::new(String::new());
            let steps = Mutex::new(Vec::new());
            let t = Instant::now();
            chat(
                port,
                false,
                &messages,
                presets::role("coder"),
                presets::style("precise"),
                Some(Project { tools, can_call: true }),
                &CancellationToken::new(),
                |e| match e {
                    Event::Text(t) => out.lock().unwrap().push_str(t),
                    Event::Step(s) => steps.lock().unwrap().push(format!("{} {} {}", s.kind, s.path, s.note)),
                    Event::Thought(t) => print!("{t}"),
                    Event::Calling(n) => println!("\n[вызов {n}]"),
                },
            )
            .await
            .unwrap();
            let (out, steps) = (out.into_inner().unwrap(), steps.into_inner().unwrap());
            println!("\n{question}\n  шаги: {steps:?}\n  за {:.1} с\n→ {out}\n", t.elapsed().as_secs_f64());
            (out, steps)
        }
        let port = llm.endpoint.clone();
        let (out, steps) = talk(&port, tools(project::Mode::Ask), "Какая скидка в проекте? Ответь числом в процентах.").await;
        assert!(steps.iter().any(|s| s.starts_with("read") || s.starts_with("search")), "{steps:?}");
        assert!(out.contains("15"), "{out}");

        let (_, steps) = talk(&port, tools(project::Mode::Ask), "Создай файл hello.py, который печатает «Привет». Сразу создай, без вопросов.").await;
        assert!(steps.iter().any(|s| s.starts_with("write")), "{steps:?}");
        let hello = std::fs::read_to_string(dir.join("hello.py")).expect("hello.py не создан");
        assert!(hello.contains("Привет") && hello.contains("print"), "{hello}");

        // Маленькая правка: чем модель её делает — куском или файлом целиком — и не портит ли остальное.
        let (_, steps) = talk(&port, tools(project::Mode::Ask), "В shop/prices.py поменяй скидку на 20%.").await;
        let prices = std::fs::read_to_string(dir.join("shop/prices.py")).unwrap();
        println!("--- shop/prices.py после правки:\n{prices}");
        assert!(steps.iter().any(|s| s.starts_with("edit") || s.starts_with("write")), "{steps:?}");
        assert!(prices.contains("0.2") && prices.contains("def total"), "{prices}");

        // «План»: инструментов записи нет, файлы не меняются.
        let before = project::list(&dir).unwrap().files;
        // «Авто» и история: в прошлом ответе модель создала notes.txt, теперь просим удалить
        // (удаление и в «Авто» идёт через вопрос — тестовый `ask` разрешает).
        // Qwen2.5 3B здесь ответила «удалён», ничего не удалив, — когда справка о прошлых
        // действиях шла в конце её ответа.
        std::fs::write(dir.join("notes.txt"), "hello\n").unwrap();
        let mut done = Msg::new("assistant", "Файл notes.txt создан.".into());
        done.steps.push(project::Step { kind: "write".into(), path: "notes.txt".into(), ok: true, ..Default::default() });
        let history = vec![Msg::new("user", "Создай notes.txt с текстом hello".into()), done, Msg::new("user", "Теперь удали notes.txt".into())];
        let (_, steps) = talk_in(&port, tools(project::Mode::Auto), history).await;
        assert!(steps.iter().any(|s| s.starts_with("delete notes.txt")), "{steps:?}");
        assert!(!dir.join("notes.txt").exists());

        let (out, steps) = talk(&port, tools(project::Mode::Plan), "Добавь скидку по промокоду: промокод SALE даёт ещё 5%.").await;
        assert!(!steps.iter().any(|s| s.starts_with("write") || s.starts_with("edit") || s.starts_with("delete")), "{steps:?}");
        assert_eq!(project::list(&dir).unwrap().files, before);
        assert!(!out.trim().is_empty());
        llm.stop().await;
    }
}
