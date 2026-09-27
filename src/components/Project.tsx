import { useState } from "react";
import { type FileMode, type Step, type WriteAsk } from "../api";
import { pl, t } from "../i18n";
import Icon, { type IconName } from "./Icon";

const lines = (n: number) => `${n} ${pl(n, ["строка", "строки", "строк"], ["line", "lines"])}`;

export const fileModes = (): { id: FileMode; name: string; hint: string }[] => [
  {
    id: "ask",
    name: t("Вручную", "Manual"),
    hint: t(
      "Перед каждым созданием, изменением или удалением файла модель спрашивает разрешения",
      "Before creating, changing or deleting any file, the model asks for permission",
    ),
  },
  {
    id: "auto",
    name: t("Авто", "Auto"),
    hint: t(
      "Модель сама создаёт, меняет и удаляет файлы в папке. Старые версии сохраняются — их можно вернуть",
      "The model creates, changes and deletes files in the folder by itself. Old versions are kept — you can restore them",
    ),
  },
  {
    id: "plan",
    name: t("План", "Plan"),
    hint: t(
      "Модель ничего не меняет: изучает файлы и пишет план. Выполнить его — одной кнопкой",
      "The model changes nothing: it studies the files and writes a plan. Carry it out with one button",
    ),
  },
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
    <div className="seg" role="radiogroup" aria-label={t("Как менять файлы", "How to change files")}>
      {fileModes().map((m) => (
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
  if (!files.length) return <div className="mentions muted small">{t("Такого файла в папке нет.", "No such file in the folder.")}</div>;
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

/** Значок шага: свой, а не эмодзи — эмодзи Windows рисует цветными картинками. */
const STEP_ICONS: Record<Step["kind"], IconName> = {
  read: "doc",
  list: "folder",
  search: "search",
  write: "doc",
  edit: "doc",
  delete: "close",
  plan: "warn",
  unknown: "warn",
};

function stepText(s: Step): string {
  const p = s.path;
  switch (s.kind) {
    case "read":
      return s.ok ? t(`Прочитала ${p}`, `Read ${p}`) : t(`Не прочитала ${p}`, `Couldn't read ${p}`);
    case "list":
      return p ? t(`Посмотрела папку ${p}`, `Looked at folder ${p}`) : t("Посмотрела папку проекта", "Looked at the project folder");
    case "search":
      return t(`Искала «${p}»`, `Searched for “${p}”`);
    case "write":
      return s.ok ? t(`Сохранила ${p}`, `Saved ${p}`) : t(`Не сохранила ${p}`, `Couldn't save ${p}`);
    case "edit":
      return s.ok ? t(`Поправила ${p}`, `Edited ${p}`) : t(`Не поправила ${p}`, `Couldn't edit ${p}`);
    case "delete":
      return s.ok ? t(`Удалила ${p}`, `Deleted ${p}`) : t(`Не удалила ${p}`, `Couldn't delete ${p}`);
    case "plan":
      return t(`Хотела изменить ${p}`, `Wanted to change ${p}`);
    default:
      return t(`Не поняла, что сделать: ${p}`, `Didn't understand what to do: ${p}`);
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
          <Icon name={STEP_ICONS[s.kind] ?? "warn"} size={14} />
          {stepText(s)}
          {times > 1 && <span className="muted"> ×{times}</span>}
          {s.note && <span className="muted"> — {s.note}</span>}
          {(s.kind === "write" || s.kind === "edit" || s.kind === "delete") && s.ok && (
            s.undone ? (
              <span className="muted"> — {t("возвращено как было", "restored")}</span>
            ) : (
              onUndo && (
                <button className="link" disabled={busy !== null} onClick={() => undo(i)}>
                  {busy === i ? t("Возвращаю…", "Restoring…") : t("Вернуть как было", "Restore")}
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
  const [yes, no] =
    ask.kind === "delete" ? [t("Удалить", "Delete"), t("Не удалять", "Don't delete")] : [t("Сохранить", "Save"), t("Не сохранять", "Don't save")];
  return (
    <div className="card notice write-ask">
      <p>
        {ask.kind === "delete" ? (
          <>
            {t("Модель хочет удалить файл", "The model wants to delete the file")} <b>{ask.path}</b> — {lines(ask.old_lines)}.{" "}
            {t("Копию сохраню — файл можно будет вернуть.", "I'll keep a copy — you'll be able to restore the file.")}
          </>
        ) : ask.kind === "edit" ? (
          <>
            {t("Модель хочет поправить файл", "The model wants to edit the file")} <b>{ask.path}</b>:{" "}
            {t(
              `было ${lines(ask.old_lines)}, станет ${lines(ask.new_lines)}.`,
              `it was ${lines(ask.old_lines)}, it will be ${lines(ask.new_lines)}.`,
            )}{" "}
            {t("Старую версию сохраню — её можно будет вернуть.", "I'll keep the old version — you'll be able to restore it.")}
          </>
        ) : ask.exists ? (
          <>
            {t("Модель хочет заменить файл", "The model wants to replace the file")} <b>{ask.path}</b>{" "}
            {t(
              `целиком: было ${lines(ask.old_lines)}, станет ${lines(ask.new_lines)}.`,
              `entirely: it was ${lines(ask.old_lines)}, it will be ${lines(ask.new_lines)}.`,
            )}{" "}
            {t("Старую версию сохраню — её можно будет вернуть.", "I'll keep the old version — you'll be able to restore it.")}
          </>
        ) : (
          <>
            {t("Модель хочет создать файл", "The model wants to create the file")} <b>{ask.path}</b> — {lines(ask.new_lines)}.
          </>
        )}
      </p>
      {ask.kind === "edit" ? (
        <details open>
          <summary>{t("Что поменяется", "What will change")}</summary>
          <p className="muted small">{t("Было:", "Before:")}</p>
          <pre className="before">{ask.before}</pre>
          <p className="muted small">{t("Станет:", "After:")}</p>
          <pre className="after">{ask.after}</pre>
        </details>
      ) : (
        <details>
          <summary>
            {ask.kind === "delete" ? t("Показать, что в файле", "Show what's in the file") : t("Показать, что будет в файле", "Show what the file will contain")}
          </summary>
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
