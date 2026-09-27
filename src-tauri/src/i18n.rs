//! Язык программы: русский или английский.
//!
//! Строки ядра, которые видит человек или модель, пишутся парой рядом: `t!("…", "…")`.
//! Языков два и третьего пока не видно — пара на месте надёжнее словаря по ключам:
//! перевод нельзя забыть, при правке видны обе строки (решение 2026-09-27).
//!
//! Язык — глобальный, а не параметр каждой функции: ошибки и подсказки рождаются
//! глубоко в ядре, протаскивать язык через все вызовы ради двух значений — шум.
//! Меняется из настроек (`set`) и действует на всё, что ядро скажет после этого.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Ru,
    /// Сюда же незнакомый язык из будущей версии: английский поймут больше людей.
    #[serde(other)]
    En,
}

impl Lang {
    /// Язык интерфейса Windows: русский для русского, украинского, белорусского
    /// и казахского — в этих странах русский интерфейс привычнее английского;
    /// для остальных — английский.
    pub fn system() -> Self {
        // PRIMARYLANGID: ru 0x19, uk 0x22, be 0x23, kk 0x3F.
        match system_langid() & 0x3ff {
            0x19 | 0x22 | 0x23 | 0x3f => Lang::Ru,
            _ => Lang::En,
        }
    }
}

#[cfg(windows)]
fn system_langid() -> u16 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // SAFETY: функция без аргументов, только читает настройку пользователя.
    unsafe { GetUserDefaultUILanguage() }
}

#[cfg(not(windows))]
fn system_langid() -> u16 {
    0x19
}

static EN: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    /// Язык для одного теста: тесты идут параллельно, общий флаг они бы перебивали.
    static TEST: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

pub fn set(lang: Lang) {
    EN.store(lang == Lang::En, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if en() { Lang::En } else { Lang::Ru }
}

pub fn en() -> bool {
    #[cfg(test)]
    if let Some(v) = TEST.with(|t| t.get()) {
        return v;
    }
    EN.load(Ordering::Relaxed)
}

/// Английский в текущем потоке теста — до конца теста.
#[cfg(test)]
pub fn test_en() {
    TEST.with(|t| t.set(Some(true)));
}

/// Строка на языке программы.
/// `t!("Отмена", "Cancel")` — `&'static str`;
/// `t!("{n} файлов", "{n} files")` с подстановкой — `String` (второй вид, с `f`).
macro_rules! t {
    ($ru:literal, $en:literal $(,)?) => {
        if $crate::i18n::en() { $en } else { $ru }
    };
}

/// Как `t!`, но через `format!`: `tf!("около {p} страниц", "about {p} pages")`.
macro_rules! tf {
    ($ru:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::en() { format!($en $(, $arg)*) } else { format!($ru $(, $arg)*) }
    };
}

/// Форма слова для числа. По-русски три («1 слово, 2 слова, 5 слов»), по-английски две.
pub fn plural<'a>(n: u64, ru: [&'a str; 3], eng: [&'a str; 2]) -> &'a str {
    if en() {
        return if n == 1 { eng[0] } else { eng[1] };
    }
    let (d, h) = (n % 10, n % 100);
    if d == 1 && h != 11 {
        ru[0]
    } else if (2..=4).contains(&d) && !(12..=14).contains(&h) {
        ru[1]
    } else {
        ru[2]
    }
}

/// Десятичная дробь: по-русски запятая, по-английски точка.
pub fn decimal(s: String) -> String {
    if en() { s } else { s.replace('.', ",") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_follow_language() {
        assert_eq!(t!("Отмена", "Cancel"), "Отмена");
        let n = 5;
        assert_eq!(tf!("{n} файлов", "{n} files"), "5 файлов");
        assert_eq!(plural(21, ["слово", "слова", "слов"], ["word", "words"]), "слово");
        assert_eq!(plural(12, ["слово", "слова", "слов"], ["word", "words"]), "слов");
        assert_eq!(decimal("1.5".into()), "1,5");
        test_en();
        assert_eq!(t!("Отмена", "Cancel"), "Cancel");
        assert_eq!(tf!("{n} файлов", "{n} files"), "5 files");
        assert_eq!(plural(1, ["слово", "слова", "слов"], ["word", "words"]), "word");
        assert_eq!(plural(21, ["слово", "слова", "слов"], ["word", "words"]), "words");
        assert_eq!(decimal("1.5".into()), "1.5");
    }

    #[test]
    fn unknown_language_is_english() {
        assert_eq!(serde_json::from_str::<Lang>(r#""de""#).unwrap(), Lang::En);
        assert_eq!(serde_json::from_str::<Lang>(r#""ru""#).unwrap(), Lang::Ru);
    }
}
