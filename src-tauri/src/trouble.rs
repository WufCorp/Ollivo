//! Понятные ошибки: сырой текст движка → что случилось, что делать и кнопки.
//!
//! Движок пишет по-английски и для программистов («ErrorOutOfDeviceMemory»,
//! «data is not within the file bounds»). Человеку нужно другое: одна фраза, что
//! случилось, одна — что делать, и кнопка, которая это сделает. Сырой текст не
//! выбрасываем — он уходит в «Подробности» для того, кто будет помогать.
//!
//! Образцы строк — из настоящих логов llama-server b11081 (Vulkan, GTX 1080),
//! сбои вызывались нарочно: огромный контекст, обрезанный и испорченный файл,
//! подменённое семейство модели, длинный вопрос при маленькой памяти разговора.

use serde::Serialize;

/// Кнопка под ошибкой. Что она делает, знает окно; ядро только решает, какие уместны.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Попробовать то же самое ещё раз.
    Retry,
    /// Запустить модель экономнее: меньше памяти разговора и слоёв на видеокарте.
    Lighter,
    /// Открыть каталог — взять версию поменьше или другую модель.
    Catalog,
    /// Убрать файл из списка моделей.
    Forget,
    /// Запустить ту же модель заново.
    Restart,
    /// Начать новый разговор.
    NewChat,
    /// Установить или починить движок чата.
    Engine,
    /// Поставить компоненты Windows (VC++ Runtime).
    Vcredist,
    /// Перейти к списку моделей.
    Models,
}

#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    /// Что случилось — одна фраза.
    pub text: String,
    /// Что делать — одна фраза; `None`, если сказать нечего, кроме кнопок.
    pub hint: Option<String>,
    pub actions: Vec<Action>,
    /// Сырой текст — под «Подробности».
    pub details: String,
}

fn problem(text: &str, hint: Option<&str>, actions: &[Action], raw: &str) -> Problem {
    Problem {
        text: text.into(),
        hint: hint.map(Into::into),
        actions: actions.to_vec(),
        details: raw.trim().into(),
    }
}

/// Windows не нашла DLL (`STATUS_DLL_NOT_FOUND`, 0xC0000135) или она не той
/// разрядности (0xC000007B): у llama.cpp это почти всегда VC++ Runtime или vulkan-1.dll.
const DLL_CODES: [&str; 2] = ["-1073741515", "-1073741701"];

fn has(raw: &str, needles: &[&str]) -> bool {
    let low = raw.to_lowercase();
    needles.iter().any(|n| low.contains(n))
}

fn out_of_memory(raw: &str) -> bool {
    has(raw, &["outofdevicememory", "out of memory", "failed to allocate", "cudamalloc failed"])
}

/// Модель не запустилась. `can_lighter` — есть ли куда урезать (ступени не кончились).
pub fn start(raw: &str, can_lighter: bool) -> Problem {
    use Action::*;
    if raw.starts_with("файл модели не найден") {
        return problem(
            "Файла модели нет на месте.",
            Some("Его переместили или удалили, или отключён диск, на котором он лежит."),
            &[Forget],
            raw,
        );
    }
    if raw == "движок чата не установлен" {
        return problem(
            "Движок чата не установлен.",
            Some("Он скачивается один раз, это около 30 МБ."),
            &[Engine],
            raw,
        );
    }
    if DLL_CODES.iter().any(|c| raw.contains(c)) {
        return problem(
            "Не хватает компонентов Windows, нужных движку чата.",
            Some("Их ставит установщик Microsoft; Windows спросит разрешение."),
            &[Vcredist, Engine],
            raw,
        );
    }
    if out_of_memory(raw) {
        let hint = if can_lighter {
            "Запустите её экономнее — будет отвечать медленнее и помнить меньше, зато заработает. \
             Или возьмите версию поменьше."
        } else {
            "Экономнее уже некуда. Закройте игры и другие тяжёлые программы или возьмите версию поменьше."
        };
        let actions: &[Action] = if can_lighter { &[Lighter, Catalog] } else { &[Retry, Catalog] };
        return problem("Модели не хватило видеопамяти.", Some(hint), actions, raw);
    }
    if has(raw, &["not within the file bounds", "corrupted or incomplete"]) {
        return problem(
            "Файл модели повреждён или скачан не до конца.",
            Some("Скачайте её заново из каталога."),
            &[Catalog, Forget],
            raw,
        );
    }
    if has(raw, &["unknown model architecture"]) {
        return problem(
            "Движок чата пока не умеет запускать это семейство моделей.",
            Some("Поддержка появится с обновлением движка. Пока возьмите другую модель из каталога."),
            &[Catalog],
            raw,
        );
    }
    if has(raw, &["invalid magic", "failed to load model"]) {
        return problem(
            "Этот файл не получается прочитать как модель.",
            Some("Похоже, он испорчен или это вовсе не модель."),
            &[Forget, Catalog],
            raw,
        );
    }
    if has(raw, &["не ответил за"]) {
        return problem(
            "Модель грузится слишком долго.",
            Some("Так бывает с большими моделями на медленном диске. Попробуйте ещё раз или возьмите версию поменьше."),
            &[Retry, Catalog],
            raw,
        );
    }
    problem("Модель не запустилась.", Some("Попробуйте ещё раз."), &[Retry], raw)
}

