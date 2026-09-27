import { openUrl } from "@tauri-apps/plugin-opener";
import { getLang } from "./i18n";

/**
 * Страница поддержки проекта — на сайте, а не внутри программы: кошельки и ссылки
 * меняются без выпуска новой версии. Когда сайт переедет на ollivo.ru, GitHub Pages
 * сам перенаправит этот адрес, и уже установленные копии не сломаются.
 */
export const SUPPORT_URL = "https://wufcorp.github.io/Ollivo/?lang=ru#support";
/** Английская страница сайта — в `site/en/`. `?lang=` сайт запоминает: язык программы
 *  известен точно, в отличие от догадки по языку браузера. */
export const SUPPORT_URL_EN = "https://wufcorp.github.io/Ollivo/en/?lang=en#support";

export function openSupport() {
  openUrl(getLang() === "en" ? SUPPORT_URL_EN : SUPPORT_URL).catch(() => {});
}
