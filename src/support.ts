import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Страница поддержки проекта — на сайте, а не внутри программы: кошельки и ссылки
 * меняются без выпуска новой версии. Когда сайт переедет на ollivo.ru, GitHub Pages
 * сам перенаправит этот адрес, и уже установленные копии не сломаются.
 */
export const SUPPORT_URL = "https://wufcorp.github.io/Ollivo/#support";

export function openSupport() {
  openUrl(SUPPORT_URL).catch(() => {});
}
