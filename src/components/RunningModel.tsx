import { useEffect, useState } from "react";
import { llmStart, llmStop, llmStatus, modelsRemove, onLlmState, vcredistInstall, type LlmState } from "../api";
import { memoryPages } from "../words";
import ProblemCard from "./ProblemCard";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/**
 * Что досталось видеокарте. 999 — «сколько влезет», выбор оставлен движку;
 * больше, чем слоёв у модели, — тоже всё (ядро добавляет выходной слой).
 */
const whoComputes = (onGpu: number | null, total: number | null) => {
  if (onGpu === null) return null;
  if (onGpu === 0) return "Считает процессор — ответы будут медленными";
  if (onGpu >= 900 || (total !== null && onGpu >= total)) return "Считает видеокарта";
  return total !== null
    ? `Видеокарта считает ${onGpu} слоёв из ${total}, остальное — процессор`
    : `Видеокарта считает ${onGpu} слоёв, остальное — процессор`;
};

/** Что сейчас загружено в видеокарту: состояние и «Остановить». Разговор — на вкладке «Чат». */
export default function RunningModel({
  onGoToChat,
  onGo,
  onRemoved,
}: {
  onGoToChat: () => void;
  /** Перейти в раздел: каталог — за версией поменьше, «Компьютер» — за движком. */
  onGo: (tab: "catalog" | "computer") => void;
  /** Модель убрали из списка кнопкой под ошибкой — список надо перечитать. */
  onRemoved: () => void;
}) {
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
            {whoComputes(state.gpu_layers, state.layers)}. {state.ctx !== null && <> Помнит {memoryPages(state.ctx)} разговора.</>}
          </p>
        </>
      )}

      {state.state === "crashed" && state.problem && (
        <ProblemCard
          problem={state.problem}
          on={crashActions(state, onGo, onRemoved)}
          extra={
            <button className="secondary" onClick={() => llmStop()}>
              Понятно
            </button>
          }
        />
      )}

      {state.state !== "crashed" && (
        <div className="actions">
          {state.state === "ready" && <button onClick={onGoToChat}>Перейти в чат</button>}
          <button className="secondary" onClick={() => llmStop()}>
            {state.state === "starting" ? "Отменить" : "Остановить"}
          </button>
        </div>
      )}
    </div>
  );
}

/** Кнопки под ошибкой запуска. Модель и ступень «экономнее» ядро прислало вместе с ошибкой. */
export function crashActions(
  state: LlmState,
  onGo: (tab: "catalog" | "computer") => void,
  onRemoved?: () => void,
) {
  const model = state.model;
  if (!model) return {};
  return {
    retry: () => llmStart(model, { lighter: state.lighter }),
    restart: () => llmStart(model, { lighter: state.lighter }),
    lighter: () => llmStart(model, { lighter: state.lighter + 1 }),
    catalog: () => onGo("catalog"),
    engine: () => onGo("computer"),
    // Компоненты поставились — сразу пробуем снова, второй кнопки не надо.
    vcredist: async () => {
      await vcredistInstall();
      await llmStart(model, { lighter: state.lighter });
    },
    ...(onRemoved && {
      forget: async () => {
        await modelsRemove(model);
        await llmStop();
        onRemoved();
      },
    }),
  };
}
