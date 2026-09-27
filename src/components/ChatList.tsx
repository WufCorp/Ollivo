import { Fragment, useEffect, useState } from "react";
import { chatsList, chatsRemove, chatsSearch, type ChatHit } from "../api";
import Icon from "./Icon";
import { t } from "../i18n";

/** Пауза после последней буквы: поиск читает все разговоры, не надо на каждую букву. */
const SEARCH_DELAY_MS = 250;

/**
 * Разговоры в боковом меню: новые сверху, над ними поиск.
 * `refresh` меняется, когда разговор сохранился, — это повод перечитать список.
 */
export default function ChatList({
  current,
  refresh,
  onPick,
  onNew,
  onRemoved,
}: {
  current: string | null;
  refresh: number;
  onPick: (id: string) => void;
  onNew: () => void;
  onRemoved: (id: string) => void;
}) {
  const [items, setItems] = useState<ChatHit[]>([]);
  const [query, setQuery] = useState("");
  const q = query.trim();

  useEffect(() => {
    let stale = false;
    const load = () =>
      (q ? chatsSearch(q) : chatsList().then((list) => list.map((c) => ({ ...c, snippet: null })))).then(
        (found) => !stale && setItems(found),
      );
    const timer = setTimeout(load, q ? SEARCH_DELAY_MS : 0);
    // Ответ на старый запрос мог прийти позже нового — его выбрасываем.
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [q, refresh]);

  const remove = async (id: string) => {
    await chatsRemove(id);
    setItems((prev) => prev.filter((c) => c.id !== id));
    onRemoved(id);
  };

  const words = q.split(/\s+/).filter(Boolean);
  // В поиске важнее совпадение, чем дата, — там без разбивки по дням.
  const day = (c: ChatHit) => (q ? null : dayOf(c.updated));

  return (
    <div className="chats">
      <div className="chats-head">
        <h2>{t("Разговоры", "Conversations")}</h2>
        <button className="icon-button accent" title={t("Новый разговор", "New conversation")} onClick={onNew}>
          <Icon name="plus" />
        </button>
      </div>
      <label className="chat-search">
        <Icon name="search" size={15} />
        <input
          type="search"
          placeholder={t("Найти в разговорах", "Search conversations")}
          aria-label={t("Найти в разговорах", "Search conversations")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setQuery("")}
        />
      </label>
      {q && items.length === 0 && <p className="muted chat-none">{t("Ничего не нашлось", "Nothing found")}</p>}
      {items.map((c, i) => (
        <Fragment key={c.id}>
          {day(c) && day(c) !== (i > 0 ? day(items[i - 1]) : null) && <p className="chat-day">{day(c)}</p>}
          <div className={c.id === current ? "chat-row active" : "chat-row"}>
            <button className="tab" title={c.title} onClick={() => onPick(c.id)}>
              <span className="chat-title">
                <Marked text={c.title} words={words} />
              </span>
              {/* Вопрос и есть название — второй раз его не показываем. */}
              {c.snippet && c.snippet !== c.title && (
                <span className="chat-snippet">
                  <Marked text={c.snippet} words={words} />
                </span>
              )}
            </button>
            <button className="forget" title={t("Удалить разговор", "Delete conversation")} onClick={() => remove(c.id)}>
              <Icon name="close" size={14} />
            </button>
          </div>
        </Fragment>
      ))}
    </div>
  );
}

/** «Сегодня», «Вчера», «На этой неделе», «Раньше» — как люди вспоминают разговоры. */
function dayOf(updated: number): string {
  const midnight = new Date();
  midnight.setHours(0, 0, 0, 0);
  const days = Math.floor((midnight.getTime() / 1000 - updated) / 86400) + 1;
  if (updated * 1000 >= midnight.getTime()) return t("Сегодня", "Today");
  if (days <= 1) return t("Вчера", "Yesterday");
  if (days < 7) return t("На этой неделе", "This week");
  return t("Раньше", "Earlier");
}

/** Строчные и «е» вместо «ё», символ в символ — чтобы места совпадений годились для исходного текста. */
function fold(text: string): string {
  let out = "";
  for (const ch of text) {
    const l = ch.toLowerCase();
    out += l.length === ch.length ? (l === "ё" ? "е" : l) : ch;
  }
  return out;
}

/** Текст с подсвеченными словами поиска. */
function Marked({ text, words }: { text: string; words: string[] }) {
  if (words.length === 0) return <>{text}</>;
  const hay = fold(text);
  const marked = new Array<boolean>(text.length).fill(false);
  for (const w of words.map(fold)) {
    for (let at = hay.indexOf(w); at >= 0; at = hay.indexOf(w, at + w.length)) {
      marked.fill(true, at, at + w.length);
    }
  }
  const parts: { text: string; mark: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const last = parts[parts.length - 1];
    if (last && last.mark === marked[i]) last.text += text[i];
    else parts.push({ text: text[i], mark: marked[i] });
  }
  return (
    <>
      {parts.map((p, i) => (
        <Fragment key={i}>{p.mark ? <mark>{p.text}</mark> : p.text}</Fragment>
      ))}
    </>
  );
}
