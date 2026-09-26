//! Зрение текстовой модели: дополнение `mmproj`, которое превращает картинку
//! в понятные модели «слова». Лежит отдельным файлом рядом с моделью — так его
//! кладут и HuggingFace-репозитории, и LM Studio.

use crate::gguf;
use std::path::{Path, PathBuf};

/// Дополнение рядом с моделью, которое ей подходит. Подходит — если размер его выхода
/// равен ширине модели (`embedding_length`): картинка должна стать «словами» той же длины,
/// что и текст. С чужим дополнением llama-server не запустится, а в общей папке
/// (`models\`, папка LM Studio) их может лежать несколько от разных моделей.
pub fn find_projector(model: &Path) -> Option<PathBuf> {
    let embd = gguf::read(model).ok()?.arch_int("embedding_length")?;
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(model.parent()?)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.contains("mmproj") && name.ends_with(".gguf")
        })
        .collect();
    // F16 раньше F32 и BF16: вдвое меньше и работает на любой видеокарте
    // (BF16 у GTX 10xx нет).
    candidates.sort_by_key(|p| {
        let name = p.to_string_lossy().to_lowercase();
        (!name.contains("f16") || name.contains("bf16"), name)
    });
    candidates.into_iter().find(|p| projection_dim(p) == Some(embd))
}

/// Размер выхода дополнения; у дополнений только для звука — звуковой.
fn projection_dim(path: &Path) -> Option<u64> {
    let g = gguf::read(path).ok()?;
    g.int("clip.vision.projection_dim").or_else(|| g.int("clip.audio.projection_dim"))
}

/// Имя файла зрения для репозитория HF: F16, если есть, — по той же причине, что выше.
pub fn pick_from_repo<'a>(files: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut mm: Vec<&str> = files
        .into_iter()
        .filter(|f| {
            let l = f.to_lowercase();
            !l.contains('/') && l.contains("mmproj") && l.ends_with(".gguf")
        })
        .collect();
    mm.sort_by_key(|f| {
        let l = f.to_lowercase();
        (if l.contains("bf16") { 2 } else if l.contains("f16") { 0 } else if l.contains("q8") { 1 } else { 3 }, l)
    });
    mm.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_prefers_f16() {
        let files = ["Qwen3.5-2B-Q4_K_M.gguf", "mmproj-BF16.gguf", "mmproj-F32.gguf", "mmproj-F16.gguf"];
        assert_eq!(pick_from_repo(files), Some("mmproj-F16.gguf"));
        let gemma = ["gemma-4-E4B-it-Q4_K_M.gguf", "mmproj-gemma-4-E4B-it-BF16.gguf", "mmproj-gemma-4-E4B-it-Q8_0.gguf"];
        assert_eq!(pick_from_repo(gemma), Some("mmproj-gemma-4-E4B-it-Q8_0.gguf"));
        assert_eq!(pick_from_repo(["model.gguf"]), None);
        // Во вложенной папке — не наш файл: качаем только из корня репозитория.
        assert_eq!(pick_from_repo(["old/mmproj-F16.gguf"]), None);
    }

    /// Настоящая модель со зрением из каталога: дополнение рядом находится и подходит.
    /// `cargo test vision::tests::real_projector -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_projector() {
        let model = Path::new(r"D:\Ollivo\models\unsloth\Qwen3.5-2B-GGUF\Qwen3.5-2B-Q4_K_M.gguf");
        let found = find_projector(model);
        println!("{found:?}");
        assert!(found.is_some_and(|p| p.ends_with("mmproj-F16.gguf")));
        // Текстовой модели без дополнения рядом — ничего.
        assert_eq!(find_projector(Path::new(r"D:\Ollivo\models\qwen2.5-0.5b-instruct-q4_k_m.gguf")), None);
    }
}
