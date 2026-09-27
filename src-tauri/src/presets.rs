//! Пресеты вместо параметров: роль (за ней — системный промпт) и манера ответа
//! «Точнее ↔ Креативнее» (за ней — температура и top-p).
//!
//! Числа и промпты живут только здесь: окно получает названия и пояснения
//! и присылает обратно id. Так простой режим не видит ни одного числа,
//! а «Профи» позже сможет перебить их своими.

use crate::llm::Msg;
use serde::Serialize;

/// Строки пресетов — парой «русский, английский»: `[ru, en]`.
type Pair = [&'static str; 2];

fn pick(p: &Pair) -> &'static str {
    if crate::i18n::en() { p[1] } else { p[0] }
}

#[derive(Debug, Clone)]
pub struct Role {
    pub id: &'static str,
    pub name: Pair,
    /// Одна фраза для подсказки: что изменится.
    pub hint: Pair,
    /// Манера, которая этой роли подходит лучше: её ставим, когда роль выбрали.
    pub style: &'static str,
    /// Системный промпт; `{target}` — язык, на который переводить (см. `prepare`).
    /// Промпт — на языке окна: спрашивающему по-английски русский промпт тянул бы
    /// ответ на русский.
    pub prompt: Pair,
    /// Запретить движку иероглифы (`NO_CJK`). Только там, где ответ заведомо
    /// русский или английский: помощника могут прямо попросить написать по-японски.
    pub no_cjk: bool,
}

impl Role {
    pub fn prompt(&self) -> &'static str {
        pick(&self.prompt)
    }
}

/// Грамматика llama.cpp: любой текст, кроме китайских, японских и корейских знаков.
/// Qwen2.5 3B вставляет китайские слова в русский перевод даже при температуре 0
/// («Кошка спит на长沙发»): 2 из 8 переводов без грамматики, 0 из 8 с ней,
/// скорость 57 ток/с против 61 (GTX 1080).
pub const NO_CJK: &str = r"root ::= [^\u3000-\u9fff\uac00-\ud7af\uff00-\uffef]*";

#[derive(Debug, Clone)]
pub struct Style {
    pub id: &'static str,
    pub name: Pair,
    pub hint: Pair,
    pub temperature: f32,
    pub top_p: f32,
}

/// Помощник — без особых указаний: модели и так обучены быть помощником,
/// лишний промпт только съедает контекст. Язык ответа — язык вопроса:
/// маленькие модели иначе нередко сбиваются на английский или китайский.
pub const ROLES: &[Role] = &[
    Role {
        id: "helper",
        name: ["Помощник", "Assistant"],
        hint: ["Отвечает на любые вопросы", "Answers any questions"],
        style: "balanced",
        prompt: [
            "Отвечай на том языке, на котором задан вопрос.",
            "Answer in the language the question is asked in.",
        ],
        no_cjk: false,
    },
    Role {
        id: "translator",
        name: ["Переводчик", "Translator"],
        hint: [
            "Переводит присланный текст: русский — на английский, остальное — на русский",
            "Translates the text you send: Russian into English, anything else into Russian",
        ],
        style: "precise",
        prompt: [
            "Ты переводчик. Переведи сообщение пользователя на {target} язык. \
             Пиши только перевод, без пояснений и кавычек, сохраняя смысл, тон и абзацы.",
            "You are a translator. Translate the user's message into {target}. \
             Write only the translation, without explanations or quotes, keeping the meaning, tone and paragraphs.",
        ],
        no_cjk: true,
    },
    Role {
        id: "coder",
        name: ["Программист", "Programmer"],
        hint: ["Пишет и объясняет код", "Writes and explains code"],
        style: "precise",
        prompt: [
            "Ты опытный программист. Давай рабочий код в блоках с указанием языка, \
             коротко объясняй, почему сделано так. Если в вопросе не хватает данных — \
             спроси, а не придумывай. Отвечай на том языке, на котором задан вопрос.",
            "You are an experienced programmer. Give working code in blocks with the language named, \
             briefly explain why it is done this way. If the question lacks details, \
             ask instead of making things up. Answer in the language the question is asked in.",
        ],
        no_cjk: false,
    },
];

/// Середина 0,7 / 0,8 — рекомендация Qwen для обычного (не «думающего») режима,
/// творческий край 1,0 / 0,95 — рекомендация Gemma. Ниже 0,3 маленькие модели
/// начинают повторяться по кругу, выше 1,0 — терять нить, поэтому края не дальше.
pub const STYLES: &[Style] = &[
    Style {
        id: "precise",
        name: ["Точнее", "Precise"],
        hint: ["Сухо и по делу, меньше выдумок", "Dry and to the point, less made up"],
        temperature: 0.3,
        top_p: 0.8,
    },
    Style {
        id: "balanced",
        name: ["Обычно", "Balanced"],
        hint: ["Подходит для большинства вопросов", "Suits most questions"],
        temperature: 0.7,
        top_p: 0.8,
    },
    Style {
        id: "creative",
        name: ["Креативнее", "Creative"],
        hint: ["Живее и разнообразнее, но чаще ошибается", "Livelier and more varied, but makes more mistakes"],
        temperature: 1.0,
        top_p: 0.95,
    },
];

