// Токены человеку ничего не говорят — в окне только слова и страницы.
// Числа — из замера в `probe.rs` (там же, где прогноз скорости в каталоге):
// токенизатор Qwen2.5, русский текст — 2,4 токена на слово, 2,7 знака на токен.

/** Страница — ~1800 знаков: 1800 / 2,7 ≈ 650 токенов. */
const TOKENS_PER_PAGE = 650;

/** «1 слово», «2 слова», «5 слов». */
export function plural(n: number, one: string, few: string, many: string) {
  const d = n % 10, h = n % 100;
  if (d === 1 && h !== 11) return one;
  if (d >= 2 && d <= 4 && (h < 12 || h > 14)) return few;
  return many;
}

/** «около 12 страниц» — сколько разговора модель держит в памяти. */
export function memoryPages(ctx: number): string {
  let p = Math.max(1, Math.floor(ctx / TOKENS_PER_PAGE));
  if (p > 20) p = Math.floor((p + 5) / 10) * 10;
  return `около ${p} ${plural(p, "страницы", "страниц", "страниц")}`;
}

/** «24 слова в секунду». Слова считаем по самому ответу, а не по среднему: так честнее
 *  и для английского, где слово — примерно токен, а не два с половиной. */
export function wordsPerSecond(text: string, tokens: number, tokensPerSec: number): string {
  const words = text.split(/\s+/).filter(Boolean).length;
  const w = tokens > 0 ? (tokensPerSec * words) / tokens : 0;
  if (w < 1) return "меньше слова в секунду";
  const n = Math.round(w);
  return `${n} ${plural(n, "слово", "слова", "слов")} в секунду`;
}
