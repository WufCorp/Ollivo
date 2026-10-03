import { useEffect, useState } from "react";
import {
  engineInstall,
  engineRepair,
  engineStatus,
  onEngineFinished,
  onEngineProgress,
  taskPause,
  type Build,
  type EngineProgress,
  type EngineStatus,
} from "./api";
import { pl, t } from "./i18n";

export const stageName = (stage: EngineProgress["stage"]) =>
  ({
    download: t("Скачиваю", "Downloading"),
    verify: t("Проверяю файлы", "Checking files"),
    unpack: t("Распаковываю", "Unpacking"),
    python: t("Готовлю движок картинок", "Preparing the image engine"),
    packages: t("Ставлю части движка картинок", "Installing parts of the image engine"),
    warmup: t("Проверяю видеокарту", "Checking the graphics card"),
  })[stage];

/** Состояние и установка движка: статус, прогресс, пауза, ошибка. */
export function useEngine(id: string) {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [progress, setProgress] = useState<EngineProgress | null>(null);
  const [paused, setPaused] = useState(false);
  /** Последний прогресс: на паузе показываем, сколько уже скачано. */
  const [last, setLast] = useState<EngineProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [errorKind, setErrorKind] = useState<string | null>(null);
  /** Итог последней «Починить» — для сообщения пользователю. */
  const [repaired, setRepaired] = useState<string | null>(null);

  const refresh = () => engineStatus(id).then(setStatus, (e) => setError(String(e)));

  useEffect(() => {
    refresh();
    const subs = [
      onEngineProgress((p) => {
        if (p.id !== id) return;
        setProgress(p);
        setLast(p);
      }),
      onEngineFinished((f) => {
        if (f.id !== id) return;
        setProgress(null);
        setPaused(f.error === "paused");
        setError(f.error && f.error !== "paused" ? f.error : null);
        setErrorKind(f.kind);
        const r = f.result;
        if (r?.reinstalled !== undefined) {
          const n = r.broken?.length ?? 0;
          const files = `${n} ${pl(n, ["файл", "файла", "файлов"], ["file", "files"])}`;
          setRepaired(
            r.reinstalled
              ? t(`Нашёл и исправил: ${files}. Движок переустановлен.`, `Found and fixed: ${files}. The engine was reinstalled.`)
              : t("Всё в порядке: файлы движка целы.", "All good: the engine files are intact."),
          );
        }
        refresh();
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, [id]);

  /** `build` — не та сборка, что по умолчанию (запасная CUDA). */
  const install = (build?: Build) => {
    setError(null);
    setErrorKind(null);
    setPaused(false);
    const total = build && build === status?.cuda ? status.cuda_size : (status?.size ?? 0);
    setProgress({ id, stage: "download", done: 0, total, speed: 0 });
    engineInstall(id, build).catch((e) => {
      setProgress(null);
      setError(String(e));
    });
  };

  const repair = () => {
    setError(null);
    setRepaired(null);
    setPaused(false);
    setProgress({ id, stage: "verify", done: 0, total: 0, speed: 0 });
    engineRepair(id).catch((e) => {
      setProgress(null);
      setError(String(e));
    });
  };

  const pause = () => taskPause(`engine:${id}`);

  const installed = status?.active ?? null;

  return { status, installed, progress, last, paused, error, errorKind, repaired, install, repair, pause };
}
