import MarkdownIt from "markdown-it";
import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Ответ модели с разметкой: заголовки, списки, таблицы, код.
 *
 * Свой HTML в ответе не рисуется (`html: false` — по умолчанию): модель может
 * прислать что угодно, а это окно программы, а не страница в браузере.
 * Одиночный перенос строки — перенос и на вид (`breaks`): модели так и пишут.
 */
const md = new MarkdownIt({ breaks: true, linkify: true });

/** Код — отдельной карточкой с языком и кнопкой «Копировать». */
md.renderer.rules.fence = (tokens, i) => {
  const t = tokens[i];
  const lang = md.utils.escapeHtml(t.info.trim().split(/\s+/)[0] ?? "");
  return (
    `<div class="code"><div class="code-top"><span>${lang}</span>` +
    `<button class="copy" type="button">Копировать</button></div>` +
    `<pre><code>${md.utils.escapeHtml(t.content)}</code></pre></div>`
  );
};

/** Кладёт текст в буфер обмена; `false` — не получилось, надо сказать человеку. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Старый webview без доступа к буферу — пробуем по-старому, через выделение.
    try {
      const box = document.createElement("textarea");
      box.value = text;
      box.style.position = "fixed";
      box.style.opacity = "0";
      document.body.append(box);
      box.select();
      const ok = document.execCommand("copy");
      box.remove();
      return ok;
    } catch {
      return false;
    }
  }
}

export default function Answer({ text }: { text: string }) {
  const click = async (e: React.MouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;

    const copy = target.closest("button.copy");
    if (copy) {
      const code = copy.closest(".code")?.querySelector("code")?.textContent ?? "";
      const ok = await copyText(code);
      copy.textContent = ok ? "Скопировано" : "Не вышло скопировать";
      setTimeout(() => (copy.textContent = "Копировать"), 2000);
      return;
    }

    // Ссылку из ответа открываем в обычном браузере: внутри программы страницам не место.
    const link = target.closest("a");
    if (link) {
      e.preventDefault();
      const href = link.getAttribute("href") ?? "";
      if (/^https?:/i.test(href)) {
        openUrl(href).catch(() => {});
      }
    }
  };

  return <div className="md" onClick={click} dangerouslySetInnerHTML={{ __html: md.render(text) }} />;
}
