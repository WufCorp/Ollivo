import { useEffect } from "react";
import { BUILD_NAMES, formatBytes } from "../api";
import { STAGE_NAMES, useEngine } from "../useEngine";

/** Карточка движка: статус, кнопка «Установить», прогресс с паузой. */
export default function EngineCard({ id, onInstalled }: { id: string; onInstalled?: () => void }) {
  const { status, installed, progress, paused, error, install, pause } = useEngine(id);

  useEffect(() => {
    if (installed) onInstalled?.();
  }, [installed?.dir]);

  if (!status) return <p className="muted">Смотрю, что уже установлено…</p>;

  return (
    <div className="card">
      {installed ? (
        <p className="ok">
          ✓ Установлен: llama.cpp {installed.version}, {BUILD_NAMES[installed.build]}
        </p>
      ) : progress ? (
        <>
          <p>
            {STAGE_NAMES[progress.stage]}… {formatBytes(progress.done)} из {formatBytes(progress.total)}
            {progress.stage === "download" && progress.speed > 0 && `, ${formatBytes(progress.speed)}/с`}
          </p>
          <progress max={progress.total || 1} value={progress.done} />
          {progress.stage === "download" && (
            <button className="secondary" onClick={pause}>
              Пауза
            </button>
          )}
        </>
      ) : (
        <>
          <p className="muted">
            {status.title}: llama.cpp {status.version}
            {status.build && `, сборка ${BUILD_NAMES[status.build]}, ${formatBytes(status.size)}`}
          </p>
          <button onClick={install} disabled={!status.build}>
            {paused ? "Продолжить" : "Установить"}
          </button>
        </>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}