/// Движок упал сам, когда модель уже работала.
pub fn crashed(raw: &str, can_lighter: bool) -> Problem {
    if out_of_memory(raw) {
        return start(raw, can_lighter);
    }
    problem(
        "Движок чата неожиданно закрылся.",
        Some("Переписка сохранена — запустите модель заново и продолжайте."),
        &[Action::Restart],
        raw,
    )
}

/// Не получилось получить ответ в чате.
pub fn chat(raw: &str) -> Problem {
    use Action::*;
    if has(raw, &["exceed_context_size", "exceeds the available context size"]) {
        return problem(
            "Разговор стал длиннее, чем модель может удержать в памяти.",
            Some("Начните новый разговор — старый останется в списке."),
            &[NewChat],
            raw,
        );
    }
    if raw == "модель не запущена" {
        return problem("Модель не запущена.", Some("Запустите её в списке моделей."), &[Models], raw);
    }
    if raw == "модель ещё загружается" {
        return problem("Модель ещё загружается.", Some("Подождите немного и спросите снова."), &[Retry], raw);
    }
    if has(raw, &["не отвечает", "оборвалась"]) {
        return problem(
            "Движок чата перестал отвечать.",
            Some("Переписка сохранена — запустите модель заново и повторите вопрос."),
            &[Restart],
            raw,
        );
    }
    problem("Не получилось получить ответ.", None, &[Retry], raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    const OOM: &str = "движок завершился при запуске (код Some(1))\n\
        ggml_vulkan: vk::Device::allocateMemory: ErrorOutOfDeviceMemory\n\
        0.03.390.378 E alloc_tensor_range: failed to allocate Vulkan0 buffer of size 1024196608\n\
        0.03.612.528 E srv  llama_server: exiting due to model loading error";
    const CUT: &str = "движок завершился при запуске (код Some(1))\n\
        E llama_model_load: error loading model: tensor 'token_embd.weight' data is not within the file bounds, \
        model is corrupted or incomplete\nE llama_model_load_from_file_impl: failed to load model";
    const JUNK: &str = "движок завершился при запуске (код Some(1))\n\
        E gguf_init_from_reader: invalid magic characters: 'a???', expected 'GGUF'\n\
        E llama_model_load: error loading model: llama_model_loader: failed to load model from junk.gguf";
    const ARCH: &str = "движок завершился при запуске (код Some(1))\n\
        E llama_model_load: error loading model: unknown model architecture: 'qwenZ'\n\
        E llama_model_load_from_file_impl: failed to load model";
    const CTX: &str = r#"движок ответил ошибкой 400: {"error":{"code":400,"message":"request (4030 tokens) exceeds the available context size (512 tokens), try increasing it","type":"exceed_context_size_error","n_prompt_tokens":4030,"n_ctx":512}}"#;

    #[test]
    fn engine_logs_become_plain_words() {
        assert_eq!(start(OOM, true).actions, [Lighter, Catalog]);
        assert!(start(OOM, true).text.contains("видеопамяти"));
        // Ступени кончились — «экономнее» не предлагаем, чтобы не гонять по кругу.
        assert!(!start(OOM, false).actions.contains(&Lighter));
        // Обрезанный файл — не «не модель»: у него тоже есть «failed to load model».
        assert!(start(CUT, true).text.contains("повреждён"));
        assert!(start(JUNK, true).text.contains("не получается прочитать"));
        assert!(start(ARCH, true).text.contains("семейство"));
        assert_eq!(start("движок не ответил за 300 с", true).actions, [Retry, Catalog]);
    }

    #[test]
    fn own_errors_are_recognized() {
        assert_eq!(start(r"файл модели не найден: D:\m.gguf", true).actions, [Forget]);
        assert_eq!(start("движок чата не установлен", true).actions, [Engine]);
        assert_eq!(start("движок завершился при запуске (код Some(-1073741515))\n", true).actions, [Vcredist, Engine]);
        assert_eq!(start("что-то совсем новое", true).actions, [Retry]);
    }

    #[test]
    fn chat_errors() {
        assert_eq!(chat(CTX).actions, [NewChat]);
        assert_eq!(chat("связь с движком оборвалась: reset").actions, [Restart]);
        assert_eq!(chat("модель не запущена").actions, [Models]);
        assert_eq!(chat("движок ответил ошибкой 500").actions, [Retry]);
        // Упал сам без нехватки памяти — перезапуск; с нехваткой — как при запуске.
        assert_eq!(crashed("движок чата упал (код Some(3))", true).actions, [Restart]);
        assert_eq!(crashed(OOM, true).actions, [Lighter, Catalog]);
    }

    /// Сырой текст не теряется: он нужен тому, кто будет помогать.
    #[test]
    fn details_keep_raw_text() {
        assert!(start(OOM, true).details.contains("ErrorOutOfDeviceMemory"));
    }
}
