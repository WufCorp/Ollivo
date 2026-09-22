//! Менеджер загрузок: несколько соединений по HTTP Range, докачка после паузы
//! или обрыва, повторы, запасные зеркала, проверка SHA256.
//!
//! Файл качается в `<dest>.part`; какие куски уже готовы — в `<dest>.part.json`.
//! Пауза — это отмена через `CancellationToken`: оба файла остаются,
//! следующий вызов `download` продолжит с того же места.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const MIB: u64 = 1 << 20;
const RETRIES: u32 = 5;

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Основной адрес и зеркала — пробуются по порядку.
    pub urls: Vec<String>,
    pub dest: PathBuf,
    /// Ожидаемый SHA256 (hex). Если нет — берётся из `X-Linked-ETag` (HuggingFace).
    pub sha256: Option<String>,
    #[serde(default = "default_connections")]
    pub connections: usize,
    #[serde(default = "default_chunk")]
    pub chunk_size: u64,
}

impl Request {
    /// Обычная загрузка: откуда, куда и чем проверить.
    pub fn new(urls: Vec<String>, dest: PathBuf, sha256: Option<String>) -> Self {
        Self { urls, dest, sha256, connections: default_connections(), chunk_size: default_chunk() }
    }
}

fn default_connections() -> usize {
    8
}

