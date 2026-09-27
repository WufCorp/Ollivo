//! Прокси: настройки, сборка HTTP-клиента, проверка «подключается ли».
//!
//! Через прокси идут все обращения ядра в сеть: движки, модели, манифест.
//! Пароль хранится не в `settings.json`, а в диспетчере учётных данных Windows.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    #[default]
    Http,
    Socks5,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxySettings {
    pub enabled: bool,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub auth: bool,
    pub username: String,
}

impl ProxySettings {
    /// Адрес прокси вида `socks5h://user:pass@host:port`. `None` — прокси выключен.
    pub fn url(&self, password: &str) -> Result<Option<String>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err(t!("не указан адрес прокси", "proxy address is missing").into());
        }
        if host.contains("://") || host.contains('/') || host.contains('@') {
            return Err(t!(
                "в поле «Адрес» нужен только адрес, например 127.0.0.1 или proxy.example.com",
                "the “Address” field needs just the address, e.g. 127.0.0.1 or proxy.example.com"
            )
            .into());
        }
        if self.port == 0 {
            return Err(t!("не указан порт прокси", "proxy port is missing").into());
        }
        // socks5h: имена сайтов тоже разрешает прокси — провайдер не видит, куда идём.
        let scheme = match self.kind {
            ProxyKind::Http => "http",
            ProxyKind::Socks5 => "socks5h",
        };
        let mut url = reqwest::Url::parse(&format!("{scheme}://{host}:{}", self.port))
            .map_err(|_| t!("адрес прокси записан с ошибкой", "the proxy address is malformed").to_string())?;
        if self.auth {
            if self.username.is_empty() {
                return Err(t!("не указан логин для прокси", "proxy login is missing").into());
            }
            url.set_username(&self.username).map_err(|_| t!("логин не подходит", "the login is not valid").to_string())?;
            url.set_password(Some(password)).map_err(|_| t!("пароль не подходит", "the password is not valid").to_string())?;
        }
        Ok(Some(url.into()))
    }

    pub fn reqwest(&self, password: &str) -> Result<Option<reqwest::Proxy>, String> {
        match self.url(password)? {
            Some(u) => reqwest::Proxy::all(&u).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }
}

/// Секреты в диспетчере учётных данных Windows: запись «Ollivo / <name>».
pub mod secret {
    const SERVICE: &str = "Ollivo";
    pub const PROXY_PASSWORD: &str = "proxy";
    pub const HF_TOKEN: &str = "huggingface";

    pub fn load(name: &str) -> String {
        keyring::Entry::new(SERVICE, name).and_then(|e| e.get_password()).unwrap_or_default()
    }

    /// Пустая строка — удалить.
    pub fn store(name: &str, value: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(SERVICE, name).map_err(|e| e.to_string())?;
        if value.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        } else {
            entry.set_password(value).map_err(|e| e.to_string())
        }
    }
}

