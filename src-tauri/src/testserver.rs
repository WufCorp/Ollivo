//! Тестовый HTTP-сервер для загрузок и установки движков.

use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Мини-сервер: отдаёт `body`, понимает Range, умеет сбоить.
pub struct Server {
    pub url: String,
    pub requests: Arc<AtomicUsize>,
    /// Заголовки `Authorization` всех запросов (пусто — не было).
    pub auth: Arc<Mutex<Vec<String>>>,
}

pub fn serve(body: Vec<u8>, ranges: bool, fail_every: usize) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let auth = Arc::new(Mutex::new(Vec::new()));
    let (body, count, auth_log) = (Arc::new(body), requests.clone(), auth.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let (body, count, auth_log) = (body.clone(), count.clone(), auth_log.clone());
            std::thread::spawn(move || {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = None;
                let mut authorization = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("authorization:") {
                        authorization = line[14..].trim().to_string();
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        let (a, b) = v.trim().split_once('-').unwrap();
                        range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
                    }
                }
                auth_log.lock().unwrap().push(authorization);
                let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                if fail_every > 0 && n % fail_every == 0 {
                    let _ = stream.write_all(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    return;
                }
                let (head, data) = match range.filter(|_| ranges) {
                    Some((a, b)) => (
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\n",
                            body.len(),
                            b - a + 1
                        ),
                        &body[a..=b],
                    ),
                    None => (format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n", body.len()), &body[..]),
                };
                let _ = stream.write_all(format!("{head}Connection: close\r\n\r\n").as_bytes());
                let _ = stream.write_all(data);
            });
        }
    });
    Server { url, requests, auth }
}

pub fn body(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

pub fn sha(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Папка «как у человека по имени Иван Петров»: кириллица и пробелы в каждом звене пути.
/// На диске D:, рядом с моделями, — чтобы жёсткие ссылки работали без копирования гигабайт.
pub fn human_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(r"D:\Ollivo\lab\Иван Петров\Ollivo данные").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Жёсткие ссылки на все файлы папки: движок «переезжает» мгновенно и без лишнего места.
pub fn link_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            link_tree(&e.path(), &dest);
        } else {
            std::fs::hard_link(e.path(), &dest).unwrap();
        }
    }
}

/// Откуда процесс на самом деле загрузил библиотеку (`msvcp140` и т.п.).
pub fn loaded_from(pid: u32, module: &str) -> String {
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
         (Get-Process -Id {pid}).Modules | Where-Object ModuleName -eq '{module}' | ForEach-Object FileName"
    );
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Временная папка тестов — `target\test-tmp` на диске проекта, а не системный TEMP: тесты
/// оставляли там папки на каждый прогон (к 2026-09-27 — ~3000 папок, 2,2 ГБ на диске C:).
/// Уходит вместе с `cargo clean`.
pub fn tmp() -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-tmp");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
