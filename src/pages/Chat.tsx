import { useEffect, useRef, useState } from "react";
import {
  chatPresets,
  chatsGet,
  chatsSave,
  llmChat,
  llmChatStop,
  llmStart,
  llmStatus,
  onLlmAnswer,
  onLlmState,
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
import { wordsPerSecond } from "../words";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** Реплика в окне: у ответа модели ещё есть числа и ошибка. */
interface Line extends Msg {
  stats?: LlmStats | null;
  problem?: Problem | null;
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

  // Свежие реплики для сохранения: обработчик событий помнит только первый рендер.
  const linesRef = useRef<Line[]>([]);
  linesRef.current = lines;
  const wasAnswering = useRef(false);
  /** Разговор, который мы сами только что записали, — перечитывать его не надо. */
  const savedId = useRef<string | null>(null);

  useEffect(() => {
    llmStatus().then(setLlm);
    chatPresets().then((p) => {
      setRoles(p.roles);
      setStyles(p.styles);
    });
    const subs = [
      // Модель могли запустить или остановить на вкладке «Модели».
      onLlmState(setLlm),
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
        .filter((l) => l.content.trim())
        .map(({ role, content }) => ({ role, content }));
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
        talk.map(({ role, content }) => ({ role, content })),
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
    if (!text || answering) return;
    setDraft("");
    ask([...lines, { role: "user", content: text }]);
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
              <p className="answer">{l.content}</p>
            ) : (
              <Answer text={l.content || (answering && i === lines.length - 1 ? "…" : "")} />
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
                {wordsPerSecond(l.content, l.stats.tokens, l.stats.speed)}
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
        <div className="ask">
          <textarea
            rows={3}
            value={draft}
            placeholder="Ваш вопрос. Enter — отправить, Shift+Enter — новая строка."
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={keys}
          />
          <div className="actions">
            {answering ? (
              <button onClick={() => llmChatStop()}>Остановить</button>
            ) : (
              <button onClick={send} disabled={!draft.trim()}>
                Отправить
              </button>
            )}
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