fn default_chunk() -> u64 {
    8 * MIB
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Downloading,
    Verifying,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: Option<u64>,
    /// Байт в секунду, сглаженная.
    pub speed: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("загрузка остановлена")]
    Cancelled,
    #[error("файл повреждён: SHA256 {actual}, ожидался {expected}")]
    Hash { expected: String, actual: String },
    #[error("сервер ответил {0}")]
    Status(u16),
    #[error("сеть: {0}")]
    Net(#[from] reqwest::Error),
    #[error("диск: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Что сервер сообщил о файле до загрузки.
#[derive(Debug)]
struct Probe {
    /// Адрес после всех перенаправлений.
    url: String,
    size: Option<u64>,
    ranges: bool,
    sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct State {
    size: u64,
    chunk: u64,
    done: Vec<bool>,
}

pub struct Downloader {
    client: reqwest::Client,
    /// Без перенаправлений: HuggingFace кладёт SHA256 в заголовки ответа 302.
    probe_client: reqwest::Client,
    /// Токен и хосты, которым его можно отправлять (HF и зеркало).
    bearer: Option<(Vec<String>, String)>,
}

impl Downloader {
    pub fn new() -> Self {
        Self::with_proxy(None)
    }

    /// `proxy: None` — напрямую, без системного прокси Windows: программа ходит
    /// в сеть только так, как настроено в Ollivo.
    pub fn with_proxy(proxy: Option<reqwest::Proxy>) -> Self {
        let ua = concat!("Ollivo/", env!("CARGO_PKG_VERSION"));
        // Только HTTP/1.1: по HTTP/2 все потоки шли бы через одно TCP-соединение,
        // и параллельная загрузка теряла бы смысл. Замер на HF: 1 поток — 0,6 МБ/с,
        // 8 потоков — 2,3 МБ/с (по HTTP/2 было те же 0,6).
        let build = |redirects| {
            let b = reqwest::Client::builder()
                .user_agent(ua)
                .http1_only()
                .redirect(redirects)
                .connect_timeout(Duration::from_secs(20))
                .read_timeout(Duration::from_secs(60));
            match proxy.clone() {
                Some(p) => b.proxy(p),
                None => b.no_proxy(),
            }
            .build()
            .expect("HTTP-клиент")
        };
        Self {
            client: build(reqwest::redirect::Policy::limited(10)),
            probe_client: build(reqwest::redirect::Policy::none()),
            bearer: None,
        }
    }

    /// Токен добавляется только к запросам на `hosts`. На CDN, куда HF перенаправляет,
    /// он не уходит: адреса перенаправлений мы проходим сами (`probe`), а reqwest
    /// снимает заголовок при переходе на другой хост.
    pub fn with_bearer(mut self, hosts: Vec<String>, token: String) -> Self {
        self.bearer = (!token.is_empty() && !hosts.is_empty()).then_some((hosts, token));
        self
    }

    /// Клиент для API-запросов (с тем же прокси).
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn get(&self, client: &reqwest::Client, url: reqwest::Url) -> reqwest::RequestBuilder {
        let req = client.get(url.clone());
        match &self.bearer {
            Some((hosts, token)) if url.host_str().is_some_and(|h| hosts.iter().any(|x| x == h)) => {
                req.bearer_auth(token)
            }
            _ => req,
        }
    }

    pub async fn download(
        &self,
        req: &Request,
        cancel: &CancellationToken,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), Error> {
        if req.dest.exists() {
            return Ok(());
        }
        if req.urls.is_empty() {
            return Err(Error::Other("нет адреса для загрузки".into()));
        }
        let mut last = None;
        for url in &req.urls {
            match self.try_url(url, req, cancel, on_progress).await {
                Ok(()) => return Ok(()),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e @ Error::Hash { .. }) => {
                    // Испорченный файл докачивать бессмысленно — с зеркала начинаем заново.
                    remove_partial(&req.dest);
                    last = Some(e);
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap())
    }

    async fn try_url(
        &self,
        url: &str,
        req: &Request,
        cancel: &CancellationToken,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), Error> {
        let probe = tokio::select! {
            p = self.probe(url) => p?,
            _ = cancel.cancelled() => return Err(Error::Cancelled),
        };
        let expected = req.sha256.clone().or(probe.sha256.clone());
        let part = with_suffix(&req.dest, ".part");
        if let Some(dir) = req.dest.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        match probe.size {
            Some(size) if probe.ranges && size > 0 => {
                self.fetch_parallel(&probe.url, size, req, &part, cancel, on_progress).await?
            }
            _ => self.fetch_single(&probe.url, probe.size, &part, cancel, on_progress).await?,
        }

        let total = tokio::fs::metadata(&part).await?.len();
        if let Some(expected) = expected {
            on_progress(Progress { phase: Phase::Verifying, done: total, total: Some(total), speed: 0.0 });
            let p = part.clone();
            let actual = tokio::task::spawn_blocking(move || sha256_file(&p))
                .await
                .map_err(|e| Error::Other(e.to_string()))??;
            if !actual.eq_ignore_ascii_case(&expected) {
                return Err(Error::Hash { expected, actual });
            }
        }
        tokio::fs::rename(&part, &req.dest).await?;
        let _ = tokio::fs::remove_file(with_suffix(&req.dest, ".part.json")).await;
        Ok(())
    }

    /// Запрос первого байта: узнаём размер, поддержку Range и SHA256.
    async fn probe(&self, url: &str) -> Result<Probe, Error> {
        let mut url = reqwest::Url::parse(url).map_err(|e| Error::Other(e.to_string()))?;
        let mut sha256 = None;
        for _ in 0..10 {
            let resp = self
                .get(&self.probe_client, url.clone())
                .header(reqwest::header::RANGE, "bytes=0-0")
                .send()
                .await?;
            let status = resp.status();
            if let Some(etag) = header(&resp, "x-linked-etag") {
                let etag = etag.trim_matches('"').to_string();
                if etag.len() == 64 && etag.bytes().all(|b| b.is_ascii_hexdigit()) {
                    sha256 = Some(etag);
                }
            }
            if status.is_redirection() {
                let loc = header(&resp, "location").ok_or(Error::Status(status.as_u16()))?;
                url = url.join(&loc).map_err(|e| Error::Other(e.to_string()))?;
                continue;
            }
            return match status.as_u16() {
                206 => {
                    let size = header(&resp, "content-range")
                        .and_then(|r| r.rsplit('/').next().and_then(|s| s.parse().ok()));
                    Ok(Probe { url: url.into(), ranges: size.is_some(), size, sha256 })
                }
                200 => Ok(Probe { url: url.into(), size: resp.content_length(), ranges: false, sha256 }),
                s => Err(Error::Status(s)),
            };
        }
        Err(Error::Other("слишком много перенаправлений".into()))
    }

    async fn fetch_parallel(
        &self,
        url: &str,
        size: u64,
        req: &Request,
        part: &Path,
        cancel: &CancellationToken,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), Error> {
        let state_path = with_suffix(&req.dest, ".part.json");
        let chunk = req.chunk_size.max(64 * 1024);
        let state = load_state(&state_path, part, size).unwrap_or_else(|| State {
            size,
            chunk,
            done: vec![false; size.div_ceil(chunk) as usize],
        });
        if !part.exists() || std::fs::metadata(part)?.len() != size {
            let f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(part)?;
            f.set_len(size)?;
        }
        let chunk = state.chunk;
        let chunk_range = move |i: usize| {
            let start = i as u64 * chunk;
            (start, (start + chunk).min(size) - 1)
        };
        let done_bytes: u64 = state
            .done
            .iter()
            .enumerate()
            .filter(|(_, d)| **d)
            .map(|(i, _)| {
                let (s, e) = chunk_range(i);
                e - s + 1
            })
            .sum();
        let queue: VecDeque<usize> =
            state.done.iter().enumerate().filter(|(_, d)| !**d).map(|(i, _)| i).collect();

        let counter = Arc::new(AtomicU64::new(done_bytes));
        let queue = Arc::new(Mutex::new(queue));
        let state = Arc::new(Mutex::new(state));
        let workers = req.connections.clamp(1, 16);

        let jobs = (0..workers).map(|_| {
            let (queue, state, counter) = (queue.clone(), state.clone(), counter.clone());
            let state_path = state_path.clone();
            async move {
                let mut file = tokio::fs::OpenOptions::new().write(true).open(part).await?;
                loop {
                    let Some(i) = queue.lock().unwrap().pop_front() else { return Ok(()) };
                    let (start, end) = chunk_range(i);
                    self.fetch_chunk_retrying(url, start, end, &mut file, &counter, cancel).await?;
                    let json = {
                        let mut st = state.lock().unwrap();
                        st.done[i] = true;
                        serde_json::to_vec(&*st).unwrap()
                    };
                    tokio::fs::write(&state_path, json).await?;
                }
            }
        });
        let all = futures_util::future::try_join_all(jobs);
        report_while(all, &counter, Some(size), on_progress).await?;
        Ok(())
    }

    async fn fetch_chunk_retrying(
        &self,
        url: &str,
        start: u64,
        end: u64,
        file: &mut tokio::fs::File,
        counter: &AtomicU64,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let mut attempt = 0;
        loop {
            let mut written = 0u64;
            let res = self.fetch_chunk(url, start, end, file, counter, &mut written, cancel).await;
            match res {
                Ok(()) => return Ok(()),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => {
                    counter.fetch_sub(written, Ordering::Relaxed);
                    attempt += 1;
                    if attempt >= RETRIES {
                        return Err(e);
                    }
                    let pause = Duration::from_millis(500 << attempt);
                    tokio::select! {
                        _ = tokio::time::sleep(pause) => {}
                        _ = cancel.cancelled() => return Err(Error::Cancelled),
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_chunk(
        &self,
        url: &str,
        start: u64,
        end: u64,
        file: &mut tokio::fs::File,
        counter: &AtomicU64,
        written: &mut u64,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let url = reqwest::Url::parse(url).map_err(|e| Error::Other(e.to_string()))?;
        let resp = self
            .get(&self.client, url)
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            .send()
            .await?;
        if resp.status().as_u16() != 206 {
            return Err(Error::Status(resp.status().as_u16()));
        }
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut stream = resp.bytes_stream();
        loop {
            let next = tokio::select! {
                n = stream.next() => n,
                _ = cancel.cancelled() => return Err(Error::Cancelled),
            };
            let Some(bytes) = next else { break };
            let bytes = bytes?;
            if start + *written + bytes.len() as u64 > end + 1 {
                return Err(Error::Other("сервер прислал лишние байты".into()));
            }
            file.write_all(&bytes).await?;
            *written += bytes.len() as u64;
            counter.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
        if *written != end - start + 1 {
            return Err(Error::Other("кусок оборвался".into()));
        }
        file.flush().await?;
        Ok(())
    }

    /// Сервер без Range: одно соединение, докачки нет.
    async fn fetch_single(
        &self,
        url: &str,
        size: Option<u64>,
        part: &Path,
        cancel: &CancellationToken,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), Error> {
        let counter = AtomicU64::new(0);
        let work = async {
            let url = reqwest::Url::parse(url).map_err(|e| Error::Other(e.to_string()))?;
            let resp = self.get(&self.client, url).send().await?;
            if !resp.status().is_success() {
                return Err(Error::Status(resp.status().as_u16()));
            }
            let mut file = tokio::fs::File::create(part).await?;
            let mut stream = resp.bytes_stream();
            loop {
                let next = tokio::select! {
                    n = stream.next() => n,
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                };
                let Some(bytes) = next else { break };
                let bytes = bytes?;
                file.write_all(&bytes).await?;
                counter.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            }
            file.flush().await?;
            Ok(())
        };
        report_while(work, &counter, size, on_progress).await
    }
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new()
    }
}

/// Выполняет `work` и раз в 250 мс сообщает прогресс.
async fn report_while<T>(
    work: impl std::future::Future<Output = Result<T, Error>>,
    counter: &AtomicU64,
    total: Option<u64>,
    on_progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<T, Error> {
    tokio::pin!(work);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let (mut last_bytes, mut last_time, mut speed) = (counter.load(Ordering::Relaxed), Instant::now(), 0.0);
    loop {
        tokio::select! {
            res = &mut work => {
                let done = counter.load(Ordering::Relaxed);
                on_progress(Progress { phase: Phase::Downloading, done, total, speed });
                return res;
            }
            _ = tick.tick() => {
                let done = counter.load(Ordering::Relaxed);
                let dt = last_time.elapsed().as_secs_f64();
                if dt > 0.0 {
                    let now = done.saturating_sub(last_bytes) as f64 / dt;
                    speed = if speed == 0.0 { now } else { speed * 0.8 + now * 0.2 };
                }
                (last_bytes, last_time) = (done, Instant::now());
                on_progress(Progress { phase: Phase::Downloading, done, total, speed });
            }
        }
    }
}

fn load_state(state_path: &Path, part: &Path, size: u64) -> Option<State> {
    let st: State = serde_json::from_slice(&std::fs::read(state_path).ok()?).ok()?;
    let ok = st.size == size
        && st.chunk > 0
        && st.done.len() as u64 == size.div_ceil(st.chunk)
        && std::fs::metadata(part).ok()?.len() == size;
    ok.then_some(st)
}

fn remove_partial(dest: &Path) {
    let _ = std::fs::remove_file(with_suffix(dest, ".part"));
    let _ = std::fs::remove_file(with_suffix(dest, ".part.json"));
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn header(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers().get(name)?.to_str().ok().map(str::to_string)
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 * MIB as usize];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{body, serve, sha};

    fn request(urls: Vec<String>, dest: PathBuf, sha256: Option<String>) -> Request {
        Request { urls, dest, sha256, connections: 4, chunk_size: 64 * 1024 }
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ollivo-dl-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("file.bin")
    }

    #[tokio::test]
    async fn parallel_with_hash() {
        let data = body(1_000_003);
        let srv = serve(data.clone(), true, 0);
        let dest = tmp("parallel");
        let req = request(vec![srv.url], dest.clone(), Some(sha(&data)));
        Downloader::new().download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!with_suffix(&dest, ".part.json").exists());
    }

    #[tokio::test]
    async fn retries_after_server_errors() {
        let data = body(700_000);
        let srv = serve(data.clone(), true, 3);
        let dest = tmp("retry");
        let req = request(vec![srv.url], dest.clone(), Some(sha(&data)));
        Downloader::new().download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn without_range_support() {
        let data = body(300_000);
        let srv = serve(data.clone(), false, 0);
        let dest = tmp("norange");
        let req = request(vec![srv.url], dest.clone(), None);
        Downloader::new().download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn resumes_only_missing_chunks() {
        let data = body(640 * 1024); // 10 кусков по 64 КБ
        let srv = serve(data.clone(), true, 0);
        let dest = tmp("resume");
        // Как будто прошлый запуск успел скачать первые 7 кусков.
        let mut part = vec![0u8; data.len()];
        part[..7 * 64 * 1024].copy_from_slice(&data[..7 * 64 * 1024]);
        std::fs::write(with_suffix(&dest, ".part"), &part).unwrap();
        let st = State { size: data.len() as u64, chunk: 64 * 1024, done: (0..10).map(|i| i < 7).collect() };
        std::fs::write(with_suffix(&dest, ".part.json"), serde_json::to_vec(&st).unwrap()).unwrap();

        let req = request(vec![srv.url], dest.clone(), Some(sha(&data)));
        Downloader::new().download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        // Запрос размера + 3 недостающих куска.
        assert_eq!(srv.requests.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn bad_hash_falls_back_to_mirror() {
        let data = body(200_000);
        let mut broken = data.clone();
        broken[1000] ^= 0xff;
        let bad = serve(broken, true, 0);
        let good = serve(data.clone(), true, 0);
        let dest = tmp("mirror");
        let req = request(vec![bad.url, good.url], dest.clone(), Some(sha(&data)));
        Downloader::new().download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    /// Настоящий HuggingFace: перенаправление на CDN, SHA256 из `X-Linked-ETag`.
    /// `cargo test -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn huggingface_real() {
        let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";
        let dl = Downloader::new();
        let probe = dl.probe(url).await.unwrap();
        println!("{probe:?}");
        assert!(probe.ranges && probe.sha256.is_some());
        let dest = tmp("hf");
        let started = Instant::now();
        let req = Request { urls: vec![url.into()], dest: dest.clone(), sha256: None, connections: std::env::var("DL_CONN").map_or(8, |v| v.parse().unwrap()), chunk_size: 8 * MIB };
        dl.download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
        let size = std::fs::metadata(&dest).unwrap().len();
        let secs = started.elapsed().as_secs_f64();
        println!("{size} байт за {secs:.1} с, {:.1} МБ/с", size as f64 / secs / MIB as f64);
        assert_eq!(sha256_file(&dest).unwrap(), probe.sha256.unwrap());
    }

    #[tokio::test]
    async fn goes_through_http_proxy() {
        // Тестовый сервер играет роль прокси: сайта `nohost.invalid` не существует,
        // так что файл может прийти только через прокси.
        let data = body(300_000);
        let proxy = serve(data.clone(), true, 0);
        let addr = proxy.url.trim_end_matches("/file.bin").to_string();
        let dl = Downloader::with_proxy(Some(reqwest::Proxy::all(&addr).unwrap()));
        let dest = tmp("proxy");
        let req = request(vec!["http://nohost.invalid/file.bin".into()], dest.clone(), Some(sha(&data)));
        dl.download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(proxy.requests.load(Ordering::SeqCst) > 1);
    }

    #[tokio::test]
    async fn bearer_only_to_its_host() {
        let data = body(200_000);
        for (host, expect) in [("127.0.0.1", "Bearer hf_x"), ("huggingface.co", "")] {
            let srv = serve(data.clone(), true, 0);
            let dl = Downloader::new().with_bearer(vec![host.into()], "hf_x".into());
            let dest = tmp(&format!("bearer-{host}"));
            let req = request(vec![srv.url.clone()], dest, None);
            dl.download(&req, &CancellationToken::new(), &|_| {}).await.unwrap();
            let seen = srv.auth.lock().unwrap().clone();
            assert!(seen.iter().all(|a| a == expect), "{host}: {seen:?}");
        }
    }

    #[tokio::test]
    async fn cancel_keeps_partial() {
        let data = body(2_000_000);
        let srv = serve(data.clone(), true, 0);
        let dest = tmp("cancel");
        let req = request(vec![srv.url], dest.clone(), None);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = Downloader::new().download(&req, &cancel, &|_| {}).await.unwrap_err();
        assert!(matches!(err, Error::Cancelled));
        assert!(!dest.exists());
    }
}
