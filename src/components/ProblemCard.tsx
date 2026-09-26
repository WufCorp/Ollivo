import { useState } from "react";
import type { Problem, ProblemAction } from "../api";

const LABELS: Record<ProblemAction, string> = {
  retry: "Попробовать ещё раз",
  lighter: "Запустить экономнее",
  catalog: "Выбрать в каталоге",
  forget: "Убрать из списка",
  restart: "Запустить модель заново",
  new_chat: "Новый разговор",
  engine: "Установить движок",
  vcredist: "Поставить компоненты",
  models: "К моделям",
};

/**
 * Ошибка человеческими словами: что случилось, что делать и кнопки.
 * Кнопки — только те, что ядро сочло уместными и для которых у этого места
 * есть обработчик: в чате, например, «Убрать из списка» не к месту.
 * Сырой текст — под «Подробности»: его попросит тот, кто будет помогать.
 */
export default function ProblemCard({
  problem,
  on,
  extra,
}: {
  problem: Problem;
  on: Partial<Record<ProblemAction, () => unknown>>;
  /** Кнопка, которая нужна всегда: например, «Понятно». */
  extra?: React.ReactNode;
}) {
  const [busy, setBusy] = useState(false);
  const actions = problem.actions.filter((a) => on[a]);

  const run = async (a: ProblemAction) => {
    setBusy(true);
    try {
      await on[a]?.();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="problem">
      <p className="error">{problem.text}</p>
      {problem.hint && <p className="muted small">{problem.hint}</p>}
      {(actions.length > 0 || extra) && (
        <div className="actions">
          {actions.map((a, i) => (
            <button key={a} className={i === 0 ? "" : "secondary"} disabled={busy} onClick={() => run(a)}>
              {LABELS[a]}
            </button>
          ))}
          {extra}
        </div>
      )}
      {problem.details && (
        <details>
          <summary className="muted small">Подробности</summary>
          <pre className="log">{problem.details}</pre>
        </details>
      )}
    </div>
  );
}
