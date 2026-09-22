import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { llmAsk, llmStart, llmStatus, llmStop, onLlmState, type LlmAnswer, type LlmState } from "../api";

const fileName = (p: string) => p.split(/[\/]/).pop() ?? p;

/** Временная проверка движка: выбрать .gguf, запустить, задать вопрос. Чат — фаза 2. */
export default function ModelCard() {
  const [state, setState] = useState<LlmState | null>(null);
  const [prompt, setPrompt] = useState("Привет! Кто ты? Ответь одним предложением.");
  const [answer, setAnswer] = useState<LlmAnswer | null>(null);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    llmStatus().then(setState);
    const sub = onLlmState((s) => {
      setState(s);
      if (s.state !== "ready") setAnswer(null);
    });
    return () => {
      sub.then((un) => un());
    };
  }, []);

  const pick = async () => {
    const path = await open({ filters: [{ name: "Модель GGUF", extensions: ["gguf"] }] });
    if (typeof path !== "string") return;
    setError(null);
    setAnswer(null);
    llmStart(path).catch((e) => setError(String(e)));
  };

  const ask = async () => {
    setAsking(true);
    setError(null);
    try {
      setAnswer(await llmAsk(prompt));
    } catch (e) {
      setError(String(e));
    } finally {
      setAsking(false);
    }
  };

  if (!state) return null;

  return (
    <div className="card form">
      {state.state === "starting" && <p>Загружаю {fileName(state.model ?? "")} в видеокарту…</p>}

      {state.state === "ready" && (
        <>
          <p className="ok">
            ✓ Работает: {fileName(state.model ?? "")}, загрузилась за {state.started_in?.toFixed(1).replace(".", ",")} с
          </p>
          <div className="row">
            <input className="grow-input" value={prompt} onChange={(e) => setPrompt(e.target.value)} />
            <button onClick={ask} disabled={asking || !prompt.trim()}>
              {asking ? "Думаю…" : "Спросить"}
            </button>
          </div>
          {answer && (
            <>
              <p className="answer">{answer.text}</p>
              <p className="muted small">
                {answer.tokens} токенов, {Math.round(answer.speed)} ток/с, чтение вопроса {Math.round(answer.prompt_ms)} мс
              </p>
            </>
          )}
        </>
      )}

      {(state.state === "stopped" || state.state === "crashed") && (
        <p className="muted">Выберите файл модели .gguf — например, скачанный с HuggingFace.</p>
      )}
      {state.state === "crashed" && state.error && <pre className="error log">{state.error}</pre>}
      {error && <p className="error">{error}</p>}

      <div className="actions">
        <button className={state.state === "ready" ? "secondary" : ""} onClick={pick} disabled={state.state === "starting"}>
          {state.state === "ready" ? "Другая модель" : "Выбрать модель"}
        </button>
        {state.state === "ready" && (
          <button className="secondary" onClick={() => llmStop()}>
            Остановить
          </button>
        )}
      </div>
    </div>
  );
}
