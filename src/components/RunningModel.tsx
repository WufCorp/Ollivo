import { useEffect, useState } from "react";
import { llmStop, llmStatus, onLlmState, type LlmState } from "../api";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

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
        <p className="ok">
          ✓ Готова к разговору: {fileName(state.model ?? "")}, загрузилась за{" "}
          {state.started_in?.toFixed(1).replace(".", ",")} с
        </p>
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
