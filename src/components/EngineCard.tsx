import { useEffect } from "react";
import { buildName, formatBytes } from "../api";
import { t } from "../i18n";
import { stageName, useEngine } from "../useEngine";

/** Карточка движка: статус, кнопка «Установить», прогресс с паузой. */
export default function EngineCard({ id, onInstalled }: { id: string; onInstalled?: () => void }) {
  const { status, installed, progress, paused, error, repaired, install, repair, pause } = useEngine(id);

  useEffect(() => {
    if (installed) onInstalled?.();
  }, [installed?.dir]);

  if (!status) return <p className="muted">{t("Смотрю, что уже установлено…", "Checking what is already installed…")}</p>;

  return (
    <div className="card">
      {progress ? (
        <>
          <p>
            {stageName(progress.stage)}…
            {progress.total > 0 && ` ${formatBytes(progress.done)} ${t("из", "of")} ${formatBytes(progress.total)}`}
            {progress.stage === "download" && progress.speed > 0 && `, ${formatBytes(progress.speed)}/${t("с", "s")}`}
          </p>
          {progress.total > 0 ? <progress max={progress.total} value={progress.done} /> : <progress />}
          {progress.stage === "download" && (
            <button className="secondary" onClick={pause}>
              {t("Пауза", "Pause")}
            </button>
          )}
        </>
      ) : installed ? (
        <>
          <p className="ok">
            ✓ {t("Установлен", "Installed")}: llama.cpp {installed.version}, {buildName(installed.build)}
          </p>
          {repaired && <p className="muted">{repaired}</p>}
          <div className="actions">
            <button
              className="secondary"
              onClick={repair}
              title={t("Проверить файлы движка и перекачать испорченные", "Check the engine files and re-download damaged ones")}
            >
              {t("Починить", "Repair")}
            </button>
            {/* Запасной путь, когда обычная сборка падает на этой карте или не видит её. */}
            {status.cuda && installed.build === "vulkan" && (
              <button
                className="secondary"
                onClick={() => install(status.cuda!)}
                title={t(
                  "Другая сборка движка — для видеокарт NVIDIA. Нужна, если модель не запускается или считает на процессоре.",
                  "Another engine build — for NVIDIA graphics cards. Needed if the model won't start or runs on the processor.",
                )}
              >
                {t(`Поставить сборку ${buildName(status.cuda)}`, `Install the ${buildName(status.cuda)} build`)} ({formatBytes(status.cuda_size)})
              </button>
            )}
          </div>
        </>
      ) : (
        <>
          <p className="muted">
            {status.title}: llama.cpp {status.version}
            {status.build && `, ${t("сборка", "build")} ${buildName(status.build)}, ${formatBytes(status.size)}`}
          </p>
          {status.installed.some((i) => i.version === status.version) && (
            <p className="muted">
              {t(
                "Установленная сборка не видит эту видеокарту — нужна другая.",
                "The installed build can't see this graphics card — another one is needed.",
              )}
            </p>
          )}
          <button onClick={() => install()} disabled={!status.build}>
            {paused ? t("Продолжить", "Resume") : t("Установить", "Install")}
          </button>
        </>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}
