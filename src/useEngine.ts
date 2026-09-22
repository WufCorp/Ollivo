import { useEffect, useState } from "react";
import {
  engineInstall,
  engineStatus,
  onEngineFinished,
  onEngineProgress,
  taskPause,
  type EngineProgress,
  type EngineStatus,
} from "./api";

export const STAGE_NAMES: Record<EngineProgress["stage"], string> = {
  download: "Скачиваю",
  verify: "Проверяю файлы",
  unpack: "Распаковываю",
};

/** Состояние и установка движка: статус, прогресс, пауза, ошибка. */
export function useEngine(id: string) {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [progress, setProgress] = useState<EngineProgress | null>(null);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => engineStatus(id).then(setStatus, (e) => setError(String(e)));

  useEffect(() => {
    refresh();
    const subs = [
      onEngineProgress((p) => p.id === id && setProgress(p)),
      onEngineFinished((f) => {
        if (f.id !== id) return;
        setProgress(null);
        setPaused(f.error === "paused");
        setError(f.error && f.error !== "paused" ? f.error : null);
        refresh();
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, [id]);

  const install = () => {
    setError(null);
    setPaused(false);
    setProgress({ id, stage: "download", done: 0, total: status?.size ?? 0, speed: 0 });
    engineInstall(id).catch((e) => {
      setProgress(null);
      setError(String(e));
    });
  };

  const pause = () => taskPause(`engine:${id}`);

  const installed = status?.installed.find((i) => i.version === status.version) ?? null;

  return { status, installed, progress, paused, error, install, pause };
}
