//! Запуск и остановка процессов движков (llama-server, позже whisper, ComfyUI).
//!
//! - Без окна консоли; stdout и stderr — в `logs\<движок>.log`, прошлый запуск — в `.prev.log`.
//! - Все процессы в одном Job Object с «убить при закрытии»: если Ollivo закроют
//!   или она упадёт, Windows сама завершит движки, и они не будут держать видеопамять.
//! - Процесс сторожит отдельная задача: знает, когда он вышел и почему.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct Spec {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub log: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit {
    pub code: Option<i32>,
    /// Остановили мы сами, а не упал.
    pub by_us: bool,
}

/// Живой процесс движка. Drop не убивает процесс — для этого `stop`.
/// Клоны смотрят на тот же процесс (например, сторож падений).
#[derive(Clone)]
pub struct Handle {
    pub pid: u32,
    pub log: PathBuf,
    kill: CancellationToken,
    exit: watch::Receiver<Option<Exit>>,
}

impl Handle {
    pub fn exited(&self) -> Option<Exit> {
        *self.exit.borrow()
    }

    pub async fn wait(&self) -> Exit {
        let mut rx = self.exit.clone();
        loop {
            if let Some(e) = *rx.borrow_and_update() {
                return e;
            }
            if rx.changed().await.is_err() {
                return Exit { code: None, by_us: false };
            }
        }
    }

    pub async fn stop(&self) -> Exit {
        self.kill.cancel();
        self.wait().await
    }
}

pub struct Supervisor {
    /// `None` — Job Object не создался; движки работают, но без авто-завершения.
    job: Option<win32job::Job>,
}

impl Supervisor {
    pub fn new() -> Self {
        let job = win32job::Job::create().ok().filter(|job| {
            let mut info = win32job::ExtendedLimitInfo::new();
            info.limit_kill_on_job_close();
            job.set_extended_limit_info(&info).is_ok()
        });
        Self { job }
    }

    pub fn spawn(&self, spec: &Spec) -> std::io::Result<Handle> {
        if let Some(dir) = spec.log.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if spec.log.exists() {
            let _ = std::fs::rename(&spec.log, spec.log.with_extension("prev.log"));
        }
        let log = std::fs::File::create(&spec.log)?;

        let mut cmd = tokio::process::Command::new(&spec.exe);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        if let Some(dir) = spec.exe.parent() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);

        #[cfg(windows)]
        if let (Some(job), Some(h)) = (&self.job, child.raw_handle()) {
            let _ = job.assign_process(h as isize);
        }

        let kill = CancellationToken::new();
        let (tx, rx) = watch::channel(None);
        let k = kill.clone();
        tokio::spawn(async move {
            let exit = tokio::select! {
                status = child.wait() => Exit { code: status.ok().and_then(|s| s.code()), by_us: false },
                _ = k.cancelled() => {
                    let _ = child.kill().await;
                    Exit { code: child.wait().await.ok().and_then(|s| s.code()), by_us: true }
                }
            };
            let _ = tx.send(Some(exit));
        });

        Ok(Handle { pid, log: spec.log.clone(), kill, exit: rx })
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

/// Последние `lines` строк лога — чтобы показать, почему движок упал.
pub fn log_tail(path: &Path, lines: usize) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(64 * 1024)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[derive(Debug, thiserror::Error)]
pub enum ReadyError {
    #[error("движок завершился при запуске (код {code:?})\n{tail}")]
    Crashed { code: Option<i32>, tail: String },
    #[error("движок не ответил за {0} с")]
    Timeout(u64),
}

/// Ждёт, пока `url` ответит 200. Пока процесс грузит модель, llama-server отвечает 503.
pub async fn wait_ready(handle: &Handle, url: &str, timeout: Duration) -> Result<(), ReadyError> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("HTTP-клиент");
    let started = Instant::now();
    loop {
        if let Some(e) = handle.exited() {
            return Err(ReadyError::Crashed { code: e.code, tail: log_tail(&handle.log, 15) });
        }
        if let Ok(r) = client.get(url).send().await {
            if r.status().is_success() {
                return Ok(());
            }
        }
        if started.elapsed() > timeout {
            return Err(ReadyError::Timeout(timeout.as_secs()));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Свободный порт на 127.0.0.1.
pub fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_log(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ollivo-proc-{}-{name}", std::process::id())).join("t.log")
    }

    fn spec(args: &[&str], log: PathBuf) -> Spec {
        Spec {
            exe: PathBuf::from(r"C:\Windows\System32\cmd.exe"),
            args: args.iter().map(|s| s.to_string()).collect(),
            log,
        }
    }

    #[tokio::test]
    async fn logs_output_and_exit_code() {
        let log = tmp_log("exit");
        let sup = Supervisor::new();
        let h = sup.spawn(&spec(&["/c", "echo", "hello-from-engine", "&", "exit", "/b", "3"], log.clone())).unwrap();
        let e = h.wait().await;
        assert_eq!(e, Exit { code: Some(3), by_us: false });
        assert!(log_tail(&log, 5).contains("hello-from-engine"));
    }

    #[tokio::test]
    async fn stop_kills_long_process() {
        let log = tmp_log("stop");
        let sup = Supervisor::new();
        let h = sup.spawn(&spec(&["/c", "ping", "-n", "30", "127.0.0.1"], log)).unwrap();
        assert!(h.exited().is_none());
        let started = Instant::now();
        let e = h.stop().await;
        assert!(e.by_us);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn crash_during_start_is_reported_with_log() {
        let log = tmp_log("crash");
        let sup = Supervisor::new();
        let h = sup.spawn(&spec(&["/c", "echo", "model-not-found", "&", "exit", "/b", "1"], log)).unwrap();
        let port = free_port().unwrap();
        let err = wait_ready(&h, &format!("http://127.0.0.1:{port}/health"), Duration::from_secs(10))
            .await
            .unwrap_err();
        match err {
            ReadyError::Crashed { code, tail } => {
                assert_eq!(code, Some(1));
                assert!(tail.contains("model-not-found"), "{tail}");
            }
            e => panic!("{e}"),
        }
    }

    /// Главная защита: закрылся Supervisor (= закрылась Ollivo) — Windows убивает движки.
    #[tokio::test]
    async fn dropping_supervisor_kills_engines() {
        let sup = Supervisor::new();
        let h = sup.spawn(&spec(&["/c", "ping", "-n", "30", "127.0.0.1"], tmp_log("job"))).unwrap();
        drop(sup);
        let e = tokio::time::timeout(Duration::from_secs(5), h.wait()).await.expect("процесс не умер");
        assert!(!e.by_us);
    }

    #[test]
    fn rotates_previous_log() {
        let log = tmp_log("rotate");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "old run").unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sup = Supervisor::new();
            sup.spawn(&spec(&["/c", "exit", "/b", "0"], log.clone())).unwrap().wait().await;
        });
        assert_eq!(std::fs::read_to_string(log.with_extension("prev.log")).unwrap(), "old run");
    }
}

