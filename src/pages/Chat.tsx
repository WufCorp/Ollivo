import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ATTACH_EXTENSIONS,
  SPEECH_PARTS,
  onSpeechProgress,
  partsStatus,
  speechDictate,
  speechFile,
  speechStop,
  attachFile,
  attachNeeds,
  attachPreview,
  attachTrim,
  formatBytes,
  onDownloadFinished,
  onDownloadProgress,
  visionDownload,
  visionOffer,
  type Attachment,
  type VisionOffer,
  type Verdict,
  catalogDownload,
  LIGHTS,
  chatPresets,
  chatsGet,
  chatsSave,
  llmChat,
  llmChatStop,
  llmStart,
  llmStatus,
  onLlmAnswer,
  onLlmState,
  onLlmThought,
  onLlmToken,
  onLlmCalling,
  onLlmStep,
  onLlmWrite,
  projectOpen,
  projectUndo,
  projectWriteAnswer,
  type FileMode,
  type Listing,
  type WriteAsk,
  type Chat as Talk,
  type ChatRole,
  type ChatStyle,
  type LlmState,
  type LlmStats,
  type Msg,
  type Problem,
} from "../api";
import Answer, { copyText } from "../components/Answer";
import ProblemCard from "../components/ProblemCard";
import PartsSetup from "../components/PartsSetup";
import { Mentions, ModeSwitch, Steps, WriteCard, claimsChanges, matchFiles, mentionAt } from "../components/Project";
import { record, type Recording } from "../recorder";
import { crashActions } from "../components/RunningModel";
import { memoryPages, plural, wordsPerSecond } from "../words";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** Картинка из папки программы; пропала — вместо неё имя файла. */
function Thumb({ file }: { file: Attachment }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    if (file.path) attachPreview(file.path).then(setSrc, () => setSrc(null));
  }, [file.path]);
  return src ? <img className="thumb" src={src} alt={file.name} title={file.name} /> : <>🖼 {file.name}</>;
}

/** Плашки приложенных документов и картинок; `onRemove` — у ещё не отправленных. */
function Files({ files, onRemove }: { files: Attachment[]; onRemove?: (i: number) => void }) {
  return (
    <div className="files">
      {files.map((f, i) => (
        <span key={i} className={f.kind === "image" ? "file image" : "file"}>
          {f.kind === "image" ? (
            <Thumb file={f} />
          ) : (
            <>
              {f.kind === "audio" ? "🎧" : "📄"} {f.name} · {f.trimmed ? "только начало, " : ""}
              {memoryPages(f.tokens)}
            </>
          )}
          {onRemove && (
            <button className="link" title="Убрать" onClick={() => onRemove(i)}>
              ✕
            </button>
          )}
        </span>
      ))}
    </div>
  );
}

/** Картинки для модели, которая их не видит, и что с этим делаем. `offer`: `undefined` —
 *  ещё узнаём, `null` — узнать не вышло. `task` — идущая загрузка: сама модель из подборки
 *  (`step: "model"`) или зрение. `target` — модель, на которую переходим; нет — остаёмся на этой. */
interface Eyes {
  images: Attachment[];
  offer?: VisionOffer | null;
  task?: string;
  step?: "model" | "vision";
  done?: number;
  total?: number;
  target?: string;
  restarting?: boolean;
  error?: string;
}

/** Что сказать, когда модель не видит приложенную картинку, и какую кнопку дать.
 *  Порядок — от простого к сложному: зрение этой же модели, своя модель со зрением,
 *  модель из подборки; ничего — в каталог. */
function EyesCard({
  eyes,
  onProjector,
  onLocal,
  onPick,
  onCatalog,
  onClose,
}: {
  eyes: Eyes;
  onProjector: () => void;
  onLocal: () => void;
  onPick: () => void;
  onCatalog: () => void;
  onClose: () => void;
}) {
  const o = eyes.offer;
  const what = eyes.images.length > 1 ? `Картинки (${eyes.images.length})` : `«${eyes.images[0].name}»`;
  // Из подробностей — только скорость («пишет примерно…»): память и слои новичку ни к чему,
  // а медленную модель он должен увидеть до загрузки. Стоит она у разных оценок в разных строках.
  const how = (v: Verdict) => {
    const speed = v.details.find((d) => d.startsWith("пишет"));
    return `${LIGHTS[v.light]} ${v.headline}${speed ? `, ${speed}` : ""}`;
  };
  let text: string;
  let action: { label: string; run: () => void } | null = null;
  if (eyes.restarting) {
    text = eyes.target ? "Запускаю модель, которая видит картинки…" : "Перезапускаю модель со зрением…";
  } else if (eyes.task) {
    const what = eyes.step === "model" ? `Качаю «${o?.pick?.title}»` : "Качаю зрение";
    text = `${what}: ${formatBytes(eyes.done ?? 0)} из ${formatBytes(eyes.total ?? 0)}.`;
  } else if (o === undefined) {
    text = "Ищу, как ей помочь…";
  } else if (o?.projector) {
    text = `Ей можно докачать зрение — ${formatBytes(o.projector.size)}, потом модель перезапустится.`;
    action = { label: "Докачать зрение", run: onProjector };
  } else if (o?.local) {
    text = `Картинки видит ваша «${o.local.name}»: ${how(o.local.verdict)}. Разговор продолжится с ней.`;
    action = { label: "Переключиться на неё", run: onLocal };
  } else if (o?.pick) {
    const p = o.pick;
    const size = p.variant.downloaded ? p.projector.size : p.variant.size + p.projector.size;
    text =
      `Картинки видит «${p.title}» из каталога: ${formatBytes(size)}` +
      `${p.variant.downloaded ? " — докачать зрение" : " вместе со зрением"}, ${how(p.variant.verdict)}. ` +
      "Потом разговор продолжится с ней.";
    action = { label: p.variant.downloaded ? "Докачать и переключиться" : "Скачать и переключиться", run: onPick };
  } else {
    text = "Зрение ей не докачать. Видят картинки модели с пометкой «видит картинки» в каталоге.";
    action = { label: "В каталог", run: onCatalog };
  }
  return (
    <div className="card notice">
      <p>
        {what} — а эта модель пока не видит картинки. {text}
      </p>
      {eyes.task && <progress value={eyes.done ?? 0} max={eyes.total || undefined} />}
      {eyes.error && <p className="error small">{eyes.error}</p>}
      {!eyes.task && !eyes.restarting && o !== undefined && (
        <div className="actions">
          {action && <button onClick={action.run}>{eyes.error ? "Ещё раз" : action.label}</button>}
          <button className="secondary" onClick={onClose}>
            Не надо
          </button>
        </div>
      )}
    </div>
  );
}

