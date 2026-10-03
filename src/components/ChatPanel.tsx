import { useState } from "react";
import type { Attachment, ChatRole, FileMode, Listing, LlmState, Step } from "../api";
import { t } from "../i18n";
import { memoryPages, pageCount, pages } from "../words";
import Icon from "./Icon";
import { ModeSwitch, Steps } from "./Project";
import { whoComputes } from "./RunningModel";

export type PanelTab = "talk" | "files";

export const panelTabs = (): { id: PanelTab; name: string; icon: "gauge" | "folder" }[] => [
  { id: "talk", name: t("Разговор", "Conversation"), icon: "gauge" },
  { id: "files", name: t("Файлы", "Files"), icon: "folder" },
];

/** Сколько файлов папки рисуем за раз: в огромной папке их тысячи, дальше — поиском. */
const SHOWN = 300;

const modelName = (p: string) => (p.split(/[\\/]/).pop() ?? p).replace(/\.gguf$/i, "");

/**
 * Панель справа от чата: то, что раньше пряталось по углам. «Разговор» — сколько памяти
 * уже занято и какая модель; «Файлы» — вложения, папка проекта и что модель в ней меняла.
 */
export default function ChatPanel(p: {
  tab: PanelTab;
  onClose: () => void;
  llm: LlmState;
  /** Сколько памяти модели занял разговор вместе с ещё не отправленными файлами, в токенах. */
  used: number;
  /** Скорость последнего ответа словами. */
  lastSpeed: string | null;
  roles: ChatRole[];
  role: string;
  onRole: (id: string) => void;
  onGoToModels: () => void;
  onStop: () => void;
  answering: boolean;
  /** Файлы, уже отправленные в разговоре, и приложенные к следующему вопросу. */
  sent: Attachment[];
  pending: Attachment[];
  folder: string | null;
  listing: Listing | null;
  folderError: string | null;
  mode: FileMode;
  onMode: (m: FileMode) => void;
  onPickFolder: () => void;
  onDropFolder: () => void;
  /** Шаги модели по репликам: `line` — номер реплики, чтобы «Вернуть» знал, какую менять. */
  steps: { line: number; steps: Step[] }[];
  onUndo: (line: number, i: number) => Promise<void>;
  /** Приложить файл папки к вопросу — то же, что выбрать его после «@». */
  onAttach: (path: string) => void;
}) {
  return (
    <aside className="chat-panel" aria-label={t("Панель разговора", "Conversation panel")}>
      {/* Вкладки — кнопками в шапке разговора; здесь только название, чтобы не было двух переключателей. */}
      <div className="panel-head">
        <h3>{panelTabs().find((x) => x.id === p.tab)?.name}</h3>
        <button className="forget" title={t("Закрыть панель", "Close the panel")} onClick={p.onClose}>
          <Icon name="close" size={16} />
        </button>
      </div>
      <div className="panel-body">{p.tab === "talk" ? <Talk {...p} /> : <Files {...p} />}</div>
    </aside>
  );
}

