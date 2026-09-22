import { useEffect, useState } from "react";
import { llmStop, llmStatus, onLlmState, type LlmState } from "../api";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** Что досталось видеокарте: 999 — «сколько влезет», выбор оставлен движку. */
const whoComputes = (layers: number | null) => {
  if (layers === null) return null;
  if (layers === 0) return "Считает процессор — ответы будут медленными";
  if (layers >= 900) return "Считает видеокарта";
  return `Видеокарта считает ${layers} слоёв, остальное — процессор`;
};

/** Что сейчас загружено в видеокарту: состояние и «Остановить». Разговор — на вкладке «Чат». */
export default function RunningModel({ onGoToChat }: { onGoToChat: () => void }) {
  const [state, setState] = useState<LlmState | null>(null);

  useEffect(() => {
    llmStatus().then(setState);
    const sub = onLlmState(setState);
    return () => {
      sub.then((un) => un());
    };
  }, []);

  if (!state || state.state === "stopped") return null;

  return (
    <div className="card form">
      {state.state === "starting" && <p>Загружаю {fileName(state.model ?? "")} в видеокарту…</p>}

      {state.state === "ready" && (
        <>
          <p className="ok">
            ✓ Готова к разговору: {fileName(state.model ?? "")}, загрузилась за{" "}
            {state.started_in?.toFixed(1).replace(".", ",")} с
          </p>
          <p className="muted small">
            {whoComputes(state.gpu_layers)}. Память разговора — до {state.ctx} токенов.
          </p>
        </>
      )}

      {state.state === "crashed" && (
        <>
          <p className="error">Модель не запустилась.</p>
          {state.error && <pre className="error log">{state.error}</pre>}
        </>
      )}

      <div className="actions">
        {state.state === "ready" && <button onClick={onGoToChat}>Перейти в чат</button>}
        <button className="secondary" onClick={() => llmStop()}>
          {state.state === "starting" ? "Отменить" : state.state === "crashed" ? "Понятно" : "Остановить"}
        </button>
      </div>
    </div>
  );
}
