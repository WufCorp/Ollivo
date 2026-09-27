import { getLang, pl, t } from "./i18n";

// Токены человеку ничего не говорят — в окне только слова и страницы.
// Числа — из замера в `probe.rs` (там же, где прогноз скорости в каталоге):
// токенизатор Qwen2.5, русский текст — 2,4 токена на слово, 2,7 знака на токен;
// английский — 1,2 токена на слово.

/** Страница — ~1800 знаков. По-русски 1800 / 2,7 ≈ 650 токенов; по-английски ~300 слов × 1,2 ≈ 360.
 *  Считаем по языку окна: на нём человек, скорее всего, и пишет. Те же числа — в `probe.rs`. */
const tokensPerPage = () => (getLang() === "en" ? 360 : 650);

/** Сколько это страниц — числом, для «5 из 12 страниц». Округление то же, что у `memoryPages`,
 *  иначе панель и строка состояния назовут разное число. Начатый разговор — не меньше страницы:
 *  «0 из 12» после первого вопроса выглядит как ошибка. */
export function pageCount(tokens: number): number {
  if (tokens <= 0) return 0;
  const p = Math.max(1, Math.floor(tokens / tokensPerPage()));
  return p > 20 ? Math.floor((p + 5) / 10) * 10 : p;
}

/** «12 страниц» — число со словом, без «около». */
export function pages(n: number): string {
  return `${n} ${pl(n, ["страница", "страницы", "страниц"], ["page", "pages"])}`;
}

/** «около 12 страниц» — сколько разговора модель держит в памяти. */
export function memoryPages(ctx: number): string {
  let p = Math.max(1, Math.floor(ctx / tokensPerPage()));
  if (p > 20) p = Math.floor((p + 5) / 10) * 10;
  return t(
    `около ${p} ${pl(p, ["страницы", "страниц", "страниц"], ["page", "pages"])}`,
    `about ${p} ${pl(p, ["", "", ""], ["page", "pages"])}`,
  );
}

/** «24 слова в секунду». Слова считаем по самому ответу, а не по среднему: так честнее
 *  и для английского, где слово — примерно токен, а не два с половиной. */
export function wordsPerSecond(text: string, tokens: number, tokensPerSec: number): string {
  const words = text.split(/\s+/).filter(Boolean).length;
  const w = tokens > 0 ? (tokensPerSec * words) / tokens : 0;
  if (w < 1) return t("меньше слова в секунду", "less than a word per second");
  const n = Math.round(w);
  return t(
    `${n} ${pl(n, ["слово", "слова", "слов"], ["", ""])} в секунду`,
    `${n} ${pl(n, ["", "", ""], ["word", "words"])} per second`,
  );
}
