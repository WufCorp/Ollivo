import { useState } from "react";
import { type FileMode, type Step, type WriteAsk } from "../api";
import { plural } from "../words";

const lines = (n: number) => `${n} ${plural(n, "строка", "строки", "строк")}`;

export const FILE_MODES: { id: FileMode; name: string; hint: string }[] = [
  {
    id: "ask",
    name: "Вручную",
    hint: "Перед каждым созданием, изменением или удалением файла модель спрашивает разрешения",
  },
  {
    id: "auto",
    name: "Авто",
    hint: "Модель сама создаёт, меняет и удаляет файлы в папке. Старые версии сохраняются — их можно вернуть",
  },
  { id: "plan", name: "План", hint: "Модель ничего не меняет: изучает файлы и пишет план. Выполнить его — одной кнопкой" },
];

/** Переключатель режима работы с файлами. */
export function ModeSwitch({
  mode,
  disabled,
  onChange,
}: {
  mode: FileMode;
  disabled?: boolean;
  onChange: (m: FileMode) => void;
}) {
  return (
    <div className="seg" role="radiogroup" aria-label="Как менять файлы">
      {FILE_MODES.map((m) => (
        <button
          key={m.id}
          role="radio"
          aria-checked={m.id === mode}
          className={m.id === mode ? "active" : ""}
          title={m.hint}
          disabled={disabled}
          onClick={() => onChange(m.id)}
        >
          {m.name}
        </button>
      ))}
    </div>
  );
}

/** «@запрос» прямо перед курсором — человек ссылается на файл папки. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const m = /(^|\s)@([^\s@]*)$/.exec(text.slice(0, caret));
  return m ? { start: caret - m[2].length - 1, query: m[2] } : null;
}

/** Строчные и «е» вместо «ё» — как в поиске по разговорам. */
const fold = (s: string) => s.toLowerCase().replace(/ё/g, "е");

/** Файлы под запрос: сначала те, у кого с запроса начинается имя, потом остальные. */
export function matchFiles(files: string[], query: string, limit = 8): string[] {
  const q = fold(query);
  const name = (f: string) => fold(f.slice(f.lastIndexOf("/") + 1));
  const hits = files.filter((f) => fold(f).includes(q));
  hits.sort((a, b) => Number(!name(a).startsWith(q)) - Number(!name(b).startsWith(q)) || a.length - b.length);
  return hits.slice(0, limit);
}

/** Список файлов по «@» над полем ввода. */
export function Mentions({
  files,
  active,
  onPick,
}: {
  files: string[];
  active: number;
  onPick: (path: string) => void;
}) {
  if (!files.length) return <div className="mentions muted small">Такого файла в папке нет.</div>;
  return (
    <div className="mentions" role="listbox">
      {files.map((f, i) => (
        <button
          key={f}
          role="option"
          aria-selected={i === active}
          className={i === active ? "active" : ""}
          // mousedown, а не click: иначе поле ввода успеет потерять фокус и закрыть список.
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(f);
          }}
        >
          {f}
        </button>
      ))}
    </div>
  );
}

function stepText(s: Step): string {
  const where = s.path || "папку проекта";
  switch (s.kind) {
    case "read":
      return s.ok ? `📄 Прочитала ${s.path}` : `📄 Не прочитала ${s.path}`;
    case "list":
      return `📂 Посмотрела ${s.path ? `папку ${s.path}` : where}`;
    case "search":
      return `🔍 Искала «${s.path}»`;
    case "write":
      return s.ok ? `💾 Сохранила ${s.path}` : `💾 Не сохранила ${s.path}`;
    case "edit":
      return s.ok ? `✏️ Поправила ${s.path}` : `✏️ Не поправила ${s.path}`;
    case "delete":
      return s.ok ? `🗑 Удалила ${s.path}` : `🗑 Не удалила ${s.path}`;
    case "plan":
      return `Хотела изменить ${s.path}`;
    default:
      return `Не поняла, что сделать: ${s.path}`;
  }
}

/** Модель пишет, что создала, сохранила, удалила или поменяла файл: «создан», «удалён»,
 *  «saved»… Окончание слова проверяем, чтобы «создание» не считалось. */
const CLAIMS =
  /(создан|сохран[её]н|удал[её]н|измен[её]н|обновл[её]н|записан|исправлен|добавлен)[аоы]?(?![а-яё])|\b(created|saved|deleted|updated|modified)\b/i;

