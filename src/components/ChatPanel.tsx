import { useState } from "react";
import type { Attachment, ChatRole, FileMode, Listing, LlmState, Step } from "../api";
import { memoryPages, pageCount } from "../words";
import Icon from "./Icon";
import { ModeSwitch, Steps } from "./Project";
import { whoComputes } from "./RunningModel";

export type PanelTab = "talk" | "files";

export const PANEL_TABS: { id: PanelTab; name: string; icon: "gauge" | "folder" }[] = [
  { id: "talk", name: "Разговор", icon: "gauge" },
  { id: "files", name: "Файлы", icon: "folder" },
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
    <aside className="chat-panel" aria-label="Панель разговора">
      {/* Вкладки — кнопками в шапке разговора; здесь только название, чтобы не было двух переключателей. */}
      <div className="panel-head">
        <h3>{PANEL_TABS.find((t) => t.id === p.tab)?.name}</h3>
        <button className="forget" title="Закрыть панель" onClick={p.onClose}>
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
        <h4>Память разговора</h4>
        {ctx ? (
          <>
            <p className="panel-big">
              {pageCount(p.used)} <small>из {pageCount(ctx)} страниц</small>
            </p>
            <div className="bar" role="img" aria-label={`занято ${Math.round(k * 100)}%`}>
              <i className={light} style={{ transform: `scaleX(${k})` }} />
            </div>
            <p className="muted small">
              {k < 0.9
                ? "Когда память заполнится, модель начнёт забывать начало разговора."
                : "Память почти заполнена: модель начнёт забывать начало. Для новой темы лучше новый разговор."}
            </p>
          </>
        ) : (
          <p className="muted small">Запустите модель — здесь будет видно, сколько разговора она помнит.</p>
        )}
      </section>

      <section className="panel-block">
        <h4>Модель</h4>
        {llm.model ? (
          <dl className="panel-facts">
            <dt title={llm.model}>{modelName(llm.model)}</dt>
            <dd>{on ? (llm.state === "ready" ? "готова" : "выгружена") : "не запущена"}</dd>
            {p.lastSpeed && (
              <>
                <dt>Последний ответ</dt>
                <dd>{p.lastSpeed}</dd>
              </>
            )}
            {llm.state === "ready" && ctx !== null && (
              <>
                <dt>Помнит</dt>
                <dd>{memoryPages(ctx)}</dd>
              </>
            )}
          </dl>
        ) : (
          <p className="muted small">Модель не запущена.</p>
        )}
        {llm.state === "ready" && (
          <p className="muted small">{whoComputes(llm.gpu_layers, llm.layers)}.</p>
        )}
        <div className="actions">
          <button className="secondary" onClick={p.onGoToModels}>
            Сменить модель
          </button>
          {llm.state === "ready" && (
            <button className="secondary" disabled={p.answering} onClick={p.onStop}>
              Выгрузить
            </button>
          )}
        </div>
      </section>

      <section className="panel-block">
        <h4>Кто отвечает</h4>
        <div className="roles" role="radiogroup" aria-label="Роль модели">
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
  const touched = new Map<string, "изменён" | "прочитан">();
  for (const { steps } of p.steps) {
    for (const s of steps) {
      if (!s.ok || s.undone) continue;
      if (s.kind === "write" || s.kind === "edit") touched.set(s.path, "изменён");
      else if (s.kind === "read" && !touched.has(s.path)) touched.set(s.path, "прочитан");
    }
  }
  const q = query.trim().toLowerCase();
  const all = p.listing?.files ?? [];
  const found = q ? all.filter((f) => f.toLowerCase().includes(q)) : all;

  return (
    <>
      <section className="panel-block">
        <h4>Приложено в разговоре</h4>
        {p.sent.length + p.pending.length === 0 ? (
          <p className="muted small">Пока ничего. Скрепка под полем ввода или перетащите файл в окно.</p>
        ) : (
          <ul className="panel-files">
            {p.pending.map((f, i) => (
              <li key={`p${i}`} className="hot">
                <Icon name={f.kind === "audio" ? "mic" : "doc"} size={16} />
                <span className="name">{f.name}</span>
                <span className="tag">к вопросу</span>
              </li>
            ))}
            {p.sent.map((f, i) => (
              <li key={`s${i}`}>
                <Icon name={f.kind === "audio" ? "mic" : "doc"} size={16} />
                <span className="name">{f.name}</span>
                <span className="muted small">{memoryPages(f.tokens).replace("около ", "")}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel-block">
        <h4>Папка проекта</h4>
        {p.folder ? (
          <>
            <div className="panel-folder">
              <Icon name="folder" size={16} />
              <b title={p.folder}>{p.listing?.name ?? p.folder}</b>
              <button className="link" disabled={p.answering} onClick={p.onPickFolder}>
                Сменить
              </button>
              <button className="link" disabled={p.answering} onClick={p.onDropFolder}>
                Убрать
              </button>
            </div>
            <ModeSwitch mode={p.mode} disabled={p.answering} onChange={p.onMode} />
            {p.folderError && <p className="error small">{p.folderError}</p>}
            {p.llm.state === "ready" && !p.llm.tools && (
              <p className="muted small">Эта модель сама файлы не открывает — прикладывайте нужные отсюда.</p>
            )}
            {all.length > 12 && (
              <label className="chat-search panel-search">
                <Icon name="search" size={15} />
                <input
                  type="search"
                  placeholder="Найти файл"
                  aria-label="Найти файл в папке"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => e.key === "Escape" && setQuery("")}
                />
              </label>
            )}
            <ul className="panel-files tree">
              {found.slice(0, SHOWN).map((f) => {
                const cut = f.lastIndexOf("/");
                const tag = attached.has(f) ? "приложен" : touched.get(f);
                return (
                  <li key={f}>
                    <button
                      title={`Приложить к вопросу: в тексте появится @${f}`}
                      disabled={p.answering || attached.has(f)}
                      onClick={() => p.onAttach(f)}
                    >
                      <Icon name="doc" size={16} />
                      <span className="name">
                        {cut > 0 && <span className="muted">{f.slice(0, cut + 1)}</span>}
                        {f.slice(cut + 1)}
                      </span>
                      {tag && <span className={tag === "изменён" ? "tag" : "tag quiet"}>{tag}</span>}
                    </button>
                  </li>
                );
              })}
            </ul>
            {found.length > SHOWN && (
              <p className="muted small">
                Показаны {SHOWN} из {found.length} — уточните поиск.
              </p>
            )}
            {q && found.length === 0 && <p className="muted small">Такого файла в папке нет.</p>}
            {p.listing?.truncated && !q && (
              <p className="muted small">Файлов очень много — показаны не все, найдите нужный поиском.</p>
            )}
            {all.length > 0 && (
              <p className="muted small">
                Нажмите на файл — он приложится к вопросу, а в тексте появится ссылка на него. То же — «@» в поле
                ввода.
              </p>
            )}
          </>
        ) : (
          <>
            <p className="muted small">
              Выберите папку — модель увидит её файлы, сможет их читать, а с вашего разрешения создавать и менять.
            </p>
            <div className="actions">
              <button className="secondary" onClick={p.onPickFolder}>
                Выбрать папку
              </button>
            </div>
          </>
        )}
      </section>

      {p.steps.length > 0 && (
        <section className="panel-block">
          <h4>Что модель делала с файлами</h4>
          {p.steps.map(({ line, steps }) => (
            <Steps key={line} steps={steps} onUndo={p.folder && !p.answering ? (i) => p.onUndo(line, i) : undefined} />
          ))}
        </section>
      )}
    </>
  );
}
