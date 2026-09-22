import { useEffect, useRef, useState } from "react";
import {
  llmChat,
  llmChatStop,
  llmStatus,
  onLlmAnswer,
  onLlmState,
  onLlmToken,
  type LlmState,
  type LlmStats,
  type Msg,
} from "../api";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** Реплика в окне: у ответа модели ещё есть числа и пометка «пишет». */
interface Line extends Msg {
  stats?: LlmStats | null;
  error?: string | null;
}

export default function Chat({ onGoToModels }: { onGoToModels: () => void }) {
  const [llm, setLlm] = useState<LlmState | null>(null);
  const [lines, setLines] = useState<Line[]>([]);
  const [draft, setDraft] = useState("");
  const [answering, setAnswering] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);

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

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [lines]);

  const send = async () => {
    const text = draft.trim();
    if (!text || answering) return;
    const talk: Line[] = [...lines, { role: "user", content: text }];
    setLines([...talk, { role: "assistant", content: "" }]);
    setDraft("");
    setAnswering(true);
    try {
      await llmChat(talk.map(({ role, content }) => ({ role, content })));
    } catch (e) {
      setAnswering(false);
      setLines((prev) => [...prev.slice(0, -1), { role: "assistant", content: "", error: String(e) }]);
    }
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
      <h2>Чат</h2>

      <div className="talk">
        {lines.length === 0 && <p className="muted">Спросите что угодно — модель отвечает прямо на вашем компьютере.</p>}
        {lines.map((l, i) => (
          <div key={i} className={l.role === "user" ? "line you" : "line bot"}>
            <p className="answer">{l.content || (answering && i === lines.length - 1 ? "…" : "")}</p>
            {l.error && <p className="error">{l.error}</p>}
            {l.stats && l.stats.tokens > 0 && (
              <p className="muted small">
                {l.stats.tokens} токенов, {Math.round(l.stats.speed)} ток/с
              </p>
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
          {lines.length > 0 && !answering && (
            <button className="secondary" onClick={() => setLines([])}>
              Начать заново
            </button>
          )}
          <span className="muted small grow-right">{fileName(llm.model ?? "")}</span>
        </div>
      </div>
    </>
  );
}
