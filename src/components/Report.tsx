import { useEffect, useState } from "react";
import { reportMake, reportSend, type Report, type ReportKind } from "../api";
import { t } from "../i18n";

const kinds = (): { id: ReportKind; label: string }[] => [
  { id: "install", label: t("Не ставится", "Won't install") },
  { id: "model", label: t("Модель не запускается", "Model won't start") },
  { id: "other", label: t("Другое", "Other") },
];

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
 * и отправляет сам: файл — в «Загрузки», форма issue — в браузере, из его аккаунта GitHub.
 */
function ReportDialog({ kind: startKind = "other", error, onClose }: Opts & { onClose: () => void }) {
  const [kind, setKind] = useState<ReportKind>(startKind);
  const [what, setWhat] = useState(error ? t(`На экране было: «${error}»\n\n`, `The screen said: “${error}”\n\n`) : "");
  const [report, setReport] = useState<Report | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    reportMake().then(setReport, (e) => setFailed(String(e)));
  }, []);

  const send = async () => {
    if (!report) return;
    setBusy(true);
    setFailed(null);
    try {
      setSaved(await reportSend(kind, what, report));
    } catch (e) {
      setFailed(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (saved) {
    return (
      <div className="dialog card" role="dialog" aria-label={t("Сообщить о проблеме", "Report a problem")}>
        <h2>{t("Почти готово", "Almost done")}</h2>
        <p>
          {t(
            "В браузере открылась форма на GitHub — там уже всё заполнено. Перетащите в поле «Отчёт» файл",
            "A GitHub form has opened in your browser — everything is already filled in. Drag the file",
          )}{" "}
          <b>{saved.split(/[\\/]/).pop()}</b>{" "}
          {t(
            "из папки «Загрузки» (она открыта в Проводнике) и отправьте форму.",
            "from the Downloads folder (it is open in Explorer) into the “Report” field and submit the form.",
          )}
        </p>
        <p className="muted small">{t("Нужен аккаунт на GitHub — он бесплатный.", "You need a GitHub account — it's free.")}</p>
        <div className="actions">
          <button onClick={onClose}>{t("Закрыть", "Close")}</button>
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
      <p className="muted small">
        {t(
          "«Отправить» откроет форму на GitHub, а файл отчёта сохранит в «Загрузки» — его нужно будет перетащить в форму.",
          "“Send” opens a form on GitHub and saves the report file to Downloads — you'll need to drag it into the form.",
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
