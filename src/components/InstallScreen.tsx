import { useEffect, useState } from "react";
import { formatBytes, type EngineProgress } from "../api";
import { openReport } from "./Report";
import { decimal, pl, t } from "../i18n";

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

const hints = () => [
  t(
    "После установки выберите модель — и с ней можно переписываться, как в мессенджере.",
    "After installing, pick a model — and you can chat with it like in a messenger.",
  ),
  t(
    "Модели работают прямо на вашем компьютере: переписка никуда не отправляется.",
    "Models run right on your computer: the conversation isn't sent anywhere.",
  ),
  t(
    "Установку можно поставить на паузу и даже закрыть программу — продолжим с того же места.",
    "You can pause the install and even close the program — we'll continue from the same point.",
  ),
  t(
    "Позже здесь же можно будет рисовать картинки и расшифровывать аудио.",
    "Later you'll be able to draw pictures and transcribe audio here too.",
  ),
];
const HINT_COUNT = 4;

/** «примерно 6 минут», «меньше минуты». */
export function formatEta(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return "";
  if (seconds < 60) return t("меньше минуты", "less than a minute");
  const min = Math.round(seconds / 60);
  if (min < 60) return `${t("примерно", "about")} ${min} ${pl(min, ["минута", "минуты", "минут"], ["minute", "minutes"])}`;
  const h = Math.round(min / 6) / 10;
  return t(`примерно ${decimal(h)} ч`, `about ${decimal(h)} h`);
}

const stageText = (stage: EngineProgress["stage"]) =>
  ({
    download: t("Скачиваю", "Downloading"),
    verify: t("Проверяю, что файлы пришли целыми", "Checking the files arrived intact"),
    unpack: t("Распаковываю", "Unpacking"),
    python: t("Готовлю движок картинок", "Preparing the image engine"),
    packages: t("Ставлю части движка картинок — это несколько минут", "Installing parts of the image engine — this takes a few minutes"),
    warmup: t("Проверяю, что видеокарта подхватилась", "Checking the graphics card is picked up"),
  })[stage];

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
    const timer = setInterval(() => setHint((h) => (h + 1) % HINT_COUNT), 7000);
    return () => clearInterval(timer);
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
        <p className="ok">{t("Всё установлено.", "Everything is installed.")}</p>
      </div>
    );
  }

  // До старта — честно предупреждаем, сколько качать и сколько ждать.
  if (!started && !paused && !error) {
    const need = left.reduce((s, i) => s + i.size, 0);
    return (
      <div className="card install">
        <p>
          {t("Скачаем", "We'll download")} <b>{formatBytes(need)}</b> — {t("обычно это", "usually that's")}{" "}
          {formatEta(need / TYPICAL_SPEED)}.
        </p>
        <Steps items={items} />
        <div className="actions">
          <button onClick={onStart}>{t("Начать", "Start")}</button>
          {onLater && (
            <button className="secondary" onClick={onLater}>
              {t("Позже", "Later")}
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
          {connecting ? t("Подключаюсь к серверу… ", "Connecting to the server… ") : `${formatBytes(done)} ${t("из", "of")} ${formatBytes(total)}`}
          {speed > 0 && `, ${formatBytes(speed)}/${t("с", "s")}, ${t("осталось", "left")} ${formatEta((total - done) / speed)}`}
          {paused && t(" — на паузе", " — paused")}
        </p>
      </div>

      {error && <ErrorBox error={error} />}

      {running && <p className="muted small hint">{hints()[hint]}</p>}

      <div className="actions">
        {running && active?.progress?.stage === "download" && (
          <button className="secondary" onClick={onPause}>
            {t("Пауза", "Pause")}
          </button>
        )}
        {!running && (paused || error) && (
          <button onClick={onStart}>{paused ? t("Продолжить", "Resume") : t("Повторить", "Retry")}</button>
        )}
        <button className="link" onClick={() => setDetails(!details)}>
          {details ? t("Скрыть подробности", "Hide details") : t("Подробнее", "Details")}
        </button>
      </div>

      {details && (
        <pre className="log">
          {items
            .map((i) => `${i.technical} — ${formatBytes(i.size)}, ${i.state}`)
            .concat(error ? [`${t("ошибка", "error")} (${error.kind ?? "?"}): ${error.text}`] : [])
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
              {i.state === "active" && i.progress ? `${stageText(i.progress.stage)}…` : i.why}
            </div>
          </div>
        </li>
      ))}
    </ul>
  );
}

function ErrorBox({ error }: { error: InstallError }) {
  const text: Record<string, string> = {
    net: t("Пропал интернет или сайт не отвечает.", "The internet dropped or the site isn't responding."),
    disk: t(
      "Не получилось записать файлы на диск. Проверьте, что на нём есть место.",
      "Couldn't write files to the disk. Check that there is free space on it.",
    ),
    broken: t("Файл пришёл испорченным. Скачаем его заново.", "The file arrived damaged. We'll download it again."),
  };
  return (
    <>
      <p className="error">
        {text[error.kind ?? ""] ?? t("Не получилось установить.", "Couldn't install.")}
        {error.kind === "net" &&
          error.retryIn != null &&
          t(` Продолжим сами через ${error.retryIn} с.`, ` We'll continue by ourselves in ${error.retryIn} s.`)}
        {error.kind !== "net" && error.kind !== "disk" && error.kind !== "broken" && (
          <span className="small"> {error.text}</span>
        )}
      </p>
      <button className="link small" onClick={() => openReport({ kind: "install", error: error.text })}>
        {t("Не получается — сообщить", "Still failing — report it")}
      </button>
    </>
  );
}
