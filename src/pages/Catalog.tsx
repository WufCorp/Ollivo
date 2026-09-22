import { useEffect, useState } from "react";
import {
  LIGHTS,
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

function params(n: number): string {
  return `${(n / 1e9).toFixed(1).replace(".", ",").replace(",0", "")} млрд параметров`;
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
      <span className="light">{LIGHTS[v.verdict.light]}</span>
      <div className="variant-text">
        <p>
          <b>{v.quant}</b> · {formatBytes(v.size)} — {v.verdict.headline}
        </p>
        <p className="muted small">{v.quality}</p>
        {v.verdict.details.map((d) => (
          <p key={d} className="muted small">
            {d}
          </p>
        ))}
        {going && p && (
          <>
            <progress value={p!.done} max={p!.total ?? undefined} />
            <p className="muted small">
              {p!.phase === "verifying"
                ? "Проверяю, всё ли скачалось целым…"
                : `${formatBytes(p!.done)} из ${formatBytes(p!.total ?? v.size)} · ${formatBytes(p!.speed)}/с`}
            </p>
          </>
        )}
        {task?.error && task.error !== "paused" && <p className="error">{task.error}</p>}
        {task?.error === "paused" && <p className="muted small">Загрузка на паузе — можно продолжить.</p>}
      </div>

      {path ? (
        <div className="actions">
          <button
            onClick={() => {
              llmStart(path);
              onGoToChat();
            }}
          >
            Запустить
          </button>
        </div>
      ) : going ? (
        <button className="secondary" onClick={() => taskPause(id)}>
          Пауза
        </button>
      ) : (
        <button onClick={() => onStart(v)}>
          {task?.error === "paused" ? "Продолжить" : task?.error ? "Ещё раз" : "Скачать"}
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

  const patch = (id: string, t: Partial<Task>) =>
    setTasks((all) => ({ ...all, [id]: { ...(all[id] ?? NOTHING), ...t } }));

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

  const start = async (repo: string, v: CatalogVariant, title?: string) => {
    const id = taskId(repo, v.name);
    patch(id, { error: null, done: null });
    try {
      await catalogDownload(repo, v.name, v.sha256, title);
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
      <h2>Каталог</h2>

      <div className="actions">
        <button className={mode === "picks" ? "" : "secondary"} onClick={() => setMode("picks")}>
          Подборка
        </button>
        <button className={mode === "search" ? "" : "secondary"} onClick={() => setMode("search")}>
          Поиск по HuggingFace
        </button>
      </div>

      <label className="check filter">
        <input type="checkbox" checked={onlyFits} onChange={(e) => setOnlyFits(e.target.checked)} />
        Показывать только то, что пойдёт на моём компьютере
      </label>

      {mode === "picks" ? (
        <>
          <p className="muted small">
            Проверенные модели для переписки. Размер выбирайте по «светофору»: зелёный — поместится в
            видеокарту целиком, жёлтый — будет работать, но медленнее.
          </p>
          {picks?.map((m) => {
            const variants = m.variants.filter(fits);
            if (!variants.length) return null;
            return (
              <div className="card model" key={m.id}>
                <p className="model-title">
                  <span className="light">{LIGHTS[variants[0].verdict.light]}</span>
                  {m.title}
                </p>
                <p className="muted small">
                  {[m.vendor, params(m.params), ...m.tags].join(" · ")}
                </p>
                <p>{m.about}</p>
                {m.license && <p className="muted small">Лицензия: {m.license}</p>}
                {variants.map((v) => (
                  <Variant
                    key={v.name}
                    v={v}
                    repo={m.repo}
                    task={tasks[taskId(m.repo, v.name)]}
                    onStart={(x) => start(m.repo, x, `${m.title} ${x.quant}`)}
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
              placeholder="Название модели, например Qwen3.5 или Gemma"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && search()}
            />
            <button onClick={search} disabled={searching || !query.trim()}>
              {searching ? "Ищу…" : "Найти"}
            </button>
          </div>
          <p className="muted small">
            Ищем среди моделей в формате GGUF — такие запускает движок чата. Ollivo не проверяла их: что
            внутри чужой модели, знает только тот, кто её выложил.
          </p>

          {searchError && <p className="error">{searchError}</p>}
          {found?.length === 0 && <p className="muted">Ничего не нашлось. Попробуйте другое название.</p>}

          {found?.map((r) => (
            <div className="card model" key={r.repo}>
              <p className="model-title">{r.name}</p>
              <p className="muted small">
                {[r.author, `скачали ${r.downloads.toLocaleString("ru")} раз`, r.license ?? ""]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
              {r.gated && (
                <p className="muted small">
                  Закрытая модель: нужен токен HuggingFace в настройках и согласие на её странице.
                </p>
              )}
              <div className="actions">
                <button className="secondary" onClick={() => openRepo(r.repo)}>
                  {open === r.repo ? "Свернуть" : "Что скачать"}
                </button>
              </div>

              {open === r.repo && (
                <>
                  {filesError && <p className="error">{filesError}</p>}
                  {!files && !filesError && <p className="muted small">Смотрю, что там есть…</p>}
                  {files?.variants.filter(fits).map((v) => (
                    <Variant
                      key={v.name}
                      v={v}
                      repo={r.repo}
                      task={tasks[taskId(r.repo, v.name)]}
                      onStart={(x) => start(r.repo, x)}
                      onGoToChat={onGoToChat}
                    />
                  ))}
                  {files?.variants.length === 0 && (
                    <p className="muted small">Готовых файлов для чата тут нет.</p>
                  )}
                  {!!files?.split && (
                    <p className="muted small">
                      Ещё {files.split} файлов разрезаны на части — такие Ollivo пока не качает.
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
