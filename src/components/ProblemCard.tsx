import { useState } from "react";
import type { Problem, ProblemAction, ReportKind } from "../api";
import { t } from "../i18n";
import { openReport } from "./Report";

const label = (a: ProblemAction) =>
  ({
    retry: t("Попробовать ещё раз", "Try again"),
    lighter: t("Запустить экономнее", "Run lighter"),
    catalog: t("Выбрать в каталоге", "Pick in the catalog"),
    forget: t("Убрать из списка", "Remove from the list"),
    restart: t("Запустить модель заново", "Restart the model"),
    new_chat: t("Новый разговор", "New conversation"),
    engine: t("Установить движок", "Install the engine"),
    vcredist: t("Поставить компоненты", "Install components"),
    models: t("К моделям", "To models"),
  })[a];

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
  kind = "model",
}: {
  problem: Problem;
  on: Partial<Record<ProblemAction, () => unknown>>;
  /** Кнопка, которая нужна всегда: например, «Понятно». */
  extra?: React.ReactNode;
  /** Какую форму отчёта открыть по «Не помогло — сообщить». */
  kind?: ReportKind;
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
              {label(a)}
            </button>
          ))}
          {extra}
        </div>
      )}
      {problem.details && (
        <details>
          <summary className="muted small">{t("Подробности", "Details")}</summary>
          <pre className="log">{problem.details}</pre>
        </details>
      )}
      <button className="link small" onClick={() => openReport({ kind, error: problem.text })}>
        {t("Не помогло — сообщить", "Didn't help — report it")}
      </button>
    </div>
  );
}
