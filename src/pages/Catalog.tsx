import { useEffect, useState } from "react";
import Fit, { Light } from "../components/Fit";
import License from "../components/License";
import {
  catalogDownload,
  catalogFiles,
  catalogPicks,
  catalogSearch,
  formatBytes,
  llmStart,
  onDownloadFinished,
  onDownloadProgress,
  taskPause,
  tasksRunning,
  type CatalogFiles,
  type CatalogPick,
  type CatalogRepo,
  type CatalogVariant,
  type DownloadProgress,
} from "../api";
import { decimal, locale, t } from "../i18n";

/** Что сейчас с загрузкой одного файла. */
interface Task {
  progress: DownloadProgress | null;
  /** Текст ошибки; `paused` — поставлено на паузу. */
  error: string | null;
  /** Путь к скачанному файлу. */
  done: string | null;
}

const NOTHING: Task = { progress: null, error: null, done: null };

/** Задачи загрузки зовутся `model:<репозиторий>/<файл>` — так их шлёт ядро. */
const taskId = (repo: string, name: string) => `model:${repo}/${name}`;

const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

function params(n: number): string {
  const b = decimal(n / 1e9).replace(/[.,]0$/, "");
  return t(`${b} млрд параметров`, `${b}B parameters`);
}

/** Строка варианта: сжатие, размер, «светофор» и кнопка. */
function Variant({
  v,
  repo,
  task,
  onStart,
  onGoToChat,
}: {
  v: CatalogVariant;
  repo: string;
  task: Task | undefined;
  onStart: (v: CatalogVariant) => void;
  onGoToChat: () => void;
}) {
  const id = taskId(repo, v.name);
  const p = task?.progress;
  const path = v.downloaded ?? task?.done ?? null;
  const going = !!task && !task.error && !task.done;

  return (
    <div className="variant">
      <Light light={v.verdict.light} />
      <div className="variant-text">
        {/* Крупно — смысл сжатия словами; код («Q4_K_M») — мелко, для тех, кто его знает,
            и чтобы различить варианты с одинаковым описанием. */}
        <p className="variant-title">
          <b>{capitalize(v.quality)}</b>{" "}
          <span className="muted">
            · {formatBytes(v.size)} · {t("вариант", "variant")} {v.quant}
          </span>
        </p>
        <p className="variant-headline">{v.verdict.headline}</p>
        <Fit verdict={v.verdict} />
        {going && p && (
          <>
            <progress value={p!.done} max={p!.total ?? undefined} />
            <p className="muted small">
              {p!.phase === "verifying"
                ? t("Проверяю, всё ли скачалось целым…", "Checking everything downloaded intact…")
                : `${formatBytes(p!.done)} ${t("из", "of")} ${formatBytes(p!.total ?? v.size)} · ${formatBytes(p!.speed)}/${t("с", "s")}`}
            </p>
          </>
        )}
        {task?.error && task.error !== "paused" && <p className="error">{task.error}</p>}
        {task?.error === "paused" && <p className="muted small">{t("Загрузка на паузе — можно продолжить.", "The download is paused — you can resume it.")}</p>}
      </div>

      {path ? (
        <div className="actions">
          <button
            onClick={() => {
              llmStart(path);
              onGoToChat();
            }}
          >
            {t("Запустить", "Start")}
          </button>
        </div>
      ) : going ? (
        <button className="secondary" onClick={() => taskPause(id)}>
          {t("Пауза", "Pause")}
        </button>
      ) : (
        <button onClick={() => onStart(v)}>
          {task?.error === "paused" ? t("Продолжить", "Resume") : task?.error ? t("Ещё раз", "Try again") : t("Скачать", "Download")}
        </button>
      )}
    </div>
  );
}

