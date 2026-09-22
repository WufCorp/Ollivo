import { useEffect } from "react";
import { BUILD_NAMES, formatBytes } from "../api";
import { STAGE_NAMES, useEngine } from "../useEngine";

/** Карточка движка: статус, кнопка «Установить», прогресс с паузой. */
export default function EngineCard({ id, onInstalled }: { id: string; onInstalled?: () => void }) {
  const { status, installed, progress, paused, error, repaired, install, repair, pause } = useEngine(id);

  useEffect(() => {
    if (installed) onInstalled?.();
  }, [installed?.dir]);

  if (!status) return <p className="muted">Смотрю, что уже установлено…</p>;

  return (
    <div className="card">
      {progress ? (
        <>
          <p>
            {STAGE_NAMES[progress.stage]}…
            {progress.total > 0 && ` ${formatBytes(progress.done)} из ${formatBytes(progress.total)}`}
            {progress.stage === "download" && progress.speed > 0 && `, ${formatBytes(progress.speed)}/с`}
          </p>
          {progress.total > 0 ? <progress max={progress.total} value={progress.done} /> : <progress />}
          {progress.stage === "download" && (
            <button className="secondary" onClick={pause}>
              Пауза
            </button>
          )}
        </>
      ) : installed ? (
        <>
          <p className="ok">
            ✓ Установлен: llama.cpp {installed.version}, {BUILD_NAMES[installed.build]}
          </p>
          {repaired && <p className="muted">{repaired}</p>}
          <button className="secondary" onClick={repair} title="Проверить файлы движка и перекачать испорченные">
            Починить
          </button>
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
