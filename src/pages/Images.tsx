import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import {
  catalogDownload,
  DRAW_TASK,
  formatBytes,
  imagesDraw,
  imagesEstimate,
  imagesGallery,
  imagesInstall,
  imagesModels,
  imagesOpen,
  imagesOpenFolder,
  imagesPicks,
  imagesReveal,
  imagesStatus,
  imagesStop,
  imagesThumb,
  modelsAdd,
  onDownloadFinished,
  onDownloadProgress,
  onDrawFinished,
  onDrawPicture,
  onDrawProgress,
  onEngineFinished,
  onEngineProgress,
  taskPause,
  tasksRunning,
  type DownloadProgress,
  type DrawEstimate,
  type DrawProgress,
  type EngineProgress,
  type ImageModel,
  type ImagePick,
  type ImageQuality,
  type ImageShape,
  type ImagesStatus,
  type Picture,
  type Problem,
} from "../api";
import { Light } from "../components/Fit";
import Icon from "../components/Icon";
import InstallScreen, { formatEta, type InstallError } from "../components/InstallScreen";
import License from "../components/License";
import { openReport } from "../components/Report";
import { pl, t } from "../i18n";

/** Раздел «Картинки»: установка движка → модель → описание → картинки и галерея. */
export default function Images({ onGo }: { onGo: (tab: "models") => void }) {
  const [status, setStatus] = useState<ImagesStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => imagesStatus().then(setStatus, (e) => setError(String(e)));
  useEffect(() => {
    refresh();
  }, []);

  if (error) return <p className="error">{error}</p>;
  if (!status) return <p className="muted">{t("Смотрю, что уже установлено…", "Checking what is already installed…")}</p>;

  return (
    <>
      <h2>{t("Картинки", "Images")}</h2>
      {!status.supported ? (
        <div className="card">
          <p>
            {t(
              "Чтобы рисовать картинки, нужна видеокарта NVIDIA. На этом компьютере её не нашлось.",
              "Drawing pictures needs an NVIDIA graphics card. None was found on this computer.",
            )}
          </p>
          <p className="muted small">
            {t(
              "Видеокарты AMD и Intel появятся после версии 1.0. Чат с моделями работает и без NVIDIA.",
              "AMD and Intel graphics cards will come after version 1.0. Chatting with models works without NVIDIA too.",
            )}
          </p>
        </div>
      ) : status.ready ? (
        <Studio onGo={onGo} />
      ) : (
        <Install status={status} onDone={refresh} />
      )}
    </>
  );
}

/** Через сколько секунд сами повторяем установку, если пропал интернет. */
const NET_RETRY = 15;
/** Установка пакетов после скачивания: ~5 минут на GTX 1080 и NVMe (замер `pyenv`, 8 минут всего). */
const PACKAGES_SECONDS = 5 * 60;

