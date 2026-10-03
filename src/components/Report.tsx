import { useEffect, useState } from "react";
import { reportMake, reportMessage, reportSend, type Report, type ReportChannel, type ReportKind } from "../api";
import { getLang, t } from "../i18n";

const kinds = (): { id: ReportKind; label: string }[] => [
  { id: "install", label: t("Не ставится", "Won't install") },
  { id: "model", label: t("Модель не запускается", "Model won't start") },
  { id: "other", label: t("Другое", "Other") },
];

const channels = (): { id: ReportChannel; label: string; hint: string }[] => [
  {
    id: "telegram",
    label: "Telegram",
    hint: t("Откроется группа поддержки Ollivo в Telegram.", "The Ollivo support group in Telegram will open."),
  },
  {
    id: "max",
    label: "MAX",
    hint: t("Откроется группа Ollivo в MAX.", "The Ollivo group in MAX will open."),
  },
  {
    id: "github",
    label: "GitHub",
    hint: t(
      "Откроется форма на GitHub — там уже всё заполнено. Нужен аккаунт, он бесплатный.",
      "A GitHub form will open with everything filled in. You need an account — it's free.",
    ),
  },
];

const CHANNEL_KEY = "ollivo.report.channel";

/** Куда отправляли в прошлый раз; впервые — мессенджер для русского, GitHub для английского. */
const savedChannel = (): ReportChannel => {
  try {
    const v = localStorage.getItem(CHANNEL_KEY);
    if (v === "telegram" || v === "max" || v === "github") return v;
  } catch {
    // Нет хранилища — просто не запомним.
  }
  return getLang() === "ru" ? "telegram" : "github";
};

/** В буфер обмена; `false` — не вышло (тогда человек скопирует текст из окна сам). */
async function copy(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

interface Opts {
  kind?: ReportKind;
  /** Ошибка, под которой нажали «Не помогло — сообщить»: попадёт в описание. */
  error?: string;
}

let show: ((o: Opts) => void) | null = null;

/** Открывает окно «Сообщить о проблеме» из любого места программы. */
export const openReport = (o: Opts = {}) => show?.(o);

/** Место для окна отчёта — одно на всю программу, в `App`. */
export function ReportHost() {
  const [opts, setOpts] = useState<Opts | null>(null);
  const [n, setN] = useState(0);
  useEffect(() => {
    show = (o) => {
      setOpts(o);
      setN((n) => n + 1);
    };
    return () => {
      show = null;
    };
  }, []);
  if (!opts) return null;
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && setOpts(null)}>
      <ReportDialog key={n} {...opts} onClose={() => setOpts(null)} />
    </div>
  );
}

/**
 * Отчёт собирает ядро; человек видит его целиком, пишет своими словами, что случилось,
 * выбирает, куда отправить, и отправляет сам: файл — в «Загрузки», дальше форма issue на GitHub
 * или чат в Telegram / MAX. Для мессенджеров текст уже в буфере обмена — остаётся вставить.
 */
