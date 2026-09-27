import { useEffect, useState } from "react";
import { llmStart, llmStop, llmStatus, modelsRemove, onLlmState, vcredistInstall, type LlmState } from "../api";
import { memoryPages } from "../words";
import ProblemCard from "./ProblemCard";
import { decimal, t } from "../i18n";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/**
 * Что досталось видеокарте. 999 — «сколько влезет», выбор оставлен движку;
 * больше, чем слоёв у модели, — тоже всё (ядро добавляет выходной слой).
 */
export const whoComputes = (onGpu: number | null, total: number | null) => {
  if (onGpu === null) return null;
  if (onGpu === 0) return t("Считает процессор — ответы будут медленными", "The processor computes — answers will be slow");
  if (onGpu >= 900 || (total !== null && onGpu >= total)) return t("Считает видеокарта", "The graphics card computes");
  return total !== null
    ? t(`Видеокарта считает ${onGpu} слоёв из ${total}, остальное — процессор`, `The graphics card computes ${onGpu} of ${total} layers, the rest — the processor`)
    : t(`Видеокарта считает ${onGpu} слоёв, остальное — процессор`, `The graphics card computes ${onGpu} layers, the rest — the processor`);
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
      {state.state === "starting" && (
        <p>{t(`Загружаю ${fileName(state.model ?? "")} в видеокарту…`, `Loading ${fileName(state.model ?? "")} into the graphics card…`)}</p>
      )}

      {state.state === "ready" && (
        <>
          <p className="ok">
            ✓{" "}
            {t(
              `Готова к разговору: ${fileName(state.model ?? "")}, загрузилась за ${decimal(state.started_in ?? 0)} с`,
              `Ready to talk: ${fileName(state.model ?? "")}, loaded in ${decimal(state.started_in ?? 0)} s`,
            )}
          </p>
          <p className="muted small">
            {whoComputes(state.gpu_layers, state.layers)}.{" "}
            {state.ctx !== null && t(`Помнит ${memoryPages(state.ctx)} разговора.`, `Remembers ${memoryPages(state.ctx)} of conversation.`)}
          </p>
        </>
      )}

      {state.state === "sleeping" && (
        <>
          <p>
            {t(
              `${fileName(state.model ?? "")} выгружена, пока вы не пользовались, — видеокарта свободна.`,
              `${fileName(state.model ?? "")} was unloaded while you weren't using it — the graphics card is free.`,
            )}
          </p>
          <p className="muted small">
            {t("Загрузится снова сама, когда вы зададите вопрос в чате.", "It will load again by itself when you ask something in the chat.")}
          </p>
        </>
      )}

      {state.state === "crashed" && state.problem && (
        <ProblemCard
          problem={state.problem}
          on={crashActions(state, onGo, onRemoved)}
          extra={
            <button className="secondary" onClick={() => llmStop()}>
              {t("Понятно", "OK")}
            </button>
          }
        />
      )}

      {state.state !== "crashed" && (
        <div className="actions">
          {state.state === "ready" && <button onClick={onGoToChat}>{t("Перейти в чат", "Go to chat")}</button>}
          {state.state === "sleeping" && state.model && (
            <button onClick={() => llmStart(state.model!, { lighter: state.lighter })}>{t("Загрузить сейчас", "Load now")}</button>
          )}
          <button className="secondary" onClick={() => llmStop()}>
            {state.state === "starting"
              ? t("Отменить", "Cancel")
              : state.state === "sleeping"
                ? t("Забыть", "Forget")
                : t("Остановить", "Stop")}
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
