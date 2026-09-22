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
