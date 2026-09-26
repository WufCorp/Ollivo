import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ATTACH_EXTENSIONS,
  attachFile,
  attachPreview,
  attachTrim,
  formatBytes,
  onDownloadFinished,
  onDownloadProgress,
  visionDownload,
  visionOffer,
  type Attachment,
  type Projector,
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
import { crashActions } from "../components/RunningModel";
import { memoryPages, wordsPerSecond } from "../words";

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
              📄 {f.name} · {f.trimmed ? "только начало, " : ""}
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

/** Зрение для модели, которая его не имеет. `offer`: `undefined` — ещё узнаём,
 *  `null` — докачать нельзя. `task` — id идущей загрузки. */
interface Eyes {
  images: Attachment[];
  offer?: Projector | null;
  task?: string;
  done?: number;
  restarting?: boolean;
  error?: string;
}

/** Реплика в окне: у ответа модели ещё есть числа и ошибка. */
interface Line extends Msg {
  stats?: LlmStats | null;
  problem?: Problem | null;
  /** Как думающая модель рассуждала перед ответом. В историю не пишется: модели
   *  прошлые рассуждения не нужны, а человеку они интересны только сейчас. */
  thought?: string;
}

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

  // Свежие реплики для сохранения: обработчик событий помнит только первый рендер.
  const linesRef = useRef<Line[]>([]);
  linesRef.current = lines;
  const filesRef = useRef<Attachment[]>([]);
  filesRef.current = files;
  const llmRef = useRef<LlmState | null>(null);
  llmRef.current = llm;
  const wasAnswering = useRef(false);
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
      // Зрение подключается при запуске — перезапускаем модель с теми же настройками.
      setEyes({ ...e, task: undefined, restarting: true });
      const llm = llmRef.current;
      if (llm?.model) llmStart(llm.model, { lighter: llm.lighter });
    });
    const subs = [
      drop,
      eyesProgress,
      eyesDone,
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
      onLlmAnswer((d) => {
        setAnswering(false);
        setLines((prev) => {
          const last = prev[prev.length - 1];
          if (!last || last.role !== "assistant") return prev;
          return [...prev.slice(0, -1), { ...last, stats: d.stats, problem: d.problem }];
        });
      }),
    ];
    return () => {
      subs.forEach((s) => s.then((un) => un()));
    };
  }, []);

  // Модель перезапустилась со зрением — ждавшие картинки встают к вопросу.
  useEffect(() => {
    if (!eyes?.restarting || llm?.state !== "ready") return;
    if (llm.vision) {
      setFiles((f) => [...f, ...eyes.images]);
      setEyes(null);
    } else {
      setEyes({ ...eyes, restarting: false, error: "Зрение скачалось, но модель его не подключила." });
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
    });
  }, [chatId]);

  // Ответ дописан (или его оборвали) — сохраняем разговор целиком.
  useEffect(() => {
    if (wasAnswering.current && !answering) {
      const messages = linesRef.current
        .filter((l) => l.content.trim() || l.files?.length)
        .map(({ role, content, files }) => ({ role, content, files }));
      if (messages.length) {
        chatsSave({
          id: chatId ?? "",
          title,
          created: 0,
          updated: 0,
          model: llm?.model ?? null,
          role,
          style,
          messages,
        }).then(
          (saved) => {
            savedId.current = saved.id;
            setTitle(saved.title);
            onSaved(saved);
          },
        );
      }
    }
    wasAnswering.current = answering;
  }, [answering]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [lines]);

  /** Спрашивает модель по всему разговору; ответ придёт кусками в `onLlmToken`. */
  const ask = async (talk: Line[]) => {
    setLines([...talk, { role: "assistant", content: "" }]);
    setAnswering(true);
    try {
      await llmChat(
        talk.map(({ role, content, files }) => ({ role, content, files })),
        role,
        style,
      );
    } catch (e) {
      setAnswering(false);
      setLines((prev) => [...prev.slice(0, -1), { role: "assistant", content: "", problem: asProblem(e) }]);
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
    if (!paths.length || llmRef.current?.state !== "ready") return;
    setFileError(null);
    setTooBig(null);
    setReading(true);
    let pending = filesRef.current;
    const blind: Attachment[] = [];
    try {
      for (const p of paths) {
        let a: Attachment;
        try {
          a = await attachFile(p);
        } catch (e) {
          setFileError(`«${fileName(p)}»: ${String(e)}.`);
          continue;
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
    if (blind.length) {
      const model = llmRef.current?.model ?? "";
      setEyes({ images: [...(eyesRef.current?.images ?? []), ...blind] });
      visionOffer(model).then(
        (offer) => setEyes((e) => e && { ...e, offer }),
        () => setEyes((e) => e && { ...e, offer: null }),
      );
    }
  };

  const getEyes = async () => {
    const model = llmRef.current?.model;
    if (!eyes || !model) return;
    try {
      const task = await visionDownload(model);
      setEyes({ ...eyes, task, done: 0, error: undefined });
    } catch (e) {
      setEyes({ ...eyes, error: String(e) });
    }
  };
  // Обработчик перетаскивания заведён один раз — зовём через ref свежую версию.
  const attachRef = useRef(attach);
  attachRef.current = attach;

  const pickFiles = async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Документы, текст и картинки", extensions: ATTACH_EXTENSIONS }],
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

  const keys = (e: React.KeyboardEvent) => {
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
    llm.state === "ready" ? null : llm.state === "crashed" && llm.problem ? (
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

  return (
    <>
      <div className="talk">
        {lines.length === 0 && (
          <p className="muted">
            {role === "helper" || !roleNow
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
                <Answer text={l.content || (answering && i === lines.length - 1 && !l.thought ? "…" : "")} />
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
            <div className="card notice">
              <p>
                {eyes.images.length > 1 ? `Картинки (${eyes.images.length})` : `«${eyes.images[0].name}»`} — а эта
                модель пока не видит картинки.{" "}
                {eyes.offer === undefined
                  ? "Проверяю, можно ли докачать ей зрение…"
                  : eyes.offer === null
                    ? "Зрение ей не докачать. Видят картинки модели с пометкой «видит картинки» в каталоге."
                    : eyes.restarting
                      ? "Перезапускаю модель со зрением…"
                      : eyes.task
                        ? `Качаю зрение: ${formatBytes(eyes.done ?? 0)} из ${formatBytes(eyes.offer.size)}.`
                        : `Ей можно докачать зрение — ${formatBytes(eyes.offer.size)}, потом модель перезапустится.`}
              </p>
              {eyes.task && eyes.offer && <progress value={eyes.done ?? 0} max={eyes.offer.size} />}
              {eyes.error && <p className="error small">{eyes.error}</p>}
              {!eyes.task && !eyes.restarting && eyes.offer !== undefined && (
                <div className="actions">
                  {eyes.offer ? (
                    <button onClick={getEyes}>{eyes.error ? "Ещё раз" : "Докачать зрение"}</button>
                  ) : (
                    <button onClick={() => onGo("catalog")}>В каталог</button>
                  )}
                  <button className="secondary" onClick={() => setEyes(null)}>
                    Не надо
                  </button>
                </div>
              )}
            </div>
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
          {files.length > 0 && <Files files={files} onRemove={(i) => setFiles(files.filter((_, j) => j !== i))} />}
          {reading && <p className="muted small">Читаю файл…</p>}
          {fileError && <p className="error small">{fileError}</p>}
          <textarea
            rows={3}
            value={draft}
            placeholder={
              files.length
                ? "Что сделать с документом? Например: «Перескажи коротко». Можно и ничего не писать."
                : "Ваш вопрос. Enter — отправить, Shift+Enter — новая строка."
            }
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={keys}
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
