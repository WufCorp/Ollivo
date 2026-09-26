import { useEffect, useRef, useState } from "react";
import {
  engineInstall,
  formatBytes,
  onDownloadFinished,
  onDownloadProgress,
  onEngineFinished,
  onEngineProgress,
  speechModelDownload,
  speechStatus,
  type SpeechStatus,
} from "../api";

/** Распознавание речи не стоит: предлагаем докачать движок и модель одной кнопкой. */
export default function SpeechSetup({
  why,
  onReady,
  onCancel,
}: {
  /** Зачем понадобилось: «надиктовать вопрос», «расшифровать запись». */
  why: string;
  onReady: () => void;
  onCancel: () => void;
}) {
  const [status, setStatus] = useState<SpeechStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // Сколько уже скачано до текущей части: движок качается первым, модель — следом.
  const base = useRef(0);
  // Обработчики событий заведены один раз — свежие значения берём через ref.
  const doneRef = useRef(0);
  doneRef.current = done;
  const readyRef = useRef(onReady);
  readyRef.current = onReady;

  useEffect(() => {
    speechStatus().then(setStatus);
    const subs = [
      onEngineProgress((p) => {
        if (p.id === "whisper.cpp" && p.stage === "download") setDone(p.done);
      }),
      onEngineFinished((f) => {
        if (f.id !== "whisper.cpp") return;
        if (f.error) fail(f.error === "paused" ? "Загрузка прервалась." : f.error);
        else next();
      }),
      onDownloadProgress((p) => {
        if (p.id === "speech:model") setDone(base.current + p.done);
      }),
      onDownloadFinished((f) => {
        if (f.id !== "speech:model") return;
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
    const s = await speechStatus();
    try {
      if (!s.engine) {
        await engineInstall("whisper.cpp");
      } else if (!s.model) {
        base.current = doneRef.current;
        await speechModelDownload();
      } else {
        setBusy(false);
        readyRef.current();
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
        Чтобы {why}, нужно один раз докачать распознавание речи — {formatBytes(status.download)}. Оно работает
        прямо на компьютере: запись никуда не отправляется.
      </p>
      {busy && (
        <>
          <progress value={done} max={status.download} />
          <p className="muted small">
            {formatBytes(done)} из {formatBytes(status.download)}
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