pub fn load_password() -> String {
    secret::load(secret::PROXY_PASSWORD)
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestReport {
    pub ok: bool,
    pub checks: Vec<Check>,
}

/// Куда проверяем доступ: отсюда программа качает движки и модели.
const TARGETS: &[(&str, &str)] = &[
    ("HuggingFace", "https://huggingface.co/api/models?limit=1"),
    ("GitHub", "https://api.github.com/zen"),
];

pub async fn test(settings: &ProxySettings, password: &str) -> TestReport {
    let mut checks = Vec::new();
    let fail = |checks: Vec<Check>| TestReport { ok: false, checks };

    let proxy = match settings.reqwest(password) {
        Ok(p) => p,
        Err(e) => return fail(vec![Check { name: t!("Настройки", "Settings").into(), ok: false, message: e }]),
    };

    if proxy.is_some() {
        // Шаг 1: отвечает ли сам прокси — отдельно, чтобы не путать с блокировкой сайтов.
        let addr = format!("{}:{}", settings.host.trim(), settings.port);
        let started = Instant::now();
        let conn = tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(&addr)).await;
        let (ok, message) = match conn {
            Ok(Ok(_)) => (true, tf!("отвечает, {} мс", "responds, {} ms", started.elapsed().as_millis())),
            Ok(Err(e)) => (false, tf!("не подключается к {addr}: {}", "can't connect to {addr}: {}", friendly_io(&e))),
            Err(_) => (false, tf!("{addr} не ответил за 5 секунд", "{addr} did not respond within 5 seconds")),
        };
        checks.push(Check { name: t!("Прокси", "Proxy").into(), ok, message });
        if !ok {
            return fail(checks);
        }
    }

    let mut b = reqwest::Client::builder()
        .user_agent(concat!("Ollivo/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20));
    b = match proxy {
        Some(p) => b.proxy(p),
        None => b.no_proxy(),
    };
    let client = match b.build() {
        Ok(c) => c,
        Err(e) => return fail(vec![Check { name: t!("Настройки", "Settings").into(), ok: false, message: e.to_string() }]),
    };

    // Шаг 2: открываются ли через него нужные сайты.
    let mut all_ok = true;
    for (name, url) in TARGETS {
        let started = Instant::now();
        let (ok, message) = match client.get(*url).send().await {
            Ok(r) if r.status().is_success() => (true, tf!("открывается, {} мс", "opens, {} ms", started.elapsed().as_millis())),
            Ok(r) if r.status().as_u16() == 407 => {
                (false, t!("прокси просит логин и пароль или они неверные", "the proxy asks for a login and password, or they are wrong").into())
            }
            Ok(r) => (false, tf!("сайт ответил ошибкой {}", "the site returned error {}", r.status().as_u16())),
            Err(e) => (false, friendly_reqwest(&e)),
        };
        all_ok &= ok;
        checks.push(Check { name: name.to_string(), ok, message });
    }
    TestReport { ok: all_ok, checks }
}

fn friendly_io(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => t!("порт закрыт — прокси не запущен или порт другой", "the port is closed — the proxy isn't running or the port is different").into(),
        TimedOut => t!("нет ответа", "no response").into(),
        _ if e.to_string().contains("11001") => t!("такого адреса нет", "no such address").into(),
        _ => e.to_string(),
    }
}

fn friendly_reqwest(e: &reqwest::Error) -> String {
    let text = format!("{e:?}");
    if e.is_timeout() {
        t!("нет ответа за 20 секунд", "no response within 20 seconds").into()
    } else if text.contains("407") || text.to_lowercase().contains("auth") {
        t!("прокси не принял логин или пароль", "the proxy did not accept the login or password").into()
    } else if e.is_connect() {
        t!("не удалось подключиться — сайт недоступен через этот прокси", "couldn't connect — the site is unreachable through this proxy").into()
    } else {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(kind: ProxyKind, auth: bool) -> ProxySettings {
        ProxySettings {
            enabled: true,
            kind,
            host: "127.0.0.1".into(),
            port: 1080,
            auth,
            username: "user".into(),
        }
    }

    #[test]
    fn builds_urls() {
        assert_eq!(proxy(ProxyKind::Http, false).url("").unwrap().unwrap(), "http://127.0.0.1:1080/");
        assert_eq!(
            proxy(ProxyKind::Socks5, true).url("p@ss:w").unwrap().unwrap(),
            "socks5h://user:p%40ss%3Aw@127.0.0.1:1080"
        );
        assert_eq!(ProxySettings::default().url("").unwrap(), None);
    }

    #[test]
    fn rejects_bad_fields() {
        let mut p = proxy(ProxyKind::Http, false);
        p.host = "http://1.2.3.4".into();
        assert!(p.url("").is_err());
        p.host = "1.2.3.4".into();
        p.port = 0;
        assert!(p.url("").is_err());
        let mut p = proxy(ProxyKind::Http, true);
        p.username.clear();
        assert!(p.url("").is_err());
    }

    #[tokio::test]
    async fn dead_proxy_is_reported_first() {
        // Порт 1 на localhost закрыт — должно упасть на шаге «Прокси», до сайтов.
        let mut p = proxy(ProxyKind::Http, false);
        p.port = 1;
        let r = test(&p, "").await;
        assert!(!r.ok);
        assert_eq!(r.checks.len(), 1);
        assert_eq!(r.checks[0].name, "Прокси");
    }
}