/** Подсказка в поле ввода, когда что-то приложено: про то, что именно приложено. */
const ASK_ABOUT: Record<Attachment["kind"], string> = {
  document: "Что сделать с документом? Например: «Перескажи коротко». Можно и ничего не писать.",
  image: "Что спросить про картинку? Например: «Что здесь написано?» Можно и ничего не писать.",
  audio: "Что сделать с записью? Например: «Выпиши главное». Можно и ничего не писать.",
};

/** Реплика в окне: у ответа модели ещё есть числа и ошибка. */
interface Line extends Msg {
  stats?: LlmStats | null;
  problem?: Problem | null;
  /** Как думающая модель рассуждала перед ответом. В историю не пишется: модели
   *  прошлые рассуждения не нужны, а человеку они интересны только сейчас. */
  thought?: string;
  /** Модель готовит обращение к файлам — имя инструмента; пока не выполнено. */
  calling?: string | null;
  /** Модель ждёт разрешения записать файл. */
  write?: WriteAsk | null;
}

/** Путь от корня папки, если файл лежит в ней: так и модели, и человеку понятнее, какой это файл. */
const inFolder = (folder: string | null, path: string) => {
  if (!folder) return null;
  const root = folder.replace(/[\\/]+$/, "") + "\\";
  return path.toLowerCase().startsWith(root.toLowerCase()) ? path.slice(root.length).replace(/\\/g, "/") : null;
};

/** Полный путь файла папки по пути от её корня. */
const fullPath = (folder: string, rel: string) => `${folder.replace(/[\\/]+$/, "")}\\${rel.replace(/\//g, "\\")}`;

/** Ядро отвечает на ошибку готовой `Problem`; строка — значит, сломалось что-то по дороге. */
const asProblem = (e: unknown): Problem =>
  typeof e === "object" && e !== null && "text" in e
    ? (e as Problem)
    : { text: "Не получилось получить ответ.", hint: null, actions: ["retry"], details: String(e) };

