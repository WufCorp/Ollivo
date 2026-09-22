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

impl Engine {
    /// Сборка для этого ПК: `wanted`, если она задана и пойдёт, иначе первая
    /// подходящая из `prefer`.
    pub fn pick(&self, hw: Build, wanted: Option<Build>) -> Option<&EngineBuild> {
        let find = |b: Build| self.builds.iter().find(|x| x.build == b && b.runs_on(hw));
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
        let llama = m.engine("llama.cpp").unwrap();
        for b in &llama.builds {
            assert!(!b.files.is_empty());
            for f in &b.files {
                assert_eq!(f.sha256.len(), 64, "{}", f.name);
                assert!(!f.urls.is_empty() && f.size > 0, "{}", f.name);
            }
        }
    }

    #[test]
    fn pick_respects_hardware() {
        let m = Manifest::bundled();
        let llama = m.engine("llama.cpp").unwrap();
        // По умолчанию чат на Vulkan (решение фазы 0).
        assert_eq!(llama.pick(Build::Cuda12, None).unwrap().build, Build::Vulkan);
        // Ускоритель CUDA 12 на GTX 10xx — можно, CUDA 13 — нельзя.
        assert_eq!(llama.pick(Build::Cuda12, Some(Build::Cuda12)).unwrap().build, Build::Cuda12);
        assert_eq!(llama.pick(Build::Cuda12, Some(Build::Cuda13)).unwrap().build, Build::Vulkan);
        assert_eq!(llama.pick(Build::Cuda13, Some(Build::Cuda13)).unwrap().build, Build::Cuda13);
        assert_eq!(llama.pick(Build::Vulkan, Some(Build::Cuda12)).unwrap().build, Build::Vulkan);
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