function Install({ status, onDone }: { status: ImagesStatus; onDone: () => void }) {
  const [progress, setProgress] = useState<EngineProgress | null>(null);
  const [last, setLast] = useState<EngineProgress | null>(null);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<InstallError | null>(null);
  const [started, setStarted] = useState(false);
  const [retryIn, setRetryIn] = useState<number | null>(null);

  useEffect(() => {
    // Экран открыли заново посреди установки — события до этого пропущены.
    tasksRunning().then((ids) => ids.includes("engine:images") && setStarted(true));
    const subs = [
      onEngineProgress((p) => {
        if (p.id !== "images") return;
        setProgress(p);
        setLast(p);
      }),
      onEngineFinished((f) => {
        if (f.id !== "images") return;
        setProgress(null);
        setPaused(f.error === "paused");
        setError(f.error && f.error !== "paused" ? { text: f.error, kind: f.kind } : null);
        if (!f.error) onDone();
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  const start = () => {
    setStarted(true);
    setPaused(false);
    setError(null);
    setRetryIn(null);
    setProgress({ id: "images", stage: "download", done: 0, total: status.download, speed: 0 });
    imagesInstall().catch((e) => {
      setProgress(null);
      setError({ text: String(e), kind: "other" });
    });
  };

  // Пропал интернет — повторяем сами: скачанное не пропадает.
  useEffect(() => {
    if (error?.kind !== "net") return setRetryIn(null);
    setRetryIn(NET_RETRY);
    const timer = setInterval(() => setRetryIn((s) => (s == null ? s : s - 1)), 1000);
    return () => clearInterval(timer);
  }, [error]);
  useEffect(() => {
    if (retryIn !== null && retryIn <= 0) start();
  }, [retryIn]);

  const noSpace = status.disk > status.free;

  return (
    <>
      <p className="muted">
        {t(
          "Для картинок нужен отдельный движок. Он большой, ставится один раз и только если вы им пользуетесь — чату он не нужен.",
          "Pictures need a separate engine. It is large, installs once, and only if you use it — the chat doesn't need it.",
        )}
      </p>
      <InstallScreen
        items={[
          {
            id: "images",
            title: t("Движок картинок", "Image engine"),
            why: t(
              `рисует картинки на вашей видеокарте; займёт на диске ${formatBytes(status.disk)}`,
              `draws pictures on your graphics card; takes ${formatBytes(status.disk)} on disk`,
            ),
            technical: "uv, Python 3.12, PyTorch (CUDA), ComfyUI",
            size: status.download,
            state: error ? "error" : progress ? "active" : "waiting",
            progress: progress ?? last,
          },
        ]}
        started={started || !!progress}
        paused={paused}
        error={error ? { ...error, retryIn } : null}
        onStart={start}
        onPause={() => taskPause("engine:images")}
        extraSeconds={PACKAGES_SECONDS}
        tips={[
          t(
            "После установки выберите модель для картинок — подскажу, какая пойдёт на вашей видеокарте.",
            "After installing, pick an image model — I'll tell you which one will run on your graphics card.",
          ),
          t(
            "Картинки рисуются прямо на вашем компьютере: описания никуда не отправляются.",
            "Pictures are drawn right on your computer: descriptions aren't sent anywhere.",
          ),
          t(
            "Установку можно поставить на паузу, пока идёт скачивание, — продолжим с того же места.",
            "You can pause the install while it downloads — we'll continue from the same point.",
          ),
          t(
            "Готовые картинки будут лежать в папке «Изображения\\Ollivo».",
            "Finished pictures will be in the “Pictures\\Ollivo” folder.",
          ),
        ]}
      />
      {noSpace && (
        <p className="error">
          {t(
            `На диске с программой свободно ${formatBytes(status.free)}, а нужно ${formatBytes(status.disk)}. Освободите место или перенесите папку программы в «Настройках».`,
            `The program's disk has ${formatBytes(status.free)} free, but ${formatBytes(status.disk)} is needed. Free up space or move the program folder in “Settings”.`,
          )}
        </p>
      )}
    </>
  );
}

const SHAPES: { id: ImageShape; name: () => string }[] = [
  { id: "square", name: () => t("Квадрат", "Square") },
  { id: "portrait", name: () => t("Портрет", "Portrait") },
  { id: "landscape", name: () => t("Пейзаж", "Landscape") },
];

const QUALITIES: { id: ImageQuality; name: () => string; hint: () => string }[] = [
  { id: "fast", name: () => t("Быстро", "Fast"), hint: () => t("набросок: проверить идею", "a sketch: to try an idea") },
  { id: "normal", name: () => t("Обычно", "Normal"), hint: () => t("хороший выбор почти всегда", "a good choice almost always") },
  { id: "best", name: () => t("Качественно", "Best"), hint: () => t("больше деталей, дольше", "more detail, takes longer") },
];

const FORM_KEY = "ollivo.images.form";

interface Form {
  model: string | null;
  shape: ImageShape;
  quality: ImageQuality;
  count: number;
}

const savedForm = (): Form => {
  const base: Form = { model: null, shape: "square", quality: "normal", count: 1 };
  try {
    return { ...base, ...JSON.parse(localStorage.getItem(FORM_KEY) ?? "{}") };
  } catch {
    return base;
  }
};

/** «12 с», «1 мин 20 с». */
function seconds(s: number): string {
  if (s < 60) return t(`${Math.max(1, Math.round(s))} с`, `${Math.max(1, Math.round(s))} s`);
  const m = Math.floor(s / 60);
  const r = Math.round(s % 60);
  return r ? t(`${m} мин ${r} с`, `${m} min ${r} s`) : t(`${m} мин`, `${m} min`);
}

/** Идущая генерация: ход, оценка и когда началась — для «осталось». */
interface Run {
  progress: DrawProgress | null;
  estimate: number | null;
  started: number;
  pictures: string[];
}

function Studio({ onGo }: { onGo: (tab: "models") => void }) {
  const [models, setModels] = useState<ImageModel[] | null>(null);
  const [form, setForm] = useState<Form>(savedForm);
  const [prompt, setPrompt] = useState("");
  const [estimate, setEstimate] = useState<DrawEstimate | null>(null);
  const [run, setRun] = useState<Run | null>(null);
  const [done, setDone] = useState<string[]>([]);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [refusal, setRefusal] = useState<string | null>(null);
  const [galleryKey, setGalleryKey] = useState(0);
  const [now, setNow] = useState(Date.now());

  const loadModels = () => imagesModels().then(setModels, () => setModels([]));

  useEffect(() => {
    loadModels();
    // Открыли раздел посреди генерации: ход придёт со следующим шагом.
    tasksRunning().then((ids) => ids.includes(DRAW_TASK) && setRun({ progress: null, estimate: null, started: Date.now(), pictures: [] }));
    const subs = [
      onDrawProgress((p) => setRun((r) => (r ? { ...r, progress: p } : { progress: p, estimate: null, started: Date.now(), pictures: [] }))),
      onDrawPicture((path) => setRun((r) => (r ? { ...r, pictures: [...r.pictures, path] } : r))),
      onDrawFinished((f) => {
        setRun(null);
        setDone(f.files);
        setProblem(f.problem);
        setGalleryKey((k) => k + 1);
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  // Тикаем раз в секунду, пока рисует: «осталось» считается от оценки до старта.
  useEffect(() => {
    if (!run) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [!!run]);

  const usable = (models ?? []).filter((m) => m.family);
  const model = usable.find((m) => m.path === form.model) ?? usable[0] ?? null;

  useEffect(() => {
    try {
      localStorage.setItem(FORM_KEY, JSON.stringify(form));
    } catch {
      // Нет хранилища — форма просто не запомнится.
    }
  }, [form]);

  // Оценка времени — при каждом изменении формы; описание влияет только на «по-английски ли».
  useEffect(() => {
    if (!model) return setEstimate(null);
    const req = { model: model.path, prompt, shape: form.shape, quality: form.quality, count: form.count };
    const timer = setTimeout(() => imagesEstimate(req).then(setEstimate, () => setEstimate(null)), 250);
    return () => clearTimeout(timer);
  }, [model?.path, form.shape, form.quality, form.count, prompt, run === null]);

  if (!models) return <p className="muted">{t("Смотрю модели…", "Looking at models…")}</p>;

  const set = (patch: Partial<Form>) => setForm((f) => ({ ...f, ...patch }));

  const draw = async () => {
    if (!model || !prompt.trim()) return;
    setProblem(null);
    setRefusal(null);
    setDone([]);
    try {
      await imagesDraw({ model: model.path, prompt, shape: form.shape, quality: form.quality, count: form.count });
      setRun({ progress: null, estimate: estimate?.seconds ?? null, started: Date.now(), pictures: [] });
    } catch (e) {
      setRefusal(String(e));
    }
  };

  if (usable.length === 0) {
    return (
      <Pick
        models={models}
        onReady={(path) => {
          set({ model: path });
          loadModels();
        }}
        onGo={onGo}
      />
    );
  }

  const elapsed = run ? (now - run.started) / 1000 : 0;
  const left = run?.estimate != null ? run.estimate - elapsed : null;
  const p = run?.progress;
  const share = p && p.count ? (p.image + (p.max ? p.value / p.max : 0)) / p.count : 0;

  return (
    <div className="studio">
      <div className="card form draw-form">
        <label>
          {t("Что нарисовать", "What to draw")}
          <textarea
            rows={3}
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) draw();
            }}
            placeholder={t(
              "По-английски, например: a red cat in a hat sitting on a windowsill, watercolor",
              "For example: a red cat in a hat sitting on a windowsill, watercolor",
            )}
            disabled={!!run}
          />
        </label>
        {estimate?.cyrillic && (
          <p className="warn small">
            {t(
              "Модели картинок понимают только английский: по-русски выйдет случайная картинка. Напишите описание по-английски — можно попросить перевести в чате.",
              "Image models only understand English: other languages give a random picture. Write the description in English.",
            )}
          </p>
        )}

        <div className="draw-options">
          <div className="seg" role="radiogroup" aria-label={t("Форма", "Shape")}>
            {SHAPES.map((s) => (
              <button key={s.id} role="radio" aria-checked={form.shape === s.id} className={form.shape === s.id ? "active" : ""} onClick={() => set({ shape: s.id })} disabled={!!run}>
                {s.name()}
              </button>
            ))}
          </div>
          <div className="seg" role="radiogroup" aria-label={t("Качество", "Quality")}>
            {QUALITIES.map((q) => (
              <button
                key={q.id}
                role="radio"
                aria-checked={form.quality === q.id}
                title={q.hint()}
                className={form.quality === q.id ? "active" : ""}
                onClick={() => set({ quality: q.id })}
                disabled={!!run}
              >
                {q.name()}
              </button>
            ))}
          </div>
          <div className="seg" role="radiogroup" aria-label={t("Сколько вариантов", "How many variants")} title={t("Сколько вариантов", "How many variants")}>
            {[1, 2, 3, 4].map((n) => (
              <button key={n} role="radio" aria-checked={form.count === n} className={form.count === n ? "active" : ""} onClick={() => set({ count: n })} disabled={!!run}>
                {n === 1 ? t("1 вариант", "1 variant") : n}
              </button>
            ))}
          </div>
        </div>

        {usable.length > 1 && (
          <label>
            {t("Модель", "Model")}
            <select value={model?.path ?? ""} onChange={(e) => set({ model: e.target.value })} disabled={!!run}>
              {usable.map((m) => (
                <option key={m.path} value={m.path}>
                  {m.title ?? m.file} — {m.info.family}
                </option>
              ))}
            </select>
          </label>
        )}

        <div className="actions">
          {run ? (
            <button className="secondary" onClick={() => imagesStop()}>
              {t("Остановить", "Stop")}
            </button>
          ) : (
            <button onClick={draw} disabled={!prompt.trim()}>
              {t("Нарисовать", "Draw")}
            </button>
          )}
          {!run && estimate && (
            <span className="muted small">
              {t("примерно", "about")} {seconds(estimate.seconds)}
              {estimate.cold && t(" — первый раз дольше: запускается движок", " — longer the first time: the engine starts")}
              {usable.length === 1 && model && ` · ${model.title ?? model.file}`}
            </span>
          )}
        </div>
        {refusal && <p className="error small">{refusal}</p>}
      </div>

      {run && (
        <div className="card draw-run">
          <p>
            {!p || p.stage === "start"
              ? t("Запускаю движок картинок — это до минуты…", "Starting the image engine — up to a minute…")
              : p.stage === "load"
                ? t("Загружаю модель в видеокарту…", "Loading the model into the graphics card…")
                : p.count > 1
                  ? t(`Рисую ${p.image + 1} из ${p.count}…`, `Drawing ${p.image + 1} of ${p.count}…`)
                  : t("Рисую…", "Drawing…")}
          </p>
          {p?.stage === "draw" ? <progress max={1} value={share} /> : <progress />}
          <p className="muted small">
            {left != null && left > 1
              ? t(`Осталось примерно ${seconds(left)}.`, `About ${seconds(left)} left.`)
              : t(`Идёт ${seconds(elapsed)}.`, `Running for ${seconds(elapsed)}.`)}{" "}
            {t(
              "Модель чата пока отдыхает — проснётся сама, когда вы её спросите.",
              "The chat model is resting — it wakes up by itself when you ask it something.",
            )}
          </p>
          {run.pictures.length > 0 && <Grid pictures={run.pictures} big />}
        </div>
      )}

      {problem && <ProblemBox problem={problem} />}

      {!run && done.length > 0 && <Grid pictures={done} big />}

      <Gallery key={galleryKey} />
    </div>
  );
}

function ProblemBox({ problem }: { problem: Problem }) {
  const [details, setDetails] = useState(false);
  return (
    <div className="card">
      <p className="error">{problem.text}</p>
      {problem.hint && <p className="small">{problem.hint}</p>}
      <div className="actions">
        <button className="link small" onClick={() => setDetails(!details)}>
          {details ? t("Скрыть подробности", "Hide details") : t("Подробности", "Details")}
        </button>
        <button className="link small" onClick={() => openReport({ kind: "other", error: problem.details })}>
          {t("Сообщить о проблеме", "Report a problem")}
        </button>
      </div>
      {details && <pre className="log">{problem.details}</pre>}
    </div>
  );
}

/** Нет ни одной модели, которую умеем запускать: подборка, «у меня уже есть файл». */
function Pick({ models, onReady, onGo }: { models: ImageModel[]; onReady: (path: string) => void; onGo: (tab: "models") => void }) {
  const [picks, setPicks] = useState<ImagePick[] | null>(null);
  const [progress, setProgress] = useState<Record<string, DownloadProgress | null>>({});
  const [errors, setErrors] = useState<Record<string, string | null>>({});
  const [adding, setAdding] = useState<string | null>(null);

  const taskId = (p: ImagePick) => `model:${p.repo}/${p.file}`;

  useEffect(() => {
    imagesPicks().then(setPicks, () => setPicks([]));
    tasksRunning().then((ids) =>
      setProgress(Object.fromEntries(ids.filter((id) => id.startsWith("model:")).map((id) => [id, null]))),
    );
    const subs = [
      onDownloadProgress((p) => setProgress((all) => (p.id in all || p.id.startsWith("model:") ? { ...all, [p.id]: p } : all))),
      onDownloadFinished((f) => {
        setProgress((all) => {
          const { [f.id]: _, ...rest } = all;
          return rest;
        });
        setErrors((all) => ({ ...all, [f.id]: f.error }));
        if (!f.error && f.result) onReady(f.result);
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  const start = async (p: ImagePick) => {
    setErrors((all) => ({ ...all, [taskId(p)]: null }));
    setProgress((all) => ({ ...all, [taskId(p)]: null }));
    try {
      await catalogDownload(p.repo, p.file, p.sha256, p.title, p.license);
    } catch (e) {
      setProgress((all) => {
        const { [taskId(p)]: _, ...rest } = all;
        return rest;
      });
      setErrors((all) => ({ ...all, [taskId(p)]: String(e) }));
    }
  };

  const pickFile = async () => {
    const picked = await open({ multiple: false, filters: [{ name: t("Модели картинок", "Image models"), extensions: ["safetensors"] }] });
    if (typeof picked !== "string") return;
    setAdding(t("Смотрю файл…", "Looking at the file…"));
    try {
      const [r] = await modelsAdd([picked]);
      if (r?.error) setAdding(r.error);
      else {
        setAdding(null);
        onReady(picked);
      }
    } catch (e) {
      setAdding(String(e));
    }
  };

  const unsupported = models.filter((m) => !m.family);

  return (
    <>
      <p>
        {t(
          "Движок готов. Теперь нужна модель, которая умеет рисовать. Вот проверенные — скачается один раз:",
          "The engine is ready. Now you need a model that can draw. Here are proven ones — it downloads once:",
        )}
      </p>
      {!picks && <p className="muted">{t("Смотрю, что пойдёт на этом компьютере…", "Checking what will run on this computer…")}</p>}
      {picks?.map((p) => {
        const id = taskId(p);
        const running = id in progress;
        const pr = progress[id];
        const err = errors[id];
        return (
          <div key={p.id} className="card pick">
            <div className="model-title">
              <Light light={p.light} />
              <b>{p.title}</b>
              <span className="muted small">{formatBytes(p.size)}</span>
            </div>
            <p className="small">{p.why}</p>
            <p className="muted small">
              {p.fits}; {t("картинка — примерно", "a picture takes about")} {seconds(p.seconds)}.{" "}
              <License code={p.license} repo={p.repo} />
            </p>
            {p.downloaded ? (
              <button onClick={() => onReady(p.downloaded!)}>{t("Выбрать", "Choose")}</button>
            ) : running ? (
              <>
                {pr?.total ? <progress max={pr.total} value={pr.done} /> : <progress />}
                <div className="actions">
                  <span className="muted small">
                    {pr?.phase === "verifying"
                      ? t("Проверяю, что файл пришёл целым…", "Checking the file arrived intact…")
                      : pr?.total
                        ? `${formatBytes(pr.done)} ${t("из", "of")} ${formatBytes(pr.total)}${pr.speed > 0 ? `, ${t("осталось", "left")} ${formatEta((pr.total - pr.done) / pr.speed)}` : ""}`
                        : t("Подключаюсь…", "Connecting…")}
                  </span>
                  <button className="link small" onClick={() => taskPause(id)}>
                    {t("Пауза", "Pause")}
                  </button>
                </div>
              </>
            ) : (
              <div className="actions">
                <button onClick={() => start(p)} disabled={p.light === "red"}>
                  {err === "paused" ? t("Продолжить", "Resume") : t("Скачать", "Download")}
                </button>
                {err && err !== "paused" && <span className="error small">{err}</span>}
              </div>
            )}
          </div>
        );
      })}

      <div className="card">
        <p>{t("Уже есть своя модель?", "Already have your own model?")}</p>
        <p className="muted small">
          {t(
            "Подойдут модели Stable Diffusion 1.5 и SDXL одним файлом .safetensors — например, с HuggingFace или Civitai. Flux и SD 3.5 научимся запускать позже.",
            "Stable Diffusion 1.5 and SDXL models in a single .safetensors file work — for example, from HuggingFace or Civitai. Flux and SD 3.5 will come later.",
          )}
        </p>
        <div className="actions">
          <button className="secondary" onClick={pickFile}>
            {t("Выбрать файл", "Choose a file")}
          </button>
          <button className="link" onClick={() => onGo("models")}>
            {t("Все модели", "All models")}
          </button>
        </div>
        {adding && <p className="small">{adding}</p>}
      </div>

      {unsupported.length > 0 && (
        <div className="card">
          <p className="small">{t("Эти модели уже есть, но пока не запускаются:", "These models are here, but can't run yet:")}</p>
          <ul className="small plain">
            {unsupported.map((m) => (
              <li key={m.path}>
                <b>{m.title ?? m.file}</b> <span className="muted">— {m.why}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}

/** Уменьшенные копии уже загруженных картинок: галерея перерисовывается часто. */
const thumbs = new Map<string, string>();

function Thumb({ path, side }: { path: string; side: number }) {
  const key = `${side}:${path}`;
  const [src, setSrc] = useState<string | null>(thumbs.get(key) ?? null);
  const [lost, setLost] = useState(false);
  useEffect(() => {
    if (thumbs.has(key)) return setSrc(thumbs.get(key)!);
    imagesThumb(path, side).then(
      (s) => {
        thumbs.set(key, s);
        setSrc(s);
      },
      () => setLost(true),
    );
  }, [key]);
  const name = path.split(/[\\/]/).pop();
  if (lost) return <div className="thumb-lost small muted">{name}</div>;
  return src ? <img src={src} alt={name} /> : <div className="thumb-wait" />;
}

/** Плитки картинок: нажатие — открыть в программе просмотра, значок — показать в папке. */
function Grid({ pictures, big }: { pictures: string[]; big?: boolean }) {
  return (
    <div className={big ? "pictures big" : "pictures"}>
      {pictures.map((path) => (
        <figure key={path}>
          <button className="picture" title={t("Открыть", "Open")} onClick={() => imagesOpen(path)}>
            <Thumb path={path} side={big ? 768 : 320} />
          </button>
          <button className="icon-button reveal" title={t("Показать в папке", "Show in folder")} onClick={() => imagesReveal(path)}>
            <Icon name="folder" size={16} />
          </button>
        </figure>
      ))}
    </div>
  );
}

/** Сколько плиток показывать сразу: остальное — по «Показать ещё». */
const PAGE = 24;

function Gallery() {
  const [pictures, setPictures] = useState<Picture[] | null>(null);
  const [shown, setShown] = useState(PAGE);

  useEffect(() => {
    imagesGallery().then(setPictures, () => setPictures([]));
  }, []);

  if (!pictures || pictures.length === 0) return null;

  return (
    <section className="gallery">
      <div className="page-head">
        <h3>
          {t("Готовые картинки", "Finished pictures")}{" "}
          <span className="muted small">
            {pictures.length} {pl(pictures.length, ["картинка", "картинки", "картинок"], ["picture", "pictures"])}
            {pictures.length >= 200 && "+"}
          </span>
        </h3>
        <button className="secondary" onClick={() => imagesOpenFolder()}>
          <Icon name="folder" size={16} /> {t("Открыть папку", "Open folder")}
        </button>
      </div>
      <Grid pictures={pictures.slice(0, shown).map((p) => p.path)} />
      {pictures.length > shown && (
        <div className="actions center">
          <button className="link" onClick={() => setShown(shown + PAGE)}>
            {t("Показать ещё", "Show more")}
          </button>
        </div>
      )}
    </section>
  );
}
