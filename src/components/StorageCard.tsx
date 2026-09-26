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
            text: `Готово: теперь всё лежит в ${m.dir}. Модель остановлена на время переноса — запустите её снова.`,
          });
        } else {
          setNote({
            ok: false,
            text: m.error === "отменено" ? "Перенос отменён, всё осталось на старом месте." : `Не перенесли: ${m.error}.`,
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
      setNote({ ok: true, text: freed ? `Освободили ${formatBytes(freed)}.` : "Убирать было нечего." });
      load();
    } catch (e) {
      setNote({ ok: false, text: `${String(e)}.` });
    } finally {
      setCleaning(false);
    }
  };

  const move = async () => {
    const picked = await open({ directory: true, title: "Куда перенести папку программы" });
    if (typeof picked !== "string") return;
    setNote(null);
    try {
      const to = await storageMove(picked);
      setMoving({ to, done: 0, total: 0 });
    } catch (e) {
      setNote({ ok: false, text: `${String(e)}.` });
    }
  };

  const rows: [string, number][] = usage
    ? [
        ["Модели", usage.models],
        ["Движки", usage.engines],
        ["Недокачанное и временное", usage.cache],
        ["Журналы работы", usage.logs],
      ]
    : [];

  return (
    <div className="card">
      <p>Здесь движки и скачанные модели: {usage?.dir ?? "…"}</p>
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
              <td>Свободно на диске</td>
              <td>{formatBytes(usage.free)}</td>
            </tr>
          </tbody>
        </table>
      )}
      <p className="muted small">
        Модели из LM Studio, Ollama и других папок остаются на своих местах и здесь не считаются.
      </p>

      {moving ? (
        <>
          <p>
            Переношу{moving.to ? ` в ${moving.to}` : ""}
            {moving.total ? `: ${formatBytes(moving.done)} из ${formatBytes(moving.total)}` : "…"}
          </p>
          {moving.total ? <progress value={moving.done} max={moving.total} /> : <progress />}
          <p className="muted small">
            Пока идёт перенос, модели не запускаются и ничего не скачивается. Старая папка удалится, только когда
            всё скопируется, — если отменить, всё останется как было.
          </p>
          <div className="actions">
            <button className="secondary" onClick={() => taskPause(MOVE_TASK)}>
              Отменить
            </button>
          </div>
        </>
      ) : (
        <div className="actions">
          <button className="secondary" onClick={clean} disabled={cleaning || !usage?.cache}>
            {cleaning ? "Убираю…" : "Очистить недокачанное и временное"}
          </button>
          <button className="secondary" onClick={move}>
            Перенести на другой диск…
          </button>
        </div>
      )}
      {note && <p className={note.ok ? "ok" : "error"}>{note.text}</p>}
    </div>
  );
}
