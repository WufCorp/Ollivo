//! HuggingFace: основной сайт или зеркало, токен для закрытых моделей.
//!
//! Зеркало (hf-mirror.com или своё) повторяет адреса HF один в один, поэтому
//! меняется только начало адреса. Токен уходит только на сайт HF/зеркала —
//! на CDN, куда HF перенаправляет загрузку, он не передаётся (см. `download`).

use serde::{Deserialize, Serialize};

pub const OFFICIAL: &str = "https://huggingface.co";
pub const MIRROR: &str = "https://hf-mirror.com";
const OFFICIAL_HOST: &str = "huggingface.co";

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?.host_str().map(str::to_string)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HfSource {
    #[default]
    Official,
    Mirror,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HfSettings {
    pub source: HfSource,
    /// Адрес своего зеркала, только для `Custom`.
    pub custom_url: String,
}

impl HfSettings {
    /// Начало адреса без `/` в конце, например `https://hf-mirror.com`.
    pub fn base(&self) -> Result<String, String> {
        let url = match self.source {
            HfSource::Official => return Ok(OFFICIAL.into()),
            HfSource::Mirror => return Ok(MIRROR.into()),
            HfSource::Custom => self.custom_url.trim().trim_end_matches('/'),
        };
        let parsed = reqwest::Url::parse(url).map_err(|_| "адрес зеркала записан с ошибкой".to_string())?;
        if !matches!(parsed.scheme(), "https" | "http") || parsed.host_str().is_none() {
            return Err("адрес зеркала должен начинаться с https://".into());
        }
        Ok(url.to_string())
    }

    /// Хосты, которым можно отправлять токен: сам HF и выбранное зеркало.
    /// HF — всегда: зеркала бывает перенаправляют на него (hf-mirror.com так делает не из Китая).
    pub fn token_hosts(&self) -> Vec<String> {
        let mut hosts = vec![OFFICIAL_HOST.to_string()];
        if let Some(h) = self.base().ok().and_then(|b| host_of(&b)) {
            if h != OFFICIAL_HOST {
                hosts.push(h);
            }
        }
        hosts
    }
}

/// Адрес файла модели: `<base>/<repo>/resolve/<revision>/<file>`.
#[allow(dead_code)] // для загрузки моделей в фазе 2
pub fn file_url(base: &str, repo: &str, revision: &str, file: &str) -> String {
    format!("{base}/{repo}/resolve/{revision}/{file}")
}

pub fn load_token() -> String {
    crate::net::secret::load(crate::net::secret::HF_TOKEN)
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenCheck {
    pub ok: bool,
    pub message: String,
}

/// Проверка токена: `/api/whoami-v2` возвращает имя владельца.
pub async fn check_token(client: &reqwest::Client, base: &str, token: &str) -> TokenCheck {
    if token.is_empty() {
        return TokenCheck { ok: false, message: "токен не указан".into() };
    }
    if !token.starts_with("hf_") {
        return TokenCheck { ok: false, message: "токен HuggingFace начинается с hf_".into() };
    }
    let mut resp = client.get(format!("{base}/api/whoami-v2")).bearer_auth(token).send().await;
    // Зеркало перенаправило на сам HF — токен при переходе отброшен, спрашиваем HF напрямую.
    if let Ok(r) = &resp {
        if r.url().host_str() == Some(OFFICIAL_HOST) && host_of(base).as_deref() != Some(OFFICIAL_HOST) {
            resp = client.get(format!("{OFFICIAL}/api/whoami-v2")).bearer_auth(token).send().await;
        }
    }
    match resp {
        Ok(r) if r.status().is_success() => {
            let name = r
                .bytes()
                .await
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .unwrap_or_default();
            TokenCheck { ok: true, message: format!("токен работает, аккаунт {name}") }
        }
        Ok(r) if r.status().as_u16() == 401 => {
            TokenCheck { ok: false, message: "токен неверный или отозван".into() }
        }
        Ok(r) if r.status().as_u16() == 404 => TokenCheck {
            ok: false,
            message: "зеркало не проверяет токены — проверьте на основном сайте".into(),
        },
        Ok(r) => TokenCheck { ok: false, message: format!("сайт ответил ошибкой {}", r.status().as_u16()) },
        Err(e) if e.is_timeout() => TokenCheck { ok: false, message: "сайт не ответил — нужен прокси или зеркало?".into() },
        Err(_) => TokenCheck { ok: false, message: "сайт не открывается — нужен прокси или зеркало?".into() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bases() {
        let mut s = HfSettings::default();
        assert_eq!(s.base().unwrap(), OFFICIAL);
        assert_eq!(s.token_hosts(), vec!["huggingface.co"]);
        s.source = HfSource::Mirror;
        assert_eq!(s.token_hosts(), vec!["huggingface.co", "hf-mirror.com"]);
        s.source = HfSource::Custom;
        s.custom_url = "https://hf.example.ru/ ".into();
        assert_eq!(s.base().unwrap(), "https://hf.example.ru");
        s.custom_url = "hf.example.ru".into();
        assert!(s.base().is_err());
    }

    #[test]
    fn file_urls() {
        assert_eq!(
            file_url(MIRROR, "Qwen/Qwen2.5-0.5B-Instruct-GGUF", "main", "q4.gguf"),
            "https://hf-mirror.com/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/q4.gguf"
        );
    }

    #[tokio::test]
    async fn token_format_checked_locally() {
        let c = reqwest::Client::new();
        assert!(!check_token(&c, OFFICIAL, "").await.ok);
        assert!(!check_token(&c, OFFICIAL, "abc").await.ok);
    }
}