/** Ответ говорит об изменённых файлах, а шагов с изменениями нет. Qwen2.5 3B ответила
 *  «Файл notes.txt успешно удалён», не удалив его. Человек должен узнать правду из окна,
 *  а не верить модели на слово. */
export function claimsChanges(text: string, steps: Step[] | undefined): boolean {
  const changed = (steps ?? []).some((s) => s.ok && (s.kind === "write" || s.kind === "edit" || s.kind === "delete"));
  return !changed && CLAIMS.test(text);
}

/** Одинаковые шаги подряд — одной строкой с числом: маленькая модель бывает читает
 *  один файл по кругу, и пять строк «Прочитала main.py» только мешают. */
function grouped(steps: Step[]): { step: Step; index: number; times: number }[] {
  const out: { step: Step; index: number; times: number }[] = [];
  steps.forEach((s, index) => {
    const last = out[out.length - 1];
    const changes = s.kind === "write" || s.kind === "edit" || s.kind === "delete";
    const same = last && !changes && last.step.kind === s.kind && last.step.path === s.path && last.step.ok === s.ok;
    if (same) last.times += 1;
    else out.push({ step: s, index, times: 1 });
  });
  return out;
}

/** Что модель делала с файлами, пока отвечала. У сохранённого — «Вернуть как было». */
export function Steps({ steps, onUndo }: { steps: Step[]; onUndo?: (i: number) => Promise<void> }) {
  const [busy, setBusy] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const undo = async (i: number) => {
    if (!onUndo) return;
    setBusy(i);
    setError(null);
    try {
      await onUndo(i);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <ul className="file-steps">
      {grouped(steps).map(({ step: s, index: i, times }) => (
        <li key={i} className={s.ok ? "" : "failed"}>
          {stepText(s)}
          {times > 1 && <span className="muted"> ×{times}</span>}
          {s.note && <span className="muted"> — {s.note}</span>}
          {(s.kind === "write" || s.kind === "edit" || s.kind === "delete") && s.ok && (
            s.undone ? (
              <span className="muted"> — возвращено как было</span>
            ) : (
              onUndo && (
                <button className="link" disabled={busy !== null} onClick={() => undo(i)}>
                  {busy === i ? "Возвращаю…" : "Вернуть как было"}
                </button>
              )
            )
          )}
        </li>
      ))}
      {error && <li className="error">{error}</li>}
    </ul>
  );
}

/** Модель просит записать файл: что именно, и решение за человеком. */
export function WriteCard({ ask, onAnswer }: { ask: WriteAsk; onAnswer: (ok: boolean) => void }) {
  const [yes, no] = ask.kind === "delete" ? ["Удалить", "Не удалять"] : ["Сохранить", "Не сохранять"];
  return (
    <div className="card notice write-ask">
      <p>
        {ask.kind === "delete" ? (
          <>
            Модель хочет удалить файл <b>{ask.path}</b> — {lines(ask.old_lines)}. Копию сохраню — файл можно
            будет вернуть.
          </>
        ) : ask.kind === "edit" ? (
          <>
            Модель хочет поправить файл <b>{ask.path}</b>: было {lines(ask.old_lines)}, станет {lines(ask.new_lines)}.
            Старую версию сохраню — её можно будет вернуть.
          </>
        ) : ask.exists ? (
          <>
            Модель хочет заменить файл <b>{ask.path}</b> целиком: было {lines(ask.old_lines)}, станет{" "}
            {lines(ask.new_lines)}. Старую версию сохраню — её можно будет вернуть.
          </>
        ) : (
          <>
            Модель хочет создать файл <b>{ask.path}</b> — {lines(ask.new_lines)}.
          </>
        )}
      </p>
      {ask.kind === "edit" ? (
        <details open>
          <summary>Что поменяется</summary>
          <p className="muted small">Было:</p>
          <pre className="before">{ask.before}</pre>
          <p className="muted small">Станет:</p>
          <pre className="after">{ask.after}</pre>
        </details>
      ) : (
        <details>
          <summary>{ask.kind === "delete" ? "Показать, что в файле" : "Показать, что будет в файле"}</summary>
          <pre>{ask.content}</pre>
        </details>
      )}
      <div className="actions">
        <button onClick={() => onAnswer(true)}>{yes}</button>
        <button className="secondary" onClick={() => onAnswer(false)}>
          {no}
        </button>
      </div>
    </div>
  );
}
