//! Манифест движков: проверенные версии, адреса, SHA256, сборки под железо.
//!
//! Копия `manifest/engines.json` встроена в программу — она работает и без сети.
//! Свежий манифест приходит из S3 (`ollivo/manifest/engines.json`) и заменяет
//! встроенный, если у него больше `revision`.

use crate::hardware::Build;
use serde::{Deserialize, Serialize};

const BUNDLED: &str = include_str!("../../manifest/engines.json");
const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub revision: u32,
    pub engines: Vec<Engine>,
    /// Окружение Python для ComfyUI. Нет — картинки этой версией манифеста не ставятся.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<PythonSpec>,
}

/// Что поставить в свой Python для картинок (`pyenv.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonSpec {
    /// Точная версия для `uv python install`: сам архив uv сверяет по своим хешам.
    pub version: String,
    /// Зависимости ComfyUI разрешаются на эту дату (`uv --exclude-newer`): в его
    /// `requirements.txt` версии почти не закреплены, без даты у двух людей
    /// в один день могли бы встать разные пакеты.
    pub exclude_newer: String,
    /// Пакеты из `requirements.txt` ComfyUI, которые не ставим: веб-интерфейс
    /// и его шаблоны (~0,5 ГБ) — окно у нас своё.
    #[serde(default)]
    pub skip: Vec<String>,
    /// Колёса torch, torchvision, torchaudio под сборку видеокарты: GTX 10xx — CUDA 12.6,
    /// RTX — CUDA 13.0 (фаза 0). Качает наш загрузчик — в один поток uv тянул бы их ~40 минут.
    pub torch: Vec<EngineBuild>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engine {
    pub id: String,
    pub title: String,
    pub version: String,
    /// Исполняемый файл после распаковки, путь внутри папки движка.
    pub exe: String,
    /// Порядок предпочтения сборок. Берётся первая, что пойдёт на этом ПК.
    pub prefer: Vec<Build>,
    pub builds: Vec<EngineBuild>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineBuild {
    pub build: Build,
    pub files: Vec<EngineFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineFile {
    pub name: String,
    pub urls: Vec<String>,
    pub sha256: String,
    pub size: u64,
    /// Что взять из архива — пути внутри него через `/`; пусто — всё. В сборке ffmpeg
    /// рядом с нужным `ffmpeg.exe` лежат ещё две программы по стольку же мегабайт.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
}

impl Manifest {
    pub fn bundled() -> Self {
        Self::parse(BUNDLED).expect("встроенный манифест")
    }

    pub fn parse(json: &str) -> Result<Self, String> {
        let m: Manifest = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if m.schema != SCHEMA {
            return Err(format!("манифест схемы {}, программа понимает {SCHEMA}", m.schema));
        }
        Ok(m)
    }

    /// Выбирает более свежий из двух манифестов.
    #[allow(dead_code)] // понадобится, когда манифест начнёт приходить из S3
    pub fn newest(self, other: Option<Manifest>) -> Manifest {
        match other {
            Some(o) if o.revision > self.revision => o,
            _ => self,
        }
    }

    pub fn engine(&self, id: &str) -> Option<&Engine> {
        self.engines.iter().find(|e| e.id == id)
    }
}

impl PythonSpec {
    /// Колёса torch под сборку этого ПК. Без NVIDIA (`Vulkan`) картинок нет — в MVP только CUDA.
    pub fn torch_for(&self, hw: Build) -> Option<&EngineBuild> {
        self.torch.iter().find(|b| b.build == hw)
    }
}

impl Engine {
    /// Сборка для этого ПК: `wanted`, если она задана и пойдёт, иначе первая
    /// подходящая из `prefer`. `vulkan` — есть ли в системе `vulkan-1.dll`.
    pub fn pick(&self, hw: Build, vulkan: bool, wanted: Option<Build>) -> Option<&EngineBuild> {
        let ok = |b: Build| b.runs_on(hw) && (b != Build::Vulkan || vulkan);
        let find = |b: Build| self.builds.iter().find(|x| x.build == b && ok(b));
        wanted.and_then(find).or_else(|| self.prefer.iter().find_map(|b| find(*b)))
    }
}

impl EngineBuild {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_is_valid() {
        let m = Manifest::bundled();
        assert!(m.engine("llama.cpp").is_some() && m.engine("whisper.cpp").is_some());
        let py = m.python.as_ref().expect("окружение для картинок");
        let torch = py.torch.iter().chain(m.engines.iter().flat_map(|e| &e.builds));
        for b in torch {
            assert!(!b.files.is_empty());
            for f in &b.files {
                assert_eq!(f.sha256.len(), 64, "{}", f.name);
                assert!(!f.urls.is_empty() && f.size > 0, "{}", f.name);
            }
        }
    }

    /// Распознавание речи ставится на любой ПК — и без видеокарты, и без Vulkan.
    #[test]
    fn whisper_goes_anywhere() {
        let m = Manifest::bundled();
        let w = m.engine("whisper.cpp").unwrap();
        for hw in [Build::Cuda13, Build::Cuda12, Build::Vulkan] {
            assert_eq!(w.pick(hw, false, None).unwrap().build, Build::Cpu);
        }
    }

    #[test]
    fn pick_respects_hardware() {
        let m = Manifest::bundled();
        let llama = m.engine("llama.cpp").unwrap();
        // По умолчанию чат на Vulkan (решение фазы 0).
        assert_eq!(llama.pick(Build::Cuda12, true, None).unwrap().build, Build::Vulkan);
        // Ускоритель CUDA 12 на GTX 10xx — можно, CUDA 13 — нельзя.
        assert_eq!(llama.pick(Build::Cuda12, true, Some(Build::Cuda12)).unwrap().build, Build::Cuda12);
        assert_eq!(llama.pick(Build::Cuda12, true, Some(Build::Cuda13)).unwrap().build, Build::Vulkan);
        assert_eq!(llama.pick(Build::Cuda13, true, Some(Build::Cuda13)).unwrap().build, Build::Cuda13);
        assert_eq!(llama.pick(Build::Vulkan, true, Some(Build::Cuda12)).unwrap().build, Build::Vulkan);
        // Нет Vulkan в системе — ставим CUDA; нет ни того, ни другого — нечего ставить.
        assert_eq!(llama.pick(Build::Cuda12, false, None).unwrap().build, Build::Cuda12);
        assert!(llama.pick(Build::Vulkan, false, None).is_none());
    }

    /// Картинкам нужна NVIDIA: GTX 10xx получает torch на CUDA 12.6, RTX — на 13.0.
    #[test]
    fn torch_by_hardware() {
        let m = Manifest::bundled();
        let py = m.python.as_ref().unwrap();
        let cu12 = py.torch_for(Build::Cuda12).unwrap();
        assert!(cu12.files.iter().all(|f| f.name.contains("+cu126-cp312")), "{:?}", cu12.files);
        let cu13 = py.torch_for(Build::Cuda13).unwrap();
        assert!(cu13.files.iter().all(|f| f.name.contains("+cu130-cp312")));
        assert!(py.torch_for(Build::Vulkan).is_none());
        assert!(py.version.starts_with("3.12."), "колёса torch собраны под cp312");
    }

    #[test]
    fn newest_wins_by_revision() {
        let a = Manifest::bundled();
        let mut b = a.clone();
        b.revision += 1;
        assert_eq!(a.clone().newest(Some(b)).revision, a.revision + 1);
        assert_eq!(a.clone().newest(None).revision, a.revision);
        assert!(Manifest::parse(r#"{"schema":2,"revision":9,"engines":[]}"#).is_err());
    }
}
