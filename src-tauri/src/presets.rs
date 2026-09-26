//! Пресеты вместо параметров: роль (за ней — системный промпт) и манера ответа
//! «Точнее ↔ Креативнее» (за ней — температура и top-p).
//!
//! Числа и промпты живут только здесь: окно получает названия и пояснения
//! и присылает обратно id. Так простой режим не видит ни одного числа,
//! а «Профи» позже сможет перебить их своими.

use crate::llm::Msg;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Role {
    pub id: &'static str,
    pub name: &'static str,
    /// Одна фраза для подсказки: что изменится.
    pub hint: &'static str,
    /// Манера, которая этой роли подходит лучше: её ставим, когда роль выбрали.
    pub style: &'static str,
    /// Системный промпт; `{target}` — язык, на который переводить (см. `system`).
    #[serde(skip)]
    pub prompt: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Style {
    pub id: &'static str,
    pub name: &'static str,
    pub hint: &'static str,
    #[serde(skip)]
    pub temperature: f32,
    #[serde(skip)]
    pub top_p: f32,
}

/// Помощник — без особых указаний: модели и так обучены быть помощником,
/// лишний промпт только съедает контекст. Язык ответа — язык вопроса:
/// маленькие модели иначе нередко сбиваются на английский или китайский.
pub const ROLES: &[Role] = &[
    Role {
        id: "helper",
        name: "Помощник",
        hint: "Отвечает на любые вопросы",
        style: "balanced",
        prompt: "Отвечай на том языке, на котором задан вопрос.",
    },
    Role {
        id: "translator",
        name: "Переводчик",
        hint: "Переводит присланный текст: русский — на английский, остальное — на русский",
        style: "precise",
        prompt: "Ты переводчик. Переведи сообщение пользователя на {target} язык. \
                 Пиши только перевод, без пояснений и кавычек, сохраняя смысл, тон и абзацы.",
    },
    Role {
        id: "coder",
        name: "Программист",
        hint: "Пишет и объясняет код",
        style: "precise",
        prompt: "Ты опытный программист. Давай рабочий код в блоках с указанием языка, \
                 коротко объясняй, почему сделано так. Если в вопросе не хватает данных — \
                 спроси, а не придумывай. Отвечай на том языке, на котором задан вопрос.",
    },
];

/// Середина 0,7 / 0,8 — рекомендация Qwen для обычного (не «думающего») режима,
/// творческий край 1,0 / 0,95 — рекомендация Gemma. Ниже 0,3 маленькие модели
/// начинают повторяться по кругу, выше 1,0 — терять нить, поэтому края не дальше.
pub const STYLES: &[Style] = &[
    Style { id: "precise", name: "Точнее", hint: "Сухо и по делу, меньше выдумок", temperature: 0.3, top_p: 0.8 },
    Style { id: "balanced", name: "Обычно", hint: "Подходит для большинства вопросов", temperature: 0.7, top_p: 0.8 },
    Style { id: "creative", name: "Креативнее", hint: "Живее и разнообразнее, но чаще ошибается", temperature: 1.0, top_p: 0.95 },
];

/// Системный промпт для этого запроса. Направление перевода решаем здесь, а не просим
/// модель: условие «с русского — на английский, иначе — на русский» Qwen2.5 3B не
/// выполняет и возвращает английский текст как есть (`llm::tests::real_roles`).
pub fn system(role: &Role, messages: &[Msg]) -> String {
    if !role.prompt.contains("{target}") {
        return role.prompt.into();
    }
    let last = messages.iter().rev().find(|m| m.role == "user").map_or("", |m| m.content.as_str());
    let target = if mostly_cyrillic(last) { "английский" } else { "русский" };
    role.prompt.replace("{target}", target)
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

#[derive(Serialize)]
pub struct All {
    pub roles: &'static [Role],
    pub styles: &'static [Style],
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

    #[test]
    fn translator_picks_direction() {
        let msg = |t: &str| vec![Msg { role: "user".into(), content: t.into() }];
        let tr = role("translator");
        assert!(system(tr, &msg("Как дела? Пишу на Python")).contains("на английский"));
        assert!(system(tr, &msg("The cat is sleeping")).contains("на русский"));
        assert!(system(tr, &msg("Der Hund schläft")).contains("на русский"));
        // Остальные роли — промпт как есть.
        assert_eq!(system(role("coder"), &msg("x")), role("coder").prompt);
    }

    /// Промпты и числа в окно не уходят — только названия и пояснения.
    #[test]
    fn window_sees_no_numbers() {
        let json = serde_json::to_string(&All { roles: ROLES, styles: STYLES }).unwrap();
        assert!(!json.contains("temperature") && !json.contains("prompt"));
    }
}
