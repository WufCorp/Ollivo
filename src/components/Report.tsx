import { useEffect, useState } from "react";
import { reportMake, reportSend, type Report, type ReportKind } from "../api";

const KINDS: { id: ReportKind; label: string }[] = [
  { id: "install", label: "Не ставится" },
  { id: "model", label: "Модель не запускается" },
  { id: "other", label: "Другое" },
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
  const [what, setWhat] = useState(error ? `На экране было: «${error}»\n\n` : "");
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
      <div className="dialog card" role="dialog" aria-label="Сообщить о проблеме">
        <h2>Почти готово</h2>
        <p>
          В браузере открылась форма на GitHub — там уже всё заполнено. Перетащите в поле «Отчёт» файл{" "}
          <b>{saved.split(/[\\/]/).pop()}</b> из папки «Загрузки» (она открыта в Проводнике) и отправьте форму.
        </p>
        <p className="muted small">Нужен аккаунт на GitHub — он бесплатный.</p>
        <div className="actions">
          <button onClick={onClose}>Закрыть</button>
        </div>
      </div>
    );
  }

  return (
    <div className="dialog card" role="dialog" aria-label="Сообщить о проблеме">
      <h2>Сообщить о проблеме</h2>
      <div className="seg" role="radiogroup" aria-label="Что случилось">
        {KINDS.map((k) => (
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
        placeholder="Что вы делали и что пошло не так — своими словами"
        onChange={(e) => setWhat(e.target.value)}
        autoFocus
      />
      <p className="muted small">
        Вместе с описанием уйдёт отчёт — вот он целиком. Ваших разговоров, файлов и паролей в нём нет, имя
        пользователя в путях заменено.
      </p>
      <pre className="log report">{report ? report.full : failed ? "" : "Собираю отчёт…"}</pre>
      {failed && <p className="error small">{failed}</p>}
      <p className="muted small">
        «Отправить» откроет форму на GitHub, а файл отчёта сохранит в «Загрузки» — его нужно будет перетащить в форму.
      </p>
      <div className="actions">
        <button onClick={send} disabled={!report || busy || !what.trim()}>
          Отправить
        </button>
        <button className="secondary" onClick={onClose}>
          Отмена
        </button>
      </div>
    </div>
  );
}
