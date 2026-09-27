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

/// Движка чата нет — ядро само передаёт это вместо текста ошибки.
pub const NO_ENGINE: &str = "no-engine";

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
    // Сырые строки ядра смотрим на обоих языках: язык могли сменить, пока модель грузилась.
    if raw.starts_with("файл модели не найден") || raw.starts_with("model file not found") {
        return problem(
            t!("Файла модели нет на месте.", "The model file is missing."),
            Some(t!(
                "Его переместили или удалили, или отключён диск, на котором он лежит.",
                "It was moved or deleted, or the disk it is on is disconnected."
            )),
            &[Forget],
            raw,
        );
    }
    if raw == NO_ENGINE {
        return problem(
            t!("Движок чата не установлен.", "The chat engine is not installed."),
            Some(t!("Он скачивается один раз, это около 30 МБ.", "It downloads once, about 30 MB.")),
            &[Engine],
            raw,
        );
    }
    if DLL_CODES.iter().any(|c| raw.contains(c)) {
        return problem(
            t!("Движку чата не хватает файлов.", "The chat engine is missing files."),
            // Библиотеки VC++ программа кладёт рядом с движком сама (`vcrt.rs`), так что
            // скорее побит сам движок: сначала «Починить», установщик Microsoft — запасной путь.
            Some(t!(
                "Обычно помогает «Починить» движок на странице «Компьютер». Не помогло — поставьте компоненты Microsoft, Windows спросит разрешение.",
                "Usually “Repair” for the engine on the “Computer” page helps. If not, install the Microsoft components — Windows will ask for permission."
            )),
            &[Engine, Vcredist],
            raw,
        );
    }
    if out_of_memory(raw) {
        let hint = if can_lighter {
            t!(
                "Запустите её экономнее — будет отвечать медленнее и помнить меньше, зато заработает. \
                 Или возьмите версию поменьше.",
                "Run it in a lighter mode — it will answer slower and remember less, but it will work. \
                 Or take a smaller version."
            )
        } else {
            t!(
                "Экономнее уже некуда. Закройте игры и другие тяжёлые программы или возьмите версию поменьше.",
                "It can't get any lighter. Close games and other heavy programs or take a smaller version."
            )
        };
        let actions: &[Action] = if can_lighter { &[Lighter, Catalog] } else { &[Retry, Catalog] };
        return problem(t!("Модели не хватило видеопамяти.", "The model ran out of video memory."), Some(hint), actions, raw);
    }
    if has(raw, &["not within the file bounds", "corrupted or incomplete"]) {
        return problem(
            t!("Файл модели повреждён или скачан не до конца.", "The model file is damaged or not fully downloaded."),
            Some(t!("Скачайте её заново из каталога.", "Download it again from the catalog.")),
            &[Catalog, Forget],
            raw,
        );
    }
    if has(raw, &["unknown model architecture"]) {
        return problem(
            t!(
                "Движок чата пока не умеет запускать это семейство моделей.",
                "The chat engine can't run this model family yet."
            ),
            Some(t!(
                "Поддержка появится с обновлением движка. Пока возьмите другую модель из каталога.",
                "Support will come with an engine update. For now, take another model from the catalog."
            )),
            &[Catalog],
            raw,
        );
    }
    if has(raw, &["invalid magic", "failed to load model"]) {
        return problem(
            t!("Этот файл не получается прочитать как модель.", "This file can't be read as a model."),
            Some(t!("Похоже, он испорчен или это вовсе не модель.", "It looks damaged, or it isn't a model at all.")),
            &[Forget, Catalog],
            raw,
        );
    }
    if has(raw, &["не ответил за", "did not respond within"]) {
        return problem(
            t!("Модель грузится слишком долго.", "The model takes too long to load."),
            Some(t!(
                "Так бывает с большими моделями на медленном диске. Попробуйте ещё раз или возьмите версию поменьше.",
                "This happens with big models on a slow disk. Try again or take a smaller version."
            )),
            &[Retry, Catalog],
            raw,
        );
    }
    problem(t!("Модель не запустилась.", "The model did not start."), Some(t!("Попробуйте ещё раз.", "Try again.")), &[Retry], raw)
}

/// Движок упал сам, когда модель уже работала.
pub fn crashed(raw: &str, can_lighter: bool) -> Problem {
    if out_of_memory(raw) {
        return start(raw, can_lighter);
    }
    problem(
        t!("Движок чата неожиданно закрылся.", "The chat engine closed unexpectedly."),
        Some(t!(
            "Переписка сохранена — запустите модель заново и продолжайте.",
            "The conversation is saved — start the model again and carry on."
        )),
        &[Action::Restart],
        raw,
    )
}

/// Не получилось получить ответ в чате.
pub fn chat(raw: &str) -> Problem {
    use Action::*;
    if has(raw, &["exceed_context_size", "exceeds the available context size"]) {
        return problem(
            t!(
                "Разговор стал длиннее, чем модель может удержать в памяти.",
                "The conversation got longer than the model can keep in memory."
            ),
            Some(t!("Начните новый разговор — старый останется в списке.", "Start a new conversation — the old one stays in the list.")),
            &[NewChat],
            raw,
        );
    }
    if raw == "модель не запущена" || raw == "the model is not running" {
        return problem(
            t!("Модель не запущена.", "The model is not running."),
            Some(t!("Запустите её в списке моделей.", "Start it in the model list.")),
            &[Models],
            raw,
        );
    }
    if raw == "модель ещё загружается" || raw == "the model is still loading" {
        return problem(
            t!("Модель ещё загружается.", "The model is still loading."),
            Some(t!("Подождите немного и спросите снова.", "Wait a little and ask again.")),
            &[Retry],
            raw,
        );
    }
    if has(raw, &["не отвечает", "оборвалась", "not responding", "connection to the engine was lost"]) {
        return problem(
            t!("Движок чата перестал отвечать.", "The chat engine stopped responding."),
            Some(t!(
                "Переписка сохранена — запустите модель заново и повторите вопрос.",
                "The conversation is saved — start the model again and repeat the question."
            )),
            &[Restart],
            raw,
        );
    }
    problem(t!("Не получилось получить ответ.", "Couldn't get an answer."), None, &[Retry], raw)
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
        assert_eq!(start(NO_ENGINE, true).actions, [Engine]);
        assert_eq!(start("движок завершился при запуске (код Some(-1073741515))\n", true).actions, [Engine, Vcredist]);
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

    /// По-английски — те же кнопки, а свои сырые строки узнаются на обоих языках.
    #[test]
    fn english() {
        crate::i18n::test_en();
        assert!(start(OOM, true).text.contains("video memory"));
        assert_eq!(start(r"model file not found: D:\m.gguf", true).actions, [Forget]);
        assert_eq!(start(r"файл модели не найден: D:\m.gguf", true).actions, [Forget]);
        assert_eq!(start("the engine did not respond within 300 s", true).actions, [Retry, Catalog]);
        assert_eq!(chat("the model is not running").actions, [Models]);
        assert_eq!(chat("connection to the engine was lost: reset").actions, [Restart]);
    }
}