function Talk(p: Parameters<typeof ChatPanel>[0]) {
  const { llm } = p;
  const ctx = llm.ctx;
  const k = ctx ? Math.min(1, p.used / ctx) : 0;
  // Жёлтая — пора думать о новом разговоре, красная — модель уже вот-вот начнёт забывать начало.
  const light = k < 0.6 ? "green" : k < 0.9 ? "yellow" : "red";
  const on = llm.state === "ready" || llm.state === "sleeping";

  return (
    <>
      <section className="panel-block">
        <h4>{t("Память разговора", "Conversation memory")}</h4>
        {ctx ? (
          <>
            <p className="panel-big">
              {pageCount(p.used)} <small>{t("из", "of")} {pages(pageCount(ctx))}</small>
            </p>
            <div className="bar" role="img" aria-label={t(`занято ${Math.round(k * 100)}%`, `${Math.round(k * 100)}% used`)}>
              <i className={light} style={{ transform: `scaleX(${k})` }} />
            </div>
            <p className="muted small">
              {k < 0.9
                ? t(
                    "Когда память заполнится, модель начнёт забывать начало разговора.",
                    "When the memory fills up, the model will start forgetting the beginning of the conversation.",
                  )
                : t(
                    "Память почти заполнена: модель начнёт забывать начало. Для новой темы лучше новый разговор.",
                    "The memory is almost full: the model will start forgetting the beginning. For a new topic, better start a new conversation.",
                  )}
            </p>
          </>
        ) : (
          <p className="muted small">
            {t(
              "Запустите модель — здесь будет видно, сколько разговора она помнит.",
              "Start a model — here you'll see how much of the conversation it remembers.",
            )}
          </p>
        )}
      </section>

      <section className="panel-block">
        <h4>{t("Модель", "Model")}</h4>
        {llm.model ? (
          <dl className="panel-facts">
            <dt title={llm.model}>{modelName(llm.model)}</dt>
            <dd>{on ? (llm.state === "ready" ? t("готова", "ready") : t("выгружена", "unloaded")) : t("не запущена", "not running")}</dd>
            {p.lastSpeed && (
              <>
                <dt>{t("Последний ответ", "Last answer")}</dt>
                <dd>{p.lastSpeed}</dd>
              </>
            )}
            {llm.state === "ready" && ctx !== null && (
              <>
                <dt>{t("Помнит", "Remembers")}</dt>
                <dd>{memoryPages(ctx)}</dd>
              </>
            )}
          </dl>
        ) : (
          <p className="muted small">{t("Модель не запущена.", "The model is not running.")}</p>
        )}
        {llm.state === "ready" && (
          <p className="muted small">{whoComputes(llm.gpu_layers, llm.layers, llm.remote)}.</p>
        )}
        <div className="actions">
          <button className="secondary" onClick={p.onGoToModels}>
            {t("Сменить модель", "Change model")}
          </button>
          {llm.state === "ready" && (
            <button className="secondary" disabled={p.answering} onClick={p.onStop}>
              {llm.remote ? t("Отключить", "Disconnect") : t("Выгрузить", "Unload")}
            </button>
          )}
        </div>
      </section>

      <section className="panel-block">
        <h4>{t("Кто отвечает", "Who answers")}</h4>
        <div className="roles" role="radiogroup" aria-label={t("Роль модели", "Model role")}>
          {p.roles.map((r) => (
            <button
              key={r.id}
              role="radio"
              aria-checked={r.id === p.role}
              className={r.id === p.role ? "role-card active" : "role-card"}
              disabled={p.answering}
              onClick={() => p.onRole(r.id)}
            >
              <b>{r.name}</b>
              <span>{r.hint}</span>
            </button>
          ))}
        </div>
      </section>
    </>
  );
}

