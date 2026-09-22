import { useEffect, useState } from "react";
import { formatBytes, type EngineProgress } from "../api";

/** Шаг установки: человеческое название, зачем он нужен и что с ним сейчас. */
export interface InstallItem {
  id: string;
  title: string;
  /** Одна фраза «зачем», без технических слов. */
  why: string;
  /** Техническое имя — только в «Подробнее». */
  technical: string;
  size: number;
  state: "waiting" | "active" | "done" | "error";
  progress?: EngineProgress | null;
}

export interface InstallError {
  text: string;
  /** `net` | `disk` | `broken` | `other` — из ядра. */
  kind: string | null;
  /** Через сколько секунд повторим сами (только для `net`). */
  retryIn?: number | null;
}

/** Обычная скорость загрузки из фазы 0 (HF, 8 потоков) — для оценки до старта. */
const TYPICAL_SPEED = 2 * 1024 * 1024;

const HINTS = [
  "После установки выберите модель — и с ней можно переписываться, как в мессенджере.",
  "Модели работают прямо на вашем компьютере: переписка никуда не отправляется.",
  "Установку можно поставить на паузу и даже закрыть программу — продолжим с того же места.",
  "Позже здесь же можно будет рисовать картинки и расшифровывать аудио.",
];

/** «примерно 6 минут», «меньше минуты». */
export function formatEta(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return "";
  if (seconds < 60) return "меньше минуты";
  const min = Math.round(seconds / 60);
  if (min < 60) return `примерно ${min} ${plural(min, "минута", "минуты", "минут")}`;
  const h = Math.round(min / 6) / 10;
  return `примерно ${String(h).replace(".", ",")} ч`;
}

function plural(n: number, one: string, few: string, many: string) {
  const d = n % 10, h = n % 100;
  if (d === 1 && h !== 11) return one;
  if (d >= 2 && d <= 4 && (h < 12 || h > 14)) return few;
  return many;
}

const STAGE_TEXT: Record<EngineProgress["stage"], string> = {
  download: "Скачиваю",
  verify: "Проверяю, что файлы пришли целыми",
  unpack: "Распаковываю",
};

/**
 * Экран установки для новичка (UX — заметка «Установка»): до старта — сколько и сколько времени,
 * потом шаги ✓ / идёт / ждёт, общий прогресс, оставшееся время словами, пауза,
 * ошибки простыми словами с кнопкой действия, «Подробнее» с техническими именами.
 */
export default function InstallScreen({
  items,
  started,
  paused,
  error,
  onStart,
  onPause,
  onLater,
}: {
  items: InstallItem[];
  started: boolean;
  paused: boolean;
  error: InstallError | null;
  onStart: () => void;
  onPause: () => void;
  onLater?: () => void;
}) {
  const [details, setDetails] = useState(false);
  const [hint, setHint] = useState(0);

  const running = items.some((i) => i.state === "active");
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setHint((h) => (h + 1) % HINTS.length), 7000);
    return () => clearInterval(t);
  }, [running]);

  const left = items.filter((i) => i.state !== "done");
  const total = items.reduce((s, i) => s + i.size, 0);
  const done = items.reduce(
    (s, i) => s + (i.state === "done" ? i.size : Math.min(i.progress?.done ?? 0, i.size)),
    0,
  );
  const active = items.find((i) => i.state === "active");
  const speed = active?.progress?.stage === "download" ? active.progress.speed : 0;
  // Первые секунды загрузки — узнаём размер и адрес у сервера, байтов ещё нет.
  const connecting = active?.progress?.stage === "download" && !active.progress.done && !speed;

  if (left.length === 0) {
    return (
      <div className="card install">
        <Steps items={items} />
        <p className="ok">Всё установлено.</p>
      </div>
    );
  }

  // До старта — честно предупреждаем, сколько качать и сколько ждать.
  if (!started && !paused && !error) {
    const need = left.reduce((s, i) => s + i.size, 0);
    return (
      <div className="card install">
        <p>
          Скачаем <b>{formatBytes(need)}</b> — обычно это {formatEta(need / TYPICAL_SPEED)}.
        </p>
        <Steps items={items} />
        <div className="actions">
          <button onClick={onStart}>Начать</button>
          {onLater && (
            <button className="secondary" onClick={onLater}>
              Позже
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="card install">
      <Steps items={items} />

      <div className="install-total">
        {connecting || active?.progress?.stage === "unpack" || (active && !active.progress?.total) ? (
          <progress />
        ) : (
          <progress max={total || 1} value={done} />
        )}
        <p className="small">
          {connecting ? "Подключаюсь к серверу… " : `${formatBytes(done)} из ${formatBytes(total)}`}
          {speed > 0 && `, ${formatBytes(speed)}/с, осталось ${formatEta((total - done) / speed)}`}
          {paused && " — на паузе"}
        </p>
      </div>

      {error && <ErrorBox error={error} />}

      {running && <p className="muted small hint">{HINTS[hint]}</p>}

      <div className="actions">
        {running && active?.progress?.stage === "download" && (
          <button className="secondary" onClick={onPause}>
            Пауза
          </button>
        )}
        {!running && (paused || error) && <button onClick={onStart}>{paused ? "Продолжить" : "Повторить"}</button>}
        <button className="link" onClick={() => setDetails(!details)}>
          {details ? "Скрыть подробности" : "Подробнее"}
        </button>
      </div>

      {details && (
        <pre className="log">
          {items
            .map((i) => `${i.technical} — ${formatBytes(i.size)}, ${i.state}`)
            .concat(error ? [`ошибка (${error.kind ?? "?"}): ${error.text}`] : [])
            .join("\n")}
        </pre>
      )}
    </div>
  );
}

function Steps({ items }: { items: InstallItem[] }) {
  return (
    <ul className="install-steps">
      {items.map((i) => (
        <li key={i.id} className={i.state}>
          <span className="icon">{i.state === "done" ? "✓" : i.state === "error" ? "!" : i.state === "active" ? "" : "·"}</span>
          <div>
            <b>{i.title}</b> <span className="muted small">{formatBytes(i.size)}</span>
            <div className="muted small">
              {i.state === "active" && i.progress ? `${STAGE_TEXT[i.progress.stage]}…` : i.why}
            </div>
          </div>
        </li>
      ))}
    </ul>
  );
}

function ErrorBox({ error }: { error: InstallError }) {
  const text: Record<string, string> = {
    net: "Пропал интернет или сайт не отвечает.",
    disk: "Не получилось записать файлы на диск. Проверьте, что на нём есть место.",
    broken: "Файл пришёл испорченным. Скачаем его заново.",
  };
  return (
    <p className="error">
      {text[error.kind ?? ""] ?? "Не получилось установить."}
      {error.kind === "net" && error.retryIn != null && ` Продолжим сами через ${error.retryIn} с.`}
      {error.kind !== "net" && error.kind !== "disk" && error.kind !== "broken" && (
        <span className="small"> {error.text}</span>
      )}
    </p>
  );
}