export default function Chat({
  chatId,
  onSaved,
  onGoToModels,
  onGo,
  onNewChat,
}: {
  /** Открытый разговор; `null` — новый, ещё не сохранённый. */
  chatId: string | null;
  onSaved: (chat: Talk) => void;
  onGoToModels: () => void;
  onGo: (tab: "catalog" | "computer") => void;
  onNewChat: () => void;
}) {
  const [llm, setLlm] = useState<LlmState | null>(null);
  const [lines, setLines] = useState<Line[]>([]);
  const [title, setTitle] = useState("");
  const [draft, setDraft] = useState("");
  const [answering, setAnswering] = useState(false);
  const [roles, setRoles] = useState<ChatRole[]>([]);
  const [styles, setStyles] = useState<ChatStyle[]>([]);
  // Новый разговор наследует роль и манеру прошлого: переводчику не выбирать их каждый раз.
  const [role, setRole] = useState("helper");
  const [style, setStyle] = useState("balanced");
  const bottom = useRef<HTMLDivElement>(null);
  /** Документы к следующему вопросу. */
  const [files, setFiles] = useState<Attachment[]>([]);
  const [reading, setReading] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);
  /** Документ, который целиком не помещается в память модели: ждёт решения человека. */
  const [tooBig, setTooBig] = useState<{ file: Attachment; room: number } | null>(null);
  const [over, setOver] = useState(false);
  /** Картинки для модели, которая пока не видит: ждут, пока докачается зрение. */
  const [eyes, setEyes] = useState<Eyes | null>(null);
  const eyesRef = useRef<Eyes | null>(null);
  eyesRef.current = eyes;
  /** Не хватает частей (распознавание речи, ffmpeg): зачем понадобились, какие и какие файлы ждут. */
  const [setup, setSetup] = useState<{ why: string; parts: string[]; paths?: string[] } | null>(null);
  const [rec, setRec] = useState<Recording | null>(null);
  const [recSec, setRecSec] = useState(0);
  /** Диктовка записана и распознаётся. */
  const [hearing, setHearing] = useState(false);
  const [transcribing, setTranscribing] = useState<{ name: string; percent: number } | null>(null);
  /** Папка проекта разговора; `null` — обычный чат. Новый разговор её наследует, как роль. */
  const [folder, setFolder] = useState<string | null>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  /** Человек пишет «@…» — список файлов папки; `active` — выбранная строка. */
  const [mention, setMention] = useState<{ start: number; query: string; active: number } | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const folderRef = useRef<string | null>(null);
  folderRef.current = folder;
  /** Как модель обращается с файлами: «Вручную», «Авто», «План». Новый разговор наследует. */
  const [mode, setMode] = useState<FileMode>("ask");
  /** Рабочий режим до «Плана» — в нём «Выполнить план». */
  const working = useRef<FileMode>("ask");
  const pickMode = (m: FileMode) => {
    if (m !== "plan") working.current = m;
    setMode(m);
  };

  // Свежие реплики для сохранения: обработчик событий помнит только первый рендер.
  const linesRef = useRef<Line[]>([]);
  linesRef.current = lines;
  const filesRef = useRef<Attachment[]>([]);
  filesRef.current = files;
  const llmRef = useRef<LlmState | null>(null);
  llmRef.current = llm;
  const wasAnswering = useRef(false);
  /** Меняет последний ответ модели — туда идут куски текста, шаги и вопросы о записи. */
  const setLast = (change: (l: Line) => Line) =>
    setLines((prev) => {
      const last = prev[prev.length - 1];
      if (!last || last.role !== "assistant") return prev;
      return [...prev.slice(0, -1), change(last)];
    });
  /** Разговор, который мы сами только что записали, — перечитывать его не надо. */
  const savedId = useRef<string | null>(null);

  useEffect(() => {
    llmStatus().then(setLlm);
    chatPresets().then((p) => {
      setRoles(p.roles);
      setStyles(p.styles);
    });
    // Файл, перетащенный в окно чата, — то же, что скрепка.
    const drop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "over") setOver(true);
      else if (e.payload.type === "drop") {
        setOver(false);
        attachRef.current(e.payload.paths);
      } else setOver(false);
    });
    // Загрузка зрения: прогресс и итог приходят теми же событиями, что у моделей.
    const eyesProgress = onDownloadProgress((p) => {
      if (p.id === eyesRef.current?.task) setEyes((e) => e && { ...e, done: p.done });
    });
    const eyesDone = onDownloadFinished((f) => {
      const e = eyesRef.current;
      if (f.id !== e?.task) return;
      if (f.error) {
        setEyes({ ...e, task: undefined, error: f.error === "paused" ? "Загрузка прервалась." : f.error });
        return;
      }
      // Модель из подборки скачалась — следом зрение к ней.
      if (e.step === "model" && f.result) {
        visionFor({ ...e, target: f.result }, f.result, e.offer?.pick?.projector.size ?? 0);
        return;
      }
      // Зрение подключается при запуске — перезапускаем модель с теми же настройками.
      setEyes({ ...e, task: undefined, restarting: true });
      const llm = llmRef.current;
      if (e.target) llmStart(e.target);
      else if (llm?.model) llmStart(llm.model, { lighter: llm.lighter });
    });
    const speechProgress = onSpeechProgress((p) => setTranscribing((t) => t && { ...t, percent: p.percent }));
    const subs = [
      drop,
      eyesProgress,
      eyesDone,
      speechProgress,
      // Модель могли запустить или остановить на вкладке «Модели».
      onLlmState(setLlm),
      onLlmThought((text) =>
        setLines((prev) => {
          const last = prev[prev.length - 1];
          if (!last || last.role !== "assistant") return prev;
          return [...prev.slice(0, -1), { ...last, thought: (last.thought ?? "") + text }];
        }),
      ),
      onLlmToken((text) =>
        setLines((prev) => {
          const last = prev[prev.length - 1];
          if (!last || last.role !== "assistant") return prev;
          return [...prev.slice(0, -1), { ...last, content: last.content + text }];
        }),
      ),
      onLlmCalling((name) => setLast((l) => ({ ...l, calling: name }))),
      onLlmWrite((w) => setLast((l) => ({ ...l, write: w }))),
      onLlmStep((step) => {
        setLast((l) => ({ ...l, calling: null, write: null, steps: [...(l.steps ?? []), step] }));
        // Модель создала или удалила файл — список по «@» и счёт над разговором должны это знать.
        const changed = step.kind === "write" || step.kind === "edit" || step.kind === "delete";
        if (changed && step.ok && folderRef.current) projectOpen(folderRef.current).then(setListing, () => {});
      }),
      onLlmAnswer((d) => {
        setAnswering(false);
        setLast((l) => ({ ...l, stats: d.stats, problem: d.problem, calling: null, write: null }));
      }),
    ];
    return () => {
      subs.forEach((s) => s.then((un) => un()));
    };
  }, []);

  // Папку выбрали или открыли разговор с папкой — читаем список файлов.
  useEffect(() => {
    setListing(null);
    setFolderError(null);
    if (folder) projectOpen(folder).then(setListing, (e) => setFolderError(String(e)));
  }, [folder]);

  // Идёт диктовка: часы на кнопке. Две минуты — предел: дальше это уже не вопрос,
  // а запись, и её честнее расшифровать файлом.
  useEffect(() => {
    if (!rec) return;
    const t = setInterval(() => {
      setRecSec(rec.seconds());
      if (rec.seconds() >= 120) micRef.current();
    }, 250);
    return () => clearInterval(t);
  }, [rec]);

  // Модель перезапустилась со зрением — ждавшие картинки встают к вопросу.
  useEffect(() => {
    if (!eyes?.restarting) return;
    if (llm?.state === "crashed") {
      setEyes({ ...eyes, restarting: false, error: "Модель не запустилась." });
      return;
    }
    // Ещё не та модель: «готова» пока прошлая, новая только загружается.
    if (llm?.state !== "ready" || (eyes.target && llm.model !== eyes.target)) return;
    if (llm.vision) {
      setFiles((f) => [...f, ...eyes.images]);
      setEyes(null);
    } else {
      setEyes({ ...eyes, restarting: false, error: "Модель запустилась без зрения — картинки ей не показать." });
    }
  }, [llm]);

  // Открыли другой разговор в меню (или начали новый).
  useEffect(() => {
    // Номер пришёл из нашего же сохранения — на экране уже то, что надо.
    // У нового разговора номера нет: `null === null` не повод оставить старую переписку.
    if (chatId !== null && chatId === savedId.current) return;
    savedId.current = null;
    if (!chatId) {
      setLines([]);
      setTitle("");
      return;
    }
    chatsGet(chatId).then((c) => {
      if (!c) return;
      setLines(c.messages);
      setTitle(c.title);
      setRole(c.role || "helper");
      setStyle(c.style || "balanced");
      setFolder(c.folder ?? null);
      if (c.mode) pickMode(c.mode);
    });
  }, [chatId]);

  /** Сохраняет разговор целиком. */
  const persist = (all: Line[]) => {
    const messages = all
      .filter((l) => l.content.trim() || l.files?.length || l.steps?.length)
      .map(({ role, content, files, steps }) => ({ role, content, files, steps }));
    if (!messages.length) return;
    chatsSave({
      id: chatId ?? "",
      title,
      created: 0,
      updated: 0,
      model: llm?.model ?? null,
      role,
      style,
      folder,
      mode,
      messages,
    }).then((saved) => {
      savedId.current = saved.id;
      setTitle(saved.title);
      onSaved(saved);
    });
  };

  // Ответ дописан (или его оборвали) — сохраняем разговор целиком.
  useEffect(() => {
    if (wasAnswering.current && !answering) persist(linesRef.current);
    wasAnswering.current = answering;
  }, [answering]);

  // Вниз — только когда реплик стало больше или модель пишет ответ: «Вернуть как было»
  // у старой реплики не должно уносить к концу разговора.
  const shown = useRef(0);
  useEffect(() => {
    if (lines.length !== shown.current || answering) bottom.current?.scrollIntoView({ block: "end" });
    shown.current = lines.length;
  }, [lines]);

  /** Спрашивает модель по всему разговору; ответ придёт кусками в `onLlmToken`. */
  const ask = async (talk: Line[], how: FileMode = mode) => {
    // Вопрос виден сразу, пока модель просыпается после простоя.
    if (llmRef.current?.state === "sleeping") {
      setLines(talk);
      if (!(await wake())) return;
    }
    setLines([...talk, { role: "assistant", content: "" }]);
    setAnswering(true);
    try {
      await llmChat(
        talk.map(({ role, content, files, steps }) => ({ role, content, files, steps })),
        role,
        style,
        folder,
        how,
      );
    } catch (e) {
      setAnswering(false);
      setLines((prev) => [...prev.slice(0, -1), { role: "assistant", content: "", problem: asProblem(e) }]);
    }
  };

  /** Модель выгружена после простоя — запускаем её снова и ждём. `false` — не завелась
   *  (что случилось, покажет карточка модели) или будить нечего. */
  const wake = async (): Promise<boolean> => {
    const l = llmRef.current;
    if (l?.state === "ready") return true;
    if (l?.state !== "sleeping" || !l.model) return false;
    let done = (_: boolean) => {};
    const ready = new Promise<boolean>((res) => (done = res));
    const un = await onLlmState((s) => {
      if (s.state === "ready" || s.state === "crashed" || s.state === "stopped") done(s.state === "ready");
    });
    try {
      await llmStart(l.model, { lighter: l.lighter });
      return await ready;
    } finally {
      un();
    }
  };

  const send = () => {
    const text = draft.trim();
    if ((!text && !files.length) || answering || reading) return;
    setDraft("");
    setFiles([]);
    setTooBig(null);
    setFileError(null);
    ask([...lines, { role: "user", content: text, files: files.length ? files : undefined }]);
  };

  /** Сколько памяти модели свободно под документ. Разговор меряем прикидкой по буквам
   *  (2,7 знака на токен, как в ядре), документы — их точным числом. Четверть памяти
   *  оставляем на вопрос и ответ: без неё модель прочтёт документ, но ответить не сможет. */
  const room = (pending: Attachment[]) => {
    const ctx = llmRef.current?.ctx;
    if (!ctx) return Infinity;
    const tokens = (fs?: Attachment[]) => (fs ?? []).reduce((n, f) => n + f.tokens, 0);
    const used =
      linesRef.current.reduce((n, l) => n + l.content.length / 2.7 + tokens(l.files), 0) + tokens(pending);
    return Math.max(0, Math.floor(ctx * 0.75 - used));
  };

  /** Читает файлы по одному; не поместившийся останавливает очередь и спрашивает, что делать. */
  const attach = async (paths: string[]) => {
    if (!paths.length || !(await wake())) return;
    setFileError(null);
    setTooBig(null);
    setReading(true);
    let pending = filesRef.current;
    const blind: Attachment[] = [];
    // Файлы, для которых сначала надо докачать части, и сами части.
    const waiting: string[] = [];
    const parts = new Set<string>();
    let waitingAudio = false;
    try {
      for (const p of paths) {
        let a: Attachment;
        const need = await attachNeeds(p);
        if (need.missing.length) {
          waiting.push(p);
          need.missing.forEach((m) => parts.add(m));
          waitingAudio ||= need.audio;
          continue;
        }
        try {
          if (need.audio) {
            setTranscribing({ name: fileName(p), percent: 0 });
            a = await speechFile(p);
          } else {
            a = await attachFile(p);
            // Файл из папки проекта — под путём от её корня, как его знает модель.
            const rel = inFolder(folderRef.current, p);
            if (rel) a = { ...a, name: rel };
          }
        } catch (e) {
          // «Остановить» — не ошибка.
          if (String(e) !== "отменено") setFileError(`«${fileName(p)}»: ${String(e)}.`);
          continue;
        } finally {
          setTranscribing(null);
        }
        if (a.kind === "image" && !llmRef.current?.vision) {
          blind.push(a);
          continue;
        }
        const free = room(pending);
        if (a.tokens > free) {
          setTooBig({ file: a, room: free });
          break;
        }
        pending = [...pending, a];
        setFiles(pending);
      }
    } finally {
      setReading(false);
    }
    if (waiting.length) {
      // Порядок установки — как у ядра: ffmpeg, движок распознавания, модель.
      const order = ["ffmpeg", ...SPEECH_PARTS];
      setSetup({
        why: waitingAudio ? "расшифровать запись" : "открыть такую картинку",
        parts: order.filter((p) => parts.has(p)),
        paths: waiting,
      });
    }
    if (blind.length) {
      const model = llmRef.current?.model ?? "";
      setEyes({ images: [...(eyesRef.current?.images ?? []), ...blind] });
      visionOffer(model).then(
        (offer) => setEyes((e) => e && { ...e, offer }),
        () => setEyes((e) => e && { ...e, offer: null }),
      );
    }
  };

  /** Качает зрение модели `model`; конец загрузки ловит обработчик выше и запускает её. */
  const visionFor = async (e: Eyes, model: string, size: number) => {
    try {
      const task = await visionDownload(model);
      setEyes({ ...e, task, step: "vision", done: 0, total: size, error: undefined });
    } catch (err) {
      setEyes({ ...e, task: undefined, error: String(err) });
    }
  };

  /** Зрение этой же модели. */
  const getEyes = () => {
    const model = llmRef.current?.model;
    const p = eyes?.offer?.projector;
    if (eyes && model && p) visionFor(eyes, model, p.size);
  };

  /** Своя модель со зрением: переключаемся, разговор продолжается с ней. */
  const switchToSeer = () => {
    const l = eyes?.offer?.local;
    if (!eyes || !l) return;
    setEyes({ ...eyes, target: l.path, restarting: true, error: undefined });
    llmStart(l.path);
  };

  /** Модель из подборки: скачать её, потом зрение, потом переключиться. Уже скачанная
   *  (или скачалась, а зрение сорвалось) — сразу за зрением. */
  const getSeer = async () => {
    const p = eyes?.offer?.pick;
    if (!eyes || !p) return;
    const have = eyes.target ?? p.variant.downloaded;
    if (have) {
      visionFor({ ...eyes, target: have }, have, p.projector.size);
      return;
    }
    try {
      const v = p.variant;
      const task = await catalogDownload(p.repo, v.name, v.sha256, p.title, p.license);
      setEyes({ ...eyes, task, step: "model", done: 0, total: v.size, error: undefined });
    } catch (e) {
      setEyes({ ...eyes, error: String(e) });
    }
  };
  // Обработчик перетаскивания заведён один раз — зовём через ref свежую версию.
  const attachRef = useRef(attach);
  attachRef.current = attach;

  /** Микрофон: первое нажатие — слушать, второе — распознать и дописать в поле ввода. */
  const mic = async () => {
    if (rec) {
      setRec(null);
      setHearing(true);
      try {
        const text = await speechDictate(await rec.stop());
        if (text) setDraft((d) => (d.trim() ? d.trimEnd() + " " : "") + text);
        else setFileError("Не расслышал — попробуйте ещё раз, ближе к микрофону.");
      } catch (e) {
        setFileError(`Не получилось распознать: ${String(e)}.`);
      } finally {
        setHearing(false);
      }
      return;
    }
    setFileError(null);
    const s = await partsStatus(SPEECH_PARTS);
    if (s.missing.length) {
      setSetup({ why: "надиктовать вопрос", parts: SPEECH_PARTS });
      return;
    }
    try {
      setRecSec(0);
      setRec(await record());
    } catch {
      setFileError(
        "Микрофон недоступен: проверьте, что он подключён и что Windows разрешает программам им пользоваться.",
      );
    }
  };
  const micRef = useRef(mic);
  micRef.current = mic;

  const pickFolder = async () => {
    const picked = await open({ directory: true, title: "Папка проекта" });
    if (typeof picked === "string") setFolder(picked);
  };

  /** Файл из списка по «@»: в тексте остаётся ссылка, а сам файл прикладывается — модель
   *  получит его точно, даже если сама открывать файлы не умеет. */
  const pickMention = (path: string) => {
    if (!mention || !folder) return;
    const end = mention.start + 1 + mention.query.length;
    setDraft(`${draft.slice(0, mention.start)}@${path} ${draft.slice(end)}`);
    setMention(null);
    const caret = mention.start + path.length + 2;
    requestAnimationFrame(() => input.current?.setSelectionRange(caret, caret));
    if (!files.some((f) => f.name === path)) attach([fullPath(folder, path)]);
  };

  /** Модель сохранила файл, человек передумал. */
  const undoStep = async (line: number, i: number) => {
    const step = lines[line].steps?.[i];
    if (!step || !folder) return;
    await projectUndo(folder, step);
    const next = lines.map((l, j) =>
      j === line ? { ...l, steps: l.steps?.map((s, k) => (k === i ? { ...s, undone: true } : s)) } : l,
    );
    setLines(next);
    persist(next);
    projectOpen(folder).then(setListing, () => {});
  };

  /** «Выполнить план»: обратно в рабочий режим и просим сделать то, что модель расписала. */
  const runPlan = () => {
    const how = working.current;
    setMode(how);
    ask([...lines, { role: "user", content: "Выполни этот план." }], how);
  };

  const answerWrite = (ok: boolean) => {
    const w = lines[lines.length - 1]?.write;
    if (!w) return;
    projectWriteAnswer(w.id, ok);
    setLast((l) => ({ ...l, write: null }));
  };

  const typed = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setDraft(e.target.value);
    const m = listing ? mentionAt(e.target.value, e.target.selectionStart) : null;
    setMention(m && { ...m, active: 0 });
  };

  const pickFiles = async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Документы, картинки и записи", extensions: ATTACH_EXTENSIONS }],
    });
    if (picked) attach(Array.isArray(picked) ? picked : [picked]);
  };

  /** «Приложить только начало»: сколько поместится. */
  const attachHead = async () => {
    if (!tooBig) return;
    const head = await attachTrim(tooBig.file, tooBig.room);
    setTooBig(null);
    setFiles([...filesRef.current, head]);
  };

  /** «Ответить заново»: убираем последний ответ и спрашиваем то же самое ещё раз. */
  const again = () => {
    if (answering || lines[lines.length - 1]?.role !== "assistant") return;
    ask(lines.slice(0, -1));
  };

  const found = mention && listing ? matchFiles(listing.files, mention.query) : [];

  const keys = (e: React.KeyboardEvent) => {
    // Открыт список по «@»: стрелки выбирают, Enter и Tab — берут файл, Esc — закрывает.
    if (mention && found.length) {
      const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
      if (step) {
        e.preventDefault();
        setMention({ ...mention, active: (mention.active + step + found.length) % found.length });
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pickMention(found[mention.active] ?? found[0]);
        return;
      }
    }
    if (mention && e.key === "Escape") {
      e.preventDefault();
      setMention(null);
      return;
    }
    // Enter отправляет, Shift+Enter — перенос строки.
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  /** Роль тянет за собой подходящую манеру; её потом можно поменять. */
  const pickRole = (id: string) => {
    setRole(id);
    const r = roles.find((x) => x.id === id);
    if (r) setStyle(r.style);
  };

  const roleNow = roles.find((r) => r.id === role);

  if (!llm) return null;

  // Модель не готова: вместо поля ввода — что с ней и что делать.
  // Переписку при этом показываем: старый разговор можно прочитать и без модели.
  const waiting =
    // Выгруженная после простоя модель проснётся сама на первом вопросе — поле ввода остаётся.
    llm.state === "ready" || llm.state === "sleeping" ? null : llm.state === "crashed" && llm.problem ? (
      <ProblemCard problem={llm.problem} on={{ ...crashActions(llm, onGo), models: onGoToModels }} />
    ) : (
      <>
        <p>
          {llm.state === "starting"
            ? `Загружаю ${fileName(llm.model ?? "")} в видеокарту — это займёт немного времени.`
            : lines.length
              ? "Чтобы продолжить разговор, запустите модель."
              : "Чтобы начать разговор, выберите модель и запустите её."}
        </p>
        {llm.state !== "starting" && (
          <div className="actions">
            <button onClick={onGoToModels}>К моделям</button>
          </div>
        )}
      </>
    );

  if (waiting && lines.length === 0) {
    return (
      <>
        <h2>Чат</h2>
        <div className="card">{waiting}</div>
      </>
    );
  }

  const folderBar = folder && (
    <div className="folder-bar">
      <span className="folder-name" title={folder}>
        📁 {listing?.name ?? folder}
      </span>
      {listing && (
        <span className="muted small">
          {listing.truncated ? "больше " : ""}
          {listing.files.length} {plural(listing.files.length, "файл", "файла", "файлов")}
        </span>
      )}
      <button className="link" disabled={answering} onClick={pickFolder}>
        Сменить
      </button>
      <button className="link" disabled={answering} onClick={() => setFolder(null)}>
        Убрать
      </button>
      <ModeSwitch mode={mode} disabled={answering} onChange={pickMode} />
      {folderError && <span className="error small">{folderError}</span>}
      {listing && llm.state === "ready" && !llm.tools && (
        <span className="muted small hint">
          Эта модель сама файлы не открывает — прикладывайте нужные через @ в поле ввода.
        </span>
      )}
    </div>
  );

  return (
    <>
      {folderBar}
      <div className="talk">
        {lines.length === 0 && (
          <p className="muted">
            {folder
              ? mode === "plan"
                ? "Режим «План»: модель изучит файлы и распишет, что сделать, но ничего не изменит."
                : mode === "auto"
                  ? "Режим «Авто»: модель сама создаёт, меняет и удаляет файлы в папке. Любое изменение можно вернуть."
                  : "Спросите про файлы папки: модель откроет нужные сама, а менять их будет только с вашего разрешения."
              : role === "helper" || !roleNow
                ? "Спросите что угодно — модель отвечает прямо на вашем компьютере."
                : `${roleNow.name}: ${roleNow.hint.toLowerCase()}.`}
          </p>
        )}
        {lines.map((l, i) => (
          <div key={i} className={l.role === "user" ? "line you" : "line bot"}>
            {l.role === "user" ? (
              <>
                {l.files?.length ? <Files files={l.files} /> : null}
                {l.content && <p className="answer">{l.content}</p>}
              </>
            ) : (
              <>
                {l.thought && (
                  <details className="thought">
                    <summary>
                      {answering && i === lines.length - 1 && !l.content
                        ? "Модель обдумывает ответ…"
                        : "Как модель рассуждала"}
                    </summary>
                    <p>{l.thought}</p>
                  </details>
                )}
                {l.steps?.length ? (
                  <Steps steps={l.steps} onUndo={folder && !answering ? (k) => undoStep(i, k) : undefined} />
                ) : null}
                <Answer
                  text={l.content || (answering && i === lines.length - 1 && !l.thought && !l.steps?.length ? "…" : "")}
                />
                {answering && i === lines.length - 1 && l.calling && !l.write && (
                  <p className="muted small">
                    {l.calling === "write_file" ? "Модель пишет файл…" : "Модель открывает файлы…"}
                  </p>
                )}
                {l.write && <WriteCard ask={l.write} onAnswer={answerWrite} />}
                {folder && !(answering && i === lines.length - 1) && claimsChanges(l.content, l.steps) && (
                  <p className="warn small">
                    ⚠ Модель пишет, что меняла файлы, но ни одного файла не изменила — проверьте. Что она на
                    самом деле делала, видно в строках над ответом.
                  </p>
                )}
              </>
            )}
            {l.problem && (
              <ProblemCard
                problem={l.problem}
                on={{
                  retry: again,
                  restart: () => llm.model && llmStart(llm.model, { lighter: llm.lighter }),
                  new_chat: onNewChat,
                  models: onGoToModels,
                }}
              />
            )}
            {l.stats && l.stats.tokens > 0 && (
              <p className="muted small">
                {/* Токены ответа включают и рассуждения — слова считаем там же. */}
                {wordsPerSecond(`${l.thought ?? ""} ${l.content}`, l.stats.tokens, l.stats.speed)}
              </p>
            )}
            {/* Кнопки — только у последнего ответа: у каждой реплики они бы мешали читать. */}
            {l.role === "assistant" && !answering && !waiting && i === lines.length - 1 && l.content.trim() && (
              <div className="after">
                <button className="link" onClick={() => copyText(l.content)}>
                  Копировать
                </button>
                <button className="link" onClick={again}>
                  Ответить заново
                </button>
                {folder && mode === "plan" && (
                  <button
                    className="link"
                    title={working.current === "auto" ? "Модель выполнит план сама" : "Каждое изменение — с вашего разрешения"}
                    onClick={runPlan}
                  >
                    Выполнить план
                  </button>
                )}
              </div>
            )}
          </div>
        ))}
        <div ref={bottom} />
      </div>

      {waiting ? (
        <div className="ask">
          <div className="card">{waiting}</div>
        </div>
      ) : (
        <div className={over ? "ask over" : "ask"}>
          {eyes && (
            <EyesCard
              eyes={eyes}
              onProjector={getEyes}
              onLocal={switchToSeer}
              onPick={getSeer}
              onCatalog={() => onGo("catalog")}
              onClose={() => setEyes(null)}
            />
          )}
          {tooBig && (
            <div className="card notice">
              {/* Меньше страницы места — резать нечего, остаётся новый разговор. */}
              {tooBig.room >= 650 ? (
                <>
                  <p>
                    «{tooBig.file.name}» — это {memoryPages(tooBig.file.tokens)}, а в память модели сейчас
                    поместится {memoryPages(tooBig.room)}. Целиком модель его не прочтёт.
                  </p>
                  <div className="actions">
                    <button onClick={attachHead}>Приложить только начало</button>
                    <button className="secondary" onClick={() => setTooBig(null)}>
                      Не прикладывать
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p>
                    Разговор уже занял почти всю память модели — «{tooBig.file.name}» сюда не поместится.
                    В новом разговоре места больше.
                  </p>
                  <div className="actions">
                    <button onClick={onNewChat}>Новый разговор</button>
                    <button className="secondary" onClick={() => setTooBig(null)}>
                      Не прикладывать
                    </button>
                  </div>
                </>
              )}
            </div>
          )}
          {setup && (
            <PartsSetup
              key={setup.parts.join()}
              why={setup.why}
              parts={setup.parts}
              onCancel={() => setSetup(null)}
              onReady={() => {
                const waiting = setup.paths;
                setSetup(null);
                if (waiting) attach(waiting);
              }}
            />
          )}
          {transcribing && (
            <div className="card notice">
              <p>
                Расшифровываю «{transcribing.name}»… {transcribing.percent}%
              </p>
              <progress value={transcribing.percent} max={100} />
              <div className="actions">
                <button className="secondary" onClick={() => speechStop()}>
                  Остановить
                </button>
              </div>
            </div>
          )}
          {files.length > 0 && <Files files={files} onRemove={(i) => setFiles(files.filter((_, j) => j !== i))} />}
          {mention && listing && <Mentions files={found} active={mention.active} onPick={pickMention} />}
          {reading && <p className="muted small">Читаю файл…</p>}
          {fileError && <p className="error small">{fileError}</p>}
          <textarea
            ref={input}
            rows={3}
            value={draft}
            placeholder={
              files.length
                ? ASK_ABOUT[files.every((f) => f.kind === files[0].kind) ? files[0].kind : "document"]
                : folder
                  ? "Вопрос про проект. @ — сослаться на файл. Enter — отправить, Shift+Enter — новая строка."
                  : "Ваш вопрос. Enter — отправить, Shift+Enter — новая строка."
            }
            onChange={typed}
            onKeyDown={keys}
            onBlur={() => setMention(null)}
          />
          <div className="actions">
            {answering ? (
              <button onClick={() => llmChatStop()}>Остановить</button>
            ) : (
              <button onClick={send} disabled={(!draft.trim() && !files.length) || reading}>
                Отправить
              </button>
            )}
            <button
              className="secondary"
              title="Приложить документ или картинку: PDF, Word, текст, код, фото. Файл можно и перетащить в окно."
              disabled={answering || reading || !!eyes?.task}
              onClick={pickFiles}
            >
              📎
            </button>
            <button
              className={folder ? "" : "secondary"}
              title={
                folder
                  ? `Папка проекта: ${folder}. Нажмите, чтобы выбрать другую.`
                  : "Работать с папкой: модель увидит её файлы, сможет их читать, а с вашего разрешения — создавать и менять."
              }
              disabled={answering}
              onClick={pickFolder}
            >
              📁
            </button>
            <button
              className={rec ? "recording" : "secondary"}
              title={rec ? "Закончить и распознать" : "Надиктовать вопрос"}
              disabled={answering || hearing || reading}
              onClick={mic}
            >
              {rec
                ? `⏹ ${Math.floor(recSec / 60)}:${String(Math.floor(recSec % 60)).padStart(2, "0")}`
                : hearing
                  ? "Распознаю…"
                  : "🎤"}
            </button>
            <select
              className="role"
              value={role}
              title={roleNow?.hint}
              disabled={answering}
              onChange={(e) => pickRole(e.target.value)}
            >
              {roles.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
            <div className="seg" role="radiogroup" aria-label="Как отвечать">
              {styles.map((s) => (
                <button
                  key={s.id}
                  role="radio"
                  aria-checked={s.id === style}
                  className={s.id === style ? "active" : ""}
                  title={s.hint}
                  disabled={answering}
                  onClick={() => setStyle(s.id)}
                >
                  {s.name}
                </button>
              ))}
            </div>
            <span className="muted small grow-right model-name" title={llm.model ?? ""}>
              {fileName(llm.model ?? "")}
            </span>
          </div>
        </div>
      )}
    </>
  );
}