function Files(p: Parameters<typeof ChatPanel>[0]) {
  const [query, setQuery] = useState("");
  const attached = new Set([...p.sent, ...p.pending].map((f) => f.name));
  const touched = new Map<string, "changed" | "read">();
  for (const { steps } of p.steps) {
    for (const s of steps) {
      if (!s.ok || s.undone) continue;
      if (s.kind === "write" || s.kind === "edit") touched.set(s.path, "changed");
      else if (s.kind === "read" && !touched.has(s.path)) touched.set(s.path, "read");
    }
  }
  const tagName = { attached: t("приложен", "attached"), changed: t("изменён", "changed"), read: t("прочитан", "read") };
  const q = query.trim().toLowerCase();
  const all = p.listing?.files ?? [];
  const found = q ? all.filter((f) => f.toLowerCase().includes(q)) : all;

  return (
    <>
      <section className="panel-block">
        <h4>{t("Приложено в разговоре", "Attached in the conversation")}</h4>
        {p.sent.length + p.pending.length === 0 ? (
          <p className="muted small">
            {t(
              "Пока ничего. Скрепка под полем ввода или перетащите файл в окно.",
              "Nothing yet. Use the paperclip under the input box or drag a file into the window.",
            )}
          </p>
        ) : (
          <ul className="panel-files">
            {p.pending.map((f, i) => (
              <li key={`p${i}`} className="hot">
                <Icon name={f.kind === "audio" ? "mic" : "doc"} size={16} />
                <span className="name">{f.name}</span>
                <span className="tag">{t("к вопросу", "for the question")}</span>
              </li>
            ))}
            {p.sent.map((f, i) => (
              <li key={`s${i}`}>
                <Icon name={f.kind === "audio" ? "mic" : "doc"} size={16} />
                <span className="name">{f.name}</span>
                <span className="muted small">{memoryPages(f.tokens).replace(/^(около|about) /, "")}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel-block">
        <h4>{t("Папка проекта", "Project folder")}</h4>
        {p.folder ? (
          <>
            <div className="panel-folder">
              <Icon name="folder" size={16} />
              <b title={p.folder}>{p.listing?.name ?? p.folder}</b>
              <button className="link" disabled={p.answering} onClick={p.onPickFolder}>
                {t("Сменить", "Change")}
              </button>
              <button className="link" disabled={p.answering} onClick={p.onDropFolder}>
                {t("Убрать", "Remove")}
              </button>
            </div>
            <ModeSwitch mode={p.mode} disabled={p.answering} onChange={p.onMode} />
            {p.folderError && <p className="error small">{p.folderError}</p>}
            {p.llm.state === "ready" && !p.llm.tools && (
              <p className="muted small">
                {t(
                  "Эта модель сама файлы не открывает — прикладывайте нужные отсюда.",
                  "This model doesn't open files by itself — attach the ones you need from here.",
                )}
              </p>
            )}
            {all.length > 12 && (
              <label className="chat-search panel-search">
                <Icon name="search" size={15} />
                <input
                  type="search"
                  placeholder={t("Найти файл", "Find a file")}
                  aria-label={t("Найти файл в папке", "Find a file in the folder")}
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => e.key === "Escape" && setQuery("")}
                />
              </label>
            )}
            <ul className="panel-files tree">
              {found.slice(0, SHOWN).map((f) => {
                const cut = f.lastIndexOf("/");
                const tag = attached.has(f) ? "attached" : touched.get(f);
                return (
                  <li key={f}>
                    <button
                      title={t(`Приложить к вопросу: в тексте появится @${f}`, `Attach to the question: @${f} will appear in the text`)}
                      disabled={p.answering || attached.has(f)}
                      onClick={() => p.onAttach(f)}
                    >
                      <Icon name="doc" size={16} />
                      <span className="name">
                        {cut > 0 && <span className="muted">{f.slice(0, cut + 1)}</span>}
                        {f.slice(cut + 1)}
                      </span>
                      {tag && <span className={tag === "changed" ? "tag" : "tag quiet"}>{tagName[tag]}</span>}
                    </button>
                  </li>
                );
              })}
            </ul>
            {found.length > SHOWN && (
              <p className="muted small">
                {t(`Показаны ${SHOWN} из ${found.length} — уточните поиск.`, `Showing ${SHOWN} of ${found.length} — narrow the search.`)}
              </p>
            )}
            {q && found.length === 0 && <p className="muted small">{t("Такого файла в папке нет.", "No such file in the folder.")}</p>}
            {p.listing?.truncated && !q && (
              <p className="muted small">
                {t(
                  "Файлов очень много — показаны не все, найдите нужный поиском.",
                  "There are very many files — not all are shown, find the one you need with search.",
                )}
              </p>
            )}
            {all.length > 0 && (
              <p className="muted small">
                {t(
                  "Нажмите на файл — он приложится к вопросу, а в тексте появится ссылка на него. То же — «@» в поле ввода.",
                  "Click a file — it gets attached to the question and a link to it appears in the text. Same as “@” in the input box.",
                )}
              </p>
            )}
          </>
        ) : (
          <>
            <p className="muted small">
              {t(
                "Выберите папку — модель увидит её файлы, сможет их читать, а с вашего разрешения создавать и менять.",
                "Pick a folder — the model will see its files, read them, and with your permission create and change them.",
              )}
            </p>
            <div className="actions">
              <button className="secondary" onClick={p.onPickFolder}>
                {t("Выбрать папку", "Pick a folder")}
              </button>
            </div>
          </>
        )}
      </section>

      {p.steps.length > 0 && (
        <section className="panel-block">
          <h4>{t("Что модель делала с файлами", "What the model did with files")}</h4>
          {p.steps.map(({ line, steps }) => (
            <Steps key={line} steps={steps} onUndo={p.folder && !p.answering ? (i) => p.onUndo(line, i) : undefined} />
          ))}
        </section>
      )}
    </>
  );
}
