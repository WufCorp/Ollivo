import { useEffect, useRef, useState } from "react";
import {
  chatsGet,
  chatsSave,
  llmChat,
  llmChatStop,
  llmStatus,
  onLlmAnswer,
  onLlmState,
  onLlmToken,
  type Chat as Talk,
  type LlmState,
  type LlmStats,
  type Msg,
} from "../api";
import Answer, { copyText } from "../components/Answer";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** Реплика в окне: у ответа модели ещё есть числа и ошибка. */
interface Line extends Msg {
  stats?: LlmStats | null;
  error?: string | null;
}

export default function Chat({
  chatId,
  onSaved,
  onGoToModels,
}: {
  /** Открытый разговор; `null` — новый, ещё не сохранённый. */
  chatId: string | null;
  onSaved: (chat: Talk) => void;
  onGoToModels: () => void;
}) {
  const [llm, setLlm] = useState<LlmState | null>(null);
  const [lines, setLines] = useState<Line[]>([]);
  const [title, setTitle] = useState("");
  const [draft, setDraft] = useState("");
  const [answering, setAnswering] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);

  // Свежие реплики для сохранения: обработчик событий помнит только первый рендер.
  const linesRef = useRef<Line[]>([]);
  linesRef.current = lines;
  const wasAnswering = useRef(false);
  /** Разговор, который мы сами только что записали, — перечитывать его не надо. */
  const savedId = useRef<string | null>(null);

  useEffect(() => {
    llmStatus().then(setLlm);
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
          return [...prev.slice(0, -1), { ...last, stats: d.stats, error: d.error }];
        });
      }),
    ];
    return () => {
      subs.forEach((s) => s.then((un) => un()));
    };
  }, []);

  // Открыли другой разговор в меню (или начали новый).
  useEffect(() => {
    if (chatId === savedId.current) return;
    if (!chatId) {
      setLines([]);
      setTitle("");
      return;
    }
    chatsGet(chatId).then((c) => {
      if (!c) return;
      setLines(c.messages);
      setTitle(c.title);
    });
  }, [chatId]);

  // Ответ дописан (или его оборвали) — сохраняем разговор целиком.
  useEffect(() => {
    if (wasAnswering.current && !answering) {
      const messages = linesRef.current
        .filter((l) => l.content.trim())
        .map(({ role, content }) => ({ role, content }));
      if (messages.length) {
        chatsSave({ id: chatId ?? "", title, created: 0, updated: 0, model: llm?.model ?? null, messages }).then(
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
      await llmChat(talk.map(({ role, content }) => ({ role, content })));
    } catch (e) {
      setAnswering(false);
      setLines((prev) => [...prev.slice(0, -1), { role: "assistant", content: "", error: String(e) }]);
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

  if (!llm) return null;

  if (llm.state !== "ready") {
    return (
      <>
        <h2>Чат</h2>
        <div className="card">
          <p>
            {llm.state === "starting"
              ? `Загружаю ${fileName(llm.model ?? "")} в видеокарту — это займёт немного времени.`
              : "Чтобы начать разговор, выберите модель и запустите её."}
          </p>
          {llm.state !== "starting" && (
            <div className="actions">
              <button onClick={onGoToModels}>К моделям</button>
            </div>
          )}
        </div>
      </>
    );
  }

  return (
    <>
      <div className="talk">
        {lines.length === 0 && <p className="muted">Спросите что угодно — модель отвечает прямо на вашем компьютере.</p>}
        {lines.map((l, i) => (
          <div key={i} className={l.role === "user" ? "line you" : "line bot"}>
            {l.role === "user" ? (
              <p className="answer">{l.content}</p>
            ) : (
              <Answer text={l.content || (answering && i === lines.length - 1 ? "…" : "")} />
            )}
            {l.error && <p className="error">{l.error}</p>}
            {l.stats && l.stats.tokens > 0 && (
              <p className="muted small">
                {l.stats.tokens} токенов, {Math.round(l.stats.speed)} ток/с
              </p>
            )}
            {/* Кнопки — только у последнего ответа: у каждой реплики они бы мешали читать. */}
            {l.role === "assistant" && !answering && i === lines.length - 1 && l.content.trim() && (
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
          <span className="muted small grow-right">{fileName(llm.model ?? "")}</span>
        </div>
      </div>
    </>
  );
}
