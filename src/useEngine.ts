import { useEffect, useState } from "react";
import {
  engineInstall,
  engineRepair,
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
  /** Итог последней «Починить» — для сообщения пользователю. */
  const [repaired, setRepaired] = useState<string | null>(null);

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
        const r = f.result;
        if (r?.reinstalled !== undefined) {
          setRepaired(
            r.reinstalled
              ? `Нашёл и исправил: ${r.broken?.length ?? 0} ${plural(r.broken?.length ?? 0)}. Движок переустановлен.`
              : "Всё в порядке: файлы движка целы.",
          );
        }
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

  const installed = status?.installed.find((i) => i.version === status.version) ?? null;

  return { status, installed, progress, paused, error, repaired, install, repair, pause };
}

const plural = (n: number) => {
  const d = n % 10, h = n % 100;
  if (d === 1 && h !== 11) return "файл";
  if (d >= 2 && d <= 4 && (h < 12 || h > 14)) return "файла";
  return "файлов";
};