/// Что уходит модели: системный промпт роли первой репликой, дальше разговор.
///
/// Переводчику — только последний текст, и указание прямо в нём. Направление решаем
/// здесь, а не просим модель: условие «с русского — на английский, иначе — на русский»
/// Qwen2.5 3B не выполняла (`llm::tests::real_roles`). А с историей 0.5B повторяла
/// английский текст как есть, копируя прошлую пару «русский → английский»: переводу
/// история не нужна, а маленькие модели указание в самом сообщении слушают лучше.
pub fn prepare(role: &Role, messages: &[Msg]) -> Vec<Msg> {
    let system = |content: String| Msg::new("system", content);
    if !role.prompt().contains("{target}") {
        return std::iter::once(system(role.prompt().into())).chain(messages.iter().cloned()).collect();
    }
    // Приложенный документ переводчик переводит вместе с вопросом.
    let last = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map_or(String::new(), |m| crate::attach::for_model(&m.files, &m.content));
    let last = last.as_str();
    let target = if mostly_cyrillic(last) { t!("английский", "English") } else { t!("русский", "Russian") };
    vec![
        system(role.prompt().replace("{target}", target)),
        Msg::new("user", tf!("Переведи на {target} язык:\n\n{last}", "Translate into {target}:\n\n{last}")),
    ]
}

/// Кириллицы среди букв больше половины. Вкрапления вроде «Python» или кода не мешают.
fn mostly_cyrillic(text: &str) -> bool {
    let (mut cyr, mut all) = (0, 0);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        all += 1;
        if matches!(c as u32, 0x400..=0x4ff) {
            cyr += 1;
        }
    }
    cyr * 2 > all
}

/// Неизвестный или пустой id (старый разговор, пресет убрали) — помощник.
pub fn role(id: &str) -> &'static Role {
    ROLES.iter().find(|r| r.id == id).unwrap_or(&ROLES[0])
}

/// Неизвестный или пустой id — «Обычно».
pub fn style(id: &str) -> &'static Style {
    STYLES.iter().find(|s| s.id == id).unwrap_or(&STYLES[1])
}

/// Роль или манера для окна: id, название и пояснение на языке окна.
/// Промпты и числа в окно не уходят.
#[derive(Serialize)]
pub struct Choice {
    pub id: &'static str,
    pub name: &'static str,
    pub hint: &'static str,
    /// У роли — манера, которую ставить вместе с ней; у манеры поля нет.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<&'static str>,
}

#[derive(Serialize)]
pub struct All {
    pub roles: Vec<Choice>,
    pub styles: Vec<Choice>,
}

pub fn all() -> All {
    All {
        roles: ROLES
            .iter()
            .map(|r| Choice { id: r.id, name: pick(&r.name), hint: pick(&r.hint), style: Some(r.style) })
            .collect(),
        styles: STYLES.iter().map(|s| Choice { id: s.id, name: pick(&s.name), hint: pick(&s.hint), style: None }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_falls_back_to_defaults() {
        assert_eq!(role("").id, "helper");
        assert_eq!(role("пират").id, "helper");
        assert_eq!(style("").id, "balanced");
        assert_eq!(style("coder").temperature, 0.7);
    }

    #[test]
    fn roles_point_to_real_styles_and_ids_are_unique() {
        for r in ROLES {
            assert!(STYLES.iter().any(|s| s.id == r.style), "{}", r.id);
            assert_eq!(ROLES.iter().filter(|x| x.id == r.id).count(), 1);
        }
        // Слева направо — от точного к творческому: так нарисован переключатель.
        assert!(STYLES.windows(2).all(|w| w[0].temperature < w[1].temperature));
    }

    fn msg(role: &str, t: &str) -> Msg {
        Msg::new(role, t.into())
    }

    #[test]
    fn translator_picks_direction_and_drops_history() {
        let tr = role("translator");
        let to = |t: &str| prepare(tr, &[msg("user", t)])[1].content.clone();
        assert!(to("Как дела? Пишу на Python").starts_with("Переведи на английский"));
        assert!(to("The cat is sleeping").starts_with("Переведи на русский"));
        assert!(to("Der Hund schläft").starts_with("Переведи на русский"));
        // Прошлая пара «русский → английский» модели не показывается.
        let talk = [msg("user", "Доброе утро"), msg("assistant", "Good morning"), msg("user", "Nice weather")];
        let sent = prepare(tr, &talk);
        assert_eq!(sent.len(), 2);
        assert!(sent[0].content.contains("на русский") && sent[1].content.ends_with("Nice weather"));
    }

    /// В грамматике — экранированные коды: сами знаки в коде не прочесть,
    /// а U+3000 и U+FFEF ещё и невидимы.
    #[test]
    fn no_cjk_grammar_is_ascii() {
        assert!(NO_CJK.is_ascii());
        assert!(role("translator").no_cjk && !role("helper").no_cjk);
    }

    #[test]
    fn other_roles_get_prompt_then_whole_talk() {
        let talk = [msg("user", "a"), msg("assistant", "b"), msg("user", "c")];
        let sent = prepare(role("coder"), &talk);
        assert_eq!(sent.len(), 4);
        assert_eq!((sent[0].role.as_str(), sent[0].content.as_str()), ("system", role("coder").prompt()));
    }

    /// Промпты и числа в окно не уходят — только названия и пояснения.
    #[test]
    fn window_sees_no_numbers() {
        let json = serde_json::to_string(&all()).unwrap();
        assert!(!json.contains("temperature") && !json.contains("prompt") && !json.contains("Ты "));
        assert!(json.contains("Помощник"));
    }

    #[test]
    fn english_names_and_prompts() {
        crate::i18n::test_en();
        assert!(serde_json::to_string(&all()).unwrap().contains("Translator"));
        let tr = role("translator");
        let sent = prepare(tr, &[msg("user", "Как дела?")]);
        assert!(sent[0].content.contains("into English") && sent[1].content.starts_with("Translate into English"));
        assert!(prepare(tr, &[msg("user", "The cat")])[1].content.starts_with("Translate into Russian"));
    }
}
