import { useSyncExternalStore } from "react";

// Язык окна: русский или английский. Строки пишутся парой рядом: t("Отмена", "Cancel").
// Языков два и третьего не видно — пара на месте надёжнее словаря по ключам: перевод
// нельзя забыть, при правке видны обе строки. То же устройство — в ядре (`i18n.rs`).
// Язык хранит ядро (`settings.language`), окно только берёт его при запуске и меняет
// из настроек — тогда ядро тоже начинает говорить на новом языке.

export type Lang = "ru" | "en";

let lang: Lang = "ru";
const listeners = new Set<() => void>();

export function getLang(): Lang {
  return lang;
}

export function setLang(next: Lang) {
  if (next === lang) return;
  lang = next;
  document.documentElement.lang = next;
  listeners.forEach((f) => f());
}

function subscribe(f: () => void) {
  listeners.add(f);
  return () => listeners.delete(f);
}

/** Текущий язык; компонент перерисуется, когда его сменят. Достаточно вызвать в корне:
 *  React перерисует всё дерево вслед за ним. */
export function useLang(): Lang {
  return useSyncExternalStore(subscribe, getLang);
}

/** Строка на языке окна. */
export function t(ru: string, en: string): string {
  return lang === "en" ? en : ru;
}

/** Форма слова для числа: по-русски три («1 слово, 2 слова, 5 слов»), по-английски две. */
export function pl(n: number, ru: [string, string, string], en: [string, string]): string {
  if (lang === "en") return n === 1 ? en[0] : en[1];
  const d = n % 10, h = n % 100;
  if (d === 1 && h !== 11) return ru[0];
  if (d >= 2 && d <= 4 && (h < 12 || h > 14)) return ru[1];
  return ru[2];
}

/** Для `toLocaleString` и дат. */
export function locale(): string {
  return lang === "en" ? "en-US" : "ru-RU";
}

/** Дробь: «1,5» по-русски, «1.5» по-английски. */
export function decimal(x: number, digits = 1): string {
  const s = x.toFixed(digits);
  return lang === "en" ? s : s.replace(".", ",");
}