export default function Catalog({ onGoToChat }: { onGoToChat: () => void }) {
  const [mode, setMode] = useState<"picks" | "search">("picks");
  const [onlyFits, setOnlyFits] = useState(false);
  const [picks, setPicks] = useState<CatalogPick[] | null>(null);
  const [tasks, setTasks] = useState<Record<string, Task>>({});

  const [query, setQuery] = useState("");
  const [found, setFound] = useState<CatalogRepo[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  /** Раскрытый репозиторий: его файлы или ошибка. */
  const [open, setOpen] = useState<string | null>(null);
  const [files, setFiles] = useState<CatalogFiles | null>(null);
  const [filesError, setFilesError] = useState<string | null>(null);

  const patch = (id: string, upd: Partial<Task>) =>
    setTasks((all) => ({ ...all, [id]: { ...(all[id] ?? NOTHING), ...upd } }));

  useEffect(() => {
    catalogPicks().then(setPicks);
    // Экран могли закрыть и открыть заново посреди загрузки — спросим, что идёт.
    tasksRunning().then((ids) =>
      ids.filter((id) => id.startsWith("model:")).forEach((id) => patch(id, {})),
    );
    // Загрузки моделей идут теми же событиями, что и всё остальное: отбираем свои.
    const progress = onDownloadProgress((p) => {
      if (p.id.startsWith("model:")) patch(p.id, { progress: p, error: null });
    });
    const finished = onDownloadFinished((f) => {
      if (!f.id.startsWith("model:")) return;
      patch(f.id, { error: f.error, done: f.result ?? null, progress: null });
    });
    return () => {
      progress.then((un) => un());
      finished.then((un) => un());
    };
  }, []);

  /** `license` едет вместе с файлом в библиотеку: в заголовке GGUF её обычно нет. */
  const start = async (repo: string, v: CatalogVariant, license: string | null, title?: string) => {
    const id = taskId(repo, v.name);
    patch(id, { error: null, done: null });
    try {
      await catalogDownload(repo, v.name, v.sha256, title, license);
    } catch (e) {
      patch(id, { error: String(e) });
    }
  };

  const search = async () => {
    setSearching(true);
    setSearchError(null);
    setOpen(null);
    try {
      setFound(await catalogSearch(query));
    } catch (e) {
      setFound(null);
      setSearchError(String(e));
    } finally {
      setSearching(false);
    }
  };

  const openRepo = async (repo: string) => {
    if (open === repo) {
      setOpen(null);
      return;
    }
    setOpen(repo);
    setFiles(null);
    setFilesError(null);
    try {
      setFiles(await catalogFiles(repo));
    } catch (e) {
      setFilesError(String(e));
    }
  };

  /** Фильтр «пойдёт на моём ПК»: прячем то, на что не хватит памяти. */
  const fits = (v: CatalogVariant) => !onlyFits || v.verdict.light !== "red";

  return (
    <>
      <div className="page-head">
        <h2>{t("Каталог", "Catalog")}</h2>
        <div className="seg" role="radiogroup" aria-label={t("Что показать", "What to show")}>
          <button role="radio" aria-checked={mode === "picks"} className={mode === "picks" ? "active" : ""} onClick={() => setMode("picks")}>
            {t("Подборка", "Picks")}
          </button>
          <button role="radio" aria-checked={mode === "search"} className={mode === "search" ? "active" : ""} onClick={() => setMode("search")}>
            {t("Поиск по HuggingFace", "Search HuggingFace")}
          </button>
        </div>
      </div>

      <label className="check filter switch">
        <input type="checkbox" checked={onlyFits} onChange={(e) => setOnlyFits(e.target.checked)} />
        {t("Показывать только то, что пойдёт на моём компьютере", "Show only what will run on my computer")}
      </label>

      {mode === "picks" ? (
        <>
          <p className="muted small">
            {t(
              "Проверенные модели для переписки. Размер выбирайте по «светофору»: зелёный — поместится в видеокарту целиком, жёлтый — будет работать, но медленнее. Полоса — сколько видеопамяти модель займёт из свободной.",
              "Tested models for chatting. Choose the size by the “traffic light”: green — fits entirely in the graphics card, yellow — will work, but slower. The bar shows how much of the free video memory the model will take.",
            )}
          </p>
          {picks?.map((m) => {
            const variants = m.variants.filter(fits);
            if (!variants.length) return null;
            return (
              <div className="card model" key={m.id}>
                <p className="model-title">
                  <Light light={variants[0].verdict.light} />
                  {m.title}
                </p>
                <p className="muted small">
                  {[m.vendor, params(m.params), ...m.tags].join(" · ")}
                </p>
                <p>{m.about}</p>
                <License code={m.license} repo={m.repo} />
                {variants.map((v) => (
                  <Variant
                    key={v.name}
                    v={v}
                    repo={m.repo}
                    task={tasks[taskId(m.repo, v.name)]}
                    onStart={(x) => start(m.repo, x, m.license, `${m.title} ${x.quant}`)}
                    onGoToChat={onGoToChat}
                  />
                ))}
              </div>
            );
          })}
        </>
      ) : (
        <>
          <div className="row">
            <input
              className="grow-input"
              placeholder={t("Название модели, например Qwen3.5 или Gemma", "Model name, e.g. Qwen3.5 or Gemma")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && search()}
            />
            <button onClick={search} disabled={searching || !query.trim()}>
              {searching ? t("Ищу…", "Searching…") : t("Найти", "Search")}
            </button>
          </div>
          <p className="muted small">
            {t(
              "Ищем среди моделей в формате GGUF — такие запускает движок чата. Ollivo не проверяла их: что внутри чужой модели, знает только тот, кто её выложил.",
              "We search among models in GGUF format — the chat engine runs these. Ollivo hasn't tested them: only whoever uploaded a model knows what is inside it.",
            )}
          </p>

          {searchError && <p className="error">{searchError}</p>}
          {found?.length === 0 && <p className="muted">{t("Ничего не нашлось. Попробуйте другое название.", "Nothing found. Try another name.")}</p>}

          {found?.map((r) => (
            <div className="card model" key={r.repo}>
              <p className="model-title">{r.name}</p>
              <p className="muted small">
                {[r.author, t(`скачали ${r.downloads.toLocaleString(locale())} раз`, `${r.downloads.toLocaleString(locale())} downloads`)].join(" · ")}
              </p>
              <License code={r.license} repo={r.repo} />
              {r.gated && (
                <p className="muted small">
                  {t(
                    "Закрытая модель: нужен токен HuggingFace в настройках и согласие на её странице.",
                    "Gated model: you need a HuggingFace token in the settings and to accept the terms on its page.",
                  )}
                </p>
              )}
              <div className="actions">
                <button className="secondary" onClick={() => openRepo(r.repo)}>
                  {open === r.repo ? t("Свернуть", "Collapse") : t("Что скачать", "What to download")}
                </button>
              </div>

              {open === r.repo && (
                <>
                  {filesError && <p className="error">{filesError}</p>}
                  {!files && !filesError && <p className="muted small">{t("Смотрю, что там есть…", "Looking at what's there…")}</p>}
                  {files?.variants.filter(fits).map((v) => (
                    <Variant
                      key={v.name}
                      v={v}
                      repo={r.repo}
                      task={tasks[taskId(r.repo, v.name)]}
                      onStart={(x) => start(r.repo, x, r.license)}
                      onGoToChat={onGoToChat}
                    />
                  ))}
                  {files?.variants.length === 0 && (
                    <p className="muted small">{t("Готовых файлов для чата тут нет.", "No ready files for chat here.")}</p>
                  )}
                  {!!files?.split && (
                    <p className="muted small">
                      {t(
                        `Ещё ${files.split} файлов разрезаны на части — такие Ollivo пока не качает.`,
                        `${files.split} more files are split into parts — Ollivo doesn't download those yet.`,
                      )}
                    </p>
                  )}
                </>
              )}
            </div>
          ))}
        </>
      )}
    </>
  );
}
