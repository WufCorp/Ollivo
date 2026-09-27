import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  MOVE_TASK,
  formatBytes,
  onStorageMoved,
  onStorageProgress,
  storageClean,
  storageMove,
  storageUsage,
  taskPause,
  tasksRunning,
  type StorageUsage,
} from "../api";
import { t } from "../i18n";

/** Ошибка ядра — строчными («идут загрузки…»); в окне — как предложение. */
const sentence = (e: unknown) => {
  const s = String(e);
  return s.charAt(0).toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? "" : ".");
};

/**
 * Папка программы: сколько заняли модели, движки и мусор, «Очистить» и «Перенести на другой
 * диск». Перенос идёт в ядре; окно только показывает прогресс и итог — его можно закрыть
 * и открыть заново, прогресс подхватится по событиям.
 */
export default function StorageCard() {
  const [usage, setUsage] = useState<StorageUsage | null>(null);
  const [moving, setMoving] = useState<{ to: string; done: number; total: number } | null>(null);
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null);
  const [cleaning, setCleaning] = useState(false);

  const load = () => storageUsage().then(setUsage, (e) => setNote({ ok: false, text: String(e) }));

  useEffect(() => {
    load();
    // Перенос начали и ушли со страницы — вернулись, а он ещё идёт.
    tasksRunning().then((ids) => ids.includes(MOVE_TASK) && setMoving((m) => m ?? { to: "", done: 0, total: 0 }));
    const subs = [
      onStorageProgress((p) => setMoving((m) => ({ to: m?.to ?? "", ...p }))),
      onStorageMoved((m) => {
        setMoving(null);
        if (m.dir) {
          setNote({
            ok: true,
            text:
              t(`Готово: теперь всё лежит в ${m.dir}.`, `Done: everything is now in ${m.dir}.`) +
              (m.stopped
                ? t(" Модель остановлена на время переноса — запустите её снова.", " The model was stopped for the move — start it again.")
                : ""),
          });
        } else {
          setNote({
            ok: false,
            text:
              // «отменено» — служебное слово ядра (`llm::CANCELLED`), не текст для человека.
              m.error === "отменено"
                ? t("Перенос отменён, всё осталось на старом месте.", "The move was cancelled, everything stayed in place.")
                : t(`Не перенесли: ${m.error}.`, `Not moved: ${m.error}.`) +
                  (m.stopped ? t(" Модель остановлена — запустите её снова.", " The model was stopped — start it again.") : ""),
          });
        }
        load();
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  const clean = async () => {
    setCleaning(true);
    setNote(null);
    try {
      const freed = await storageClean();
      setNote({
        ok: true,
        text: freed ? t(`Освободили ${formatBytes(freed)}.`, `Freed ${formatBytes(freed)}.`) : t("Убирать было нечего.", "Nothing to clean up."),
      });
      load();
    } catch (e) {
      setNote({ ok: false, text: sentence(e) });
    } finally {
      setCleaning(false);
    }
  };

  const move = async () => {
    const picked = await open({ directory: true, title: t("Куда перенести папку программы", "Where to move the program folder") });
    if (typeof picked !== "string") return;
    setNote(null);
    try {
      const to = await storageMove(picked);
      setMoving({ to, done: 0, total: 0 });
    } catch (e) {
      setNote({ ok: false, text: sentence(e) });
    }
  };

  const rows: [string, number][] = usage
    ? [
        [t("Модели", "Models"), usage.models],
        [t("Движки", "Engines"), usage.engines],
        [t("Недокачанное и временное", "Unfinished and temporary"), usage.cache],
        [t("Журналы работы", "Logs"), usage.logs],
      ]
    : [];

  return (
    <div className="card">
      <p>
        {t("Здесь движки и скачанные модели:", "Engines and downloaded models are here:")} {usage?.dir ?? "…"}
      </p>
      {usage && (
        <table className="usage">
          <tbody>
            {rows.map(([name, size]) => (
              <tr key={name}>
                <td>{name}</td>
                <td>{formatBytes(size)}</td>
              </tr>
            ))}
            <tr className="muted">
              <td>{t("Свободно на диске", "Free on disk")}</td>
              <td>{formatBytes(usage.free)}</td>
            </tr>
          </tbody>
        </table>
      )}
      <p className="muted small">
        {t(
          "Модели из LM Studio, Ollama и других папок остаются на своих местах и здесь не считаются.",
          "Models from LM Studio, Ollama and other folders stay where they are and aren't counted here.",
        )}
      </p>

      {moving ? (
        <>
          <p>
            {t("Переношу", "Moving")}
            {moving.to ? ` ${t("в", "to")} ${moving.to}` : ""}
            {moving.total ? `: ${formatBytes(moving.done)} ${t("из", "of")} ${formatBytes(moving.total)}` : "…"}
          </p>
          {moving.total ? <progress value={moving.done} max={moving.total} /> : <progress />}
          <p className="muted small">
            {t(
              "Пока идёт перенос, модели не запускаются и ничего не скачивается. Старая папка удалится, только когда всё скопируется, — если отменить, всё останется как было.",
              "While moving, models don't start and nothing downloads. The old folder is deleted only after everything is copied — if you cancel, everything stays as it was.",
            )}
          </p>
          <div className="actions">
            <button className="secondary" onClick={() => taskPause(MOVE_TASK)}>
              {t("Отменить", "Cancel")}
            </button>
          </div>
        </>
      ) : (
        <div className="actions">
          <button className="secondary" onClick={clean} disabled={cleaning || !usage?.cache}>
            {cleaning ? t("Убираю…", "Cleaning…") : t("Очистить недокачанное и временное", "Clean up unfinished and temporary")}
          </button>
          <button className="secondary" onClick={move}>
            {t("Перенести на другой диск…", "Move to another disk…")}
          </button>
        </div>
      )}
      {note && <p className={note.ok ? "ok" : "error"}>{note.text}</p>}
    </div>
  );
}
