//! Где ещё на компьютере лежат модели: LM Studio, Ollama, ComfyUI, наша папка данных.
//!
//! Ничего не копируем и не переносим — берём только пути, чтобы человек не качал
//! второй раз то, что уже скачано другой программой.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Файлы моделей ищем только по этим расширениям; у Ollama расширений нет —
/// там пути берутся из манифестов.
const EXTENSIONS: [&str; 2] = ["gguf", "safetensors"];

/// Вглубь дальше не идём: у ComfyUI модели лежат максимум на 2–3 уровне.
const MAX_DEPTH: usize = 5;

/// Защита от папки, куда случайно указали (например, весь диск).
const MAX_FILES: usize = 2000;

/// Мелочь вроде конфигов и превью — не модели.
const MIN_SIZE: u64 = 4 << 20;

/// Известное место с моделями.
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub title: String,
    pub dir: PathBuf,
}

/// Найденный файл модели. `title` — понятное имя, если у файла его не видно
/// (у Ollama файлы называются по хешу).
#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    pub title: Option<String>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Папки, где уже лежат чужие модели. Возвращаются только существующие.
pub fn sources(data_dir: &Path) -> Vec<Source> {
    let mut found: Vec<Source> = vec![];
    let mut add = |title: &str, dir: PathBuf| {
        if dir.is_dir() && !found.iter().any(|s| s.dir == dir) {
            found.push(Source { title: title.into(), dir });
        }
    };

    add("Папка Ollivo", data_dir.join("models"));
    if let Some(home) = home() {
        add("LM Studio", home.join(".lmstudio").join("models"));
        // Раскладка старых версий LM Studio.
        add("LM Studio", home.join(".cache").join("lm-studio").join("models"));
        add("Ollama", home.join(".ollama").join("models"));
        add("ComfyUI", home.join("ComfyUI").join("models"));
    }
    // Ollama можно перенести на другой диск переменной окружения.
    if let Some(dir) = std::env::var_os("OLLAMA_MODELS") {
        add("Ollama", PathBuf::from(dir));
    }
    // ComfyUI обычно распакован в корень диска — туда и смотрим.
    for disk in crate::hardware::detect_disks() {
        let root = PathBuf::from(&disk.mount);
        add("ComfyUI", root.join("ComfyUI").join("models"));
        add("ComfyUI", root.join("ComfyUI_windows_portable").join("ComfyUI").join("models"));
    }
    found
}

/// Ищет файлы моделей в папке. Папка Ollama разбирается по манифестам.
pub fn scan(dir: &Path) -> Vec<Found> {
    if dir.join("blobs").is_dir() && dir.join("manifests").is_dir() {
        return ollama(dir);
    }
    let mut found = vec![];
    walk(dir, 0, &mut found);
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<Found>) {
    if depth > MAX_DEPTH || found.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            walk(&path, depth + 1, found);
        } else if meta.len() >= MIN_SIZE && has_model_ext(&path) {
            found.push(Found { path, title: None });
            if found.len() >= MAX_FILES {
                return;
            }
        }
    }
}

fn has_model_ext(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}

/// Ollama: файлы лежат в `blobs` под именем-хешем, а понятное имя — в манифесте
/// `manifests/<реестр>/<автор>/<модель>/<тег>`. Берём слой с самой моделью.
fn ollama(dir: &Path) -> Vec<Found> {
    let mut manifests = vec![];
    collect_files(&dir.join("manifests"), 0, &mut manifests);
    let mut found = vec![];
    for m in manifests {
        let Ok(text) = std::fs::read_to_string(&m) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let digest = json["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["mediaType"].as_str() == Some("application/vnd.ollama.image.model"))
            .and_then(|l| l["digest"].as_str());
        let Some(digest) = digest else { continue };
        let path = dir.join("blobs").join(digest.replace(':', "-"));
        // Модель могли удалить, оставив манифест — такого файла просто нет.
        if !path.is_file() {
            continue;
        }
        found.push(Found { path, title: ollama_name(dir, &m) });
    }
    found
}

/// `manifests\registry.ollama.ai\library\gemma3\4b` → `gemma3:4b`.
/// Автор остаётся в имени, если это не официальная `library`.
fn ollama_name(root: &Path, manifest: &Path) -> Option<String> {
    let rest = manifest.strip_prefix(root.join("manifests")).ok()?;
    let parts: Vec<String> =
        rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let [.., author, model, tag] = parts.as_slice() else { return None };
    let name = if author == "library" { model.clone() } else { format!("{author}/{model}") };
    Some(format!("{name}:{tag}"))
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        match e.metadata() {
            Ok(m) if m.is_dir() => collect_files(&path, depth + 1, out),
            Ok(_) => out.push(path),
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ollivo-scan-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn big_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; MIN_SIZE as usize + 1]).unwrap();
    }

    #[test]
    fn finds_models_deep_and_skips_junk() {
        let dir = tmp("walk");
        big_file(&dir.join("checkpoints").join("sdxl.safetensors"));
        big_file(&dir.join("a").join("b").join("c").join("chat.gguf"));
        big_file(&dir.join("notes.txt"));
        std::fs::write(dir.join("tiny.gguf"), b"GGUF").unwrap();

        let mut names: Vec<String> =
            scan(&dir).iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into()).collect();
        names.sort();
        assert_eq!(names, ["chat.gguf", "sdxl.safetensors"]);
    }

    /// Ollama: имя берём из манифеста, а манифест без файла пропускаем.
    #[test]
    fn ollama_names_from_manifests() {
        let dir = tmp("ollama");
        let manifest = |name: &str, digest: &str| {
            let path = dir.join("manifests").join("registry.ollama.ai").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                format!(
                    r#"{{"layers":[{{"mediaType":"application/vnd.ollama.image.license","digest":"sha256:aaa"}},
                       {{"mediaType":"application/vnd.ollama.image.model","digest":"sha256:{digest}"}}]}}"#
                ),
            )
            .unwrap();
        };
        manifest("library/gemma3/4b", "111");
        manifest("hf.co/qwen3/8b", "222");
        manifest("library/удалённая/latest", "333");
        big_file(&dir.join("blobs").join("sha256-111"));
        big_file(&dir.join("blobs").join("sha256-222"));

        let mut titles: Vec<String> = scan(&dir).into_iter().filter_map(|f| f.title).collect();
        titles.sort();
        assert_eq!(titles, ["gemma3:4b", "hf.co/qwen3:8b"]);
    }

    /// На этом ПК есть и LM Studio, и Ollama — смотрим, что находится.
    /// `cargo test scan::tests::real_sources -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_sources() {
        for s in sources(Path::new(r"D:\Ollivo")) {
            let found = scan(&s.dir);
            println!("{} — {} ({} шт.)", s.title, s.dir.display(), found.len());
            for f in found.iter().take(10) {
                println!("   {} {}", f.title.clone().unwrap_or_default(), f.path.display());
            }
        }
    }
}
