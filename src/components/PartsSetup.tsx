import { useEffect, useRef, useState } from "react";
import {
  engineInstall,
  formatBytes,
  onDownloadFinished,
  onDownloadProgress,
  onEngineFinished,
  onEngineProgress,
  partsStatus,
  speechModelDownload,
  PART_SPEECH_MODEL,
  type PartsStatus,
} from "../api";

/** Как назвать докачку человеку: распознавание — это движок и модель вместе. */
const whatToGet = (parts: string[]) => {
  const names = [];
  if (parts.some((p) => p !== "ffmpeg")) names.push("распознавание речи");
  if (parts.includes("ffmpeg")) names.push("чтение таких файлов");
  return names.join(" и ");
};

/**
 * Не хватает частей, которые ставятся по требованию (распознавание речи, ffmpeg):
 * предлагаем докачать их одной кнопкой и ставим по очереди.
 */
export default function PartsSetup({
  why,
  parts,
  onReady,
  onCancel,
}: {
  /** Зачем понадобилось: «надиктовать вопрос», «расшифровать запись». */
  why: string;
  /** Что нужно: `ffmpeg`, `whisper.cpp`, `speech:model`; уже стоящее пропускается. */
  parts: string[];
  onReady: () => void;
  onCancel: () => void;
}) {
  const [status, setStatus] = useState<PartsStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(0);
  const [unpacking, setUnpacking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Сколько уже скачано до текущей части: части качаются одна за другой.
  const base = useRef(0);
  // Обработчики событий заведены один раз — свежие значения берём через ref.
  const doneRef = useRef(0);
  doneRef.current = done;
  const readyRef = useRef(onReady);
  readyRef.current = onReady;
  const partsRef = useRef(parts);
  partsRef.current = parts;

  useEffect(() => {
    partsStatus(parts).then(setStatus);
    const mine = (id: string) => partsRef.current.includes(id);
    const subs = [
      onEngineProgress((p) => {
        if (!mine(p.id)) return;
        setUnpacking(p.stage === "unpack");
        if (p.stage === "download") setDone(base.current + p.done);
      }),
      onEngineFinished((f) => {
        if (!mine(f.id)) return;
        setUnpacking(false);
        if (f.error) fail(f.error === "paused" ? "Загрузка прервалась." : f.error);
        else next();
      }),
      onDownloadProgress((p) => {
        if (p.id === PART_SPEECH_MODEL) setDone(base.current + p.done);
      }),
      onDownloadFinished((f) => {
        if (f.id !== PART_SPEECH_MODEL) return;
        if (f.error) fail(f.error === "paused" ? "Загрузка прервалась." : f.error);
        else next();
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  const fail = (e: string) => {
    setBusy(false);
    setError(e);
  };

  /** Следующая недостающая часть; всё на месте — готово. */
  const next = async () => {
    // Общий размер на экране не пересчитываем: полоса не должна прыгать между частями.
    const s = await partsStatus(partsRef.current);
    base.current = doneRef.current;
    const part = s.missing[0];
    try {
      if (!part) {
        setBusy(false);
        readyRef.current();
      } else if (part === PART_SPEECH_MODEL) {
        await speechModelDownload();
      } else {
        await engineInstall(part);
      }
    } catch (e) {
      fail(String(e));
    }
  };

  const start = () => {
    setBusy(true);
    setError(null);
    setDone(0);
    base.current = 0;
    next();
  };

  if (!status) return null;
  return (
    <div className="card notice">
      <p>
        Чтобы {why}, нужно один раз докачать {whatToGet(status.missing)} — {formatBytes(status.download)}. Всё
        работает прямо на компьютере: файлы никуда не отправляются.
      </p>
      {busy && (
        <>
          <progress value={done} max={status.download} />
          <p className="muted small">
            {unpacking ? "Распаковываю…" : `${formatBytes(done)} из ${formatBytes(status.download)}`}
          </p>
        </>
      )}
      {error && <p className="error small">{error}</p>}
      {!busy && (
        <div className="actions">
          <button onClick={start}>{error ? "Ещё раз" : "Докачать"}</button>
          <button className="secondary" onClick={onCancel}>
            Не надо
          </button>
        </div>
      )}
    </div>
  );
}