function ReportDialog({ kind: startKind = "other", error, onClose }: Opts & { onClose: () => void }) {
  const [kind, setKind] = useState<ReportKind>(startKind);
  const [what, setWhat] = useState(error ? t(`На экране было: «${error}»\n\n`, `The screen said: “${error}”\n\n`) : "");
  const [report, setReport] = useState<Report | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [channel, setChannel] = useState<ReportChannel>(savedChannel);
  /** Текст для мессенджера; `copied` — удалось ли положить его в буфер обмена. */
  const [message, setMessage] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    reportMake().then(setReport, (e) => setFailed(String(e)));
  }, []);

  const send = async () => {
    if (!report) return;
    setBusy(true);
    setFailed(null);
    try {
      localStorage.setItem(CHANNEL_KEY, channel);
    } catch {
      // Не запомним выбор — не беда.
    }
    try {
      // В буфер обмена — до отправки: потом фокус уйдёт в Проводник и мессенджер.
      if (channel !== "github") {
        const text = await reportMessage(kind, what, report.summary);
        setMessage(text);
        setCopied(await copy(text));
      }
      setSaved(await reportSend(kind, what, report, channel));
    } catch (e) {
      setFailed(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (saved) {
    const file = <b>{saved.split(/[\\/]/).pop()}</b>;
    return (
      <div className="dialog card" role="dialog" aria-label={t("Сообщить о проблеме", "Report a problem")}>
        <h2>{t("Почти готово", "Almost done")}</h2>
        {channel === "github" ? (
          <>
            <p>
              {t(
                "В браузере открылась форма на GitHub — там уже всё заполнено. Перетащите в поле «Отчёт» файл",
                "A GitHub form has opened in your browser — everything is already filled in. Drag the file",
              )}{" "}
              {file}{" "}
              {t(
                "из папки «Загрузки» (она открыта в Проводнике) и отправьте форму.",
                "from the Downloads folder (it is open in Explorer) into the “Report” field and submit the form.",
              )}
            </p>
            <p className="muted small">{t("Нужен аккаунт на GitHub — он бесплатный.", "You need a GitHub account — it's free.")}</p>
          </>
        ) : (
          <>
            <p>
              {channel === "telegram"
                ? t("Открывается группа поддержки Ollivo в Telegram — вступите в неё, если ещё не вступили.", "The Ollivo support group is opening in Telegram — join it if you haven't yet.")
                : t(
                    "Открывается группа Ollivo в MAX — вступите в неё, если ещё не вступили.",
                    "The Ollivo group is opening in MAX — join it if you haven't yet.",
                  )}{" "}
              {copied
                ? t("Текст сообщения уже скопирован: вставьте его (Ctrl+V)", "The message text is already copied: paste it (Ctrl+V)")
                : t("Скопируйте текст ниже, вставьте его в сообщение", "Copy the text below, paste it into a message")}{" "}
              {t("и приложите файл", "and attach the file")} {file}{" "}
              {t("из папки «Загрузки» — она открыта в Проводнике.", "from the Downloads folder — it is open in Explorer.")}
            </p>
            {message && <pre className="log report-message">{message}</pre>}
            <p className="muted small">
              {t(
                "Файл можно просто перетащить в окно мессенджера.",
                "You can simply drag the file into the messenger window.",
              )}
            </p>
          </>
        )}
        <div className="actions">
          <button onClick={onClose}>{t("Закрыть", "Close")}</button>
          {message && (
            <button className="secondary" onClick={async () => setCopied(await copy(message))}>
              {t("Скопировать текст ещё раз", "Copy the text again")}
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="dialog card" role="dialog" aria-label={t("Сообщить о проблеме", "Report a problem")}>
      <h2>{t("Сообщить о проблеме", "Report a problem")}</h2>
      <div className="seg" role="radiogroup" aria-label={t("Что случилось", "What happened")}>
        {kinds().map((k) => (
          <button
            key={k.id}
            role="radio"
            aria-checked={k.id === kind}
            className={k.id === kind ? "active" : ""}
            onClick={() => setKind(k.id)}
          >
            {k.label}
          </button>
        ))}
      </div>
      <textarea
        rows={4}
        value={what}
        placeholder={t("Что вы делали и что пошло не так — своими словами", "What you were doing and what went wrong — in your own words")}
        onChange={(e) => setWhat(e.target.value)}
        autoFocus
      />
      <p className="muted small">
        {t(
          "Вместе с описанием уйдёт отчёт — вот он целиком. Ваших разговоров, файлов и паролей в нём нет, имя пользователя в путях заменено.",
          "A report goes along with the description — here it is in full. Your conversations, files and passwords are not in it, and the user name in paths is replaced.",
        )}
      </p>
      <pre className="log report">{report ? report.full : failed ? "" : t("Собираю отчёт…", "Putting the report together…")}</pre>
      {failed && <p className="error small">{failed}</p>}
      <div className="report-to">
        <span className="small">{t("Куда отправить", "Where to send")}</span>
        <div className="seg" role="radiogroup" aria-label={t("Куда отправить", "Where to send")}>
          {channels().map((c) => (
            <button
              key={c.id}
              role="radio"
              aria-checked={c.id === channel}
              className={c.id === channel ? "active" : ""}
              onClick={() => setChannel(c.id)}
            >
              {c.label}
            </button>
          ))}
        </div>
      </div>
      <p className="muted small">
        {channels().find((c) => c.id === channel)?.hint}{" "}
        {t(
          "Файл отчёта сохранится в «Загрузки» — его нужно будет приложить.",
          "The report file will be saved to Downloads — you'll need to attach it.",
        )}
      </p>
      <div className="actions">
        <button onClick={send} disabled={!report || busy || !what.trim()}>
          {t("Отправить", "Send")}
        </button>
        <button className="secondary" onClick={onClose}>
          {t("Отмена", "Cancel")}
        </button>
      </div>
    </div>
  );
}
