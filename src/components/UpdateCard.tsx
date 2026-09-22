import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import {
  formatBytes,
  onUpdateFailed,
  onUpdateProgress,
  updateCheck,
  updateInstall,
  type UpdateAvailable,
  type UpdateProgress,
  type UpdateSettings,
} from "../api";

/**
 * Обновления программы: переключатель автопроверки, канал, «Проверить сейчас»
 * и установка с прогрессом. Выключенная автопроверка = ни одного запроса без спроса.
 */
export default function UpdateCard({
  settings,
  onChange,
}: {
  settings: UpdateSettings;
  onChange: (s: UpdateSettings) => void;
}) {
  const [checking, setChecking] = useState(false);
  const [found, setFound] = useState<UpdateAvailable | null>(null);
  const [fresh, setFresh] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion, () => {});
    const subs = [
      onUpdateProgress(setProgress),
      onUpdateFailed((e) => {
        setProgress(null);
        setError(e);
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  const check = async () => {
    setChecking(true);
    setError(null);
    setFresh(false);
    try {
      const u = await updateCheck(settings.channel);
      setFound(u);
      setFresh(!u);
    } catch (e) {
      setError(String(e));
    } finally {
      setChecking(false);
    }
  };

  const install = () => {
    setError(null);
    setProgress({ done: 0, total: null });
    updateInstall().catch((e) => {
      setProgress(null);
      setError(String(e));
    });
  };

  return (
    <div className="card form">
      {version && <p className="muted small">У вас версия {version}</p>}
      <label className="check">
        <input
          type="checkbox"
          checked={settings.auto_check}
          onChange={(e) => onChange({ ...settings, auto_check: e.target.checked })}
        />
        Проверять обновления автоматически
      </label>
      <p className="muted small">
        Если выключить, Ollivo не будет выходить в интернет сама — проверяйте кнопкой ниже.
      </p>

      <label>
        Версии
        <select
          value={settings.channel}
          onChange={(e) => onChange({ ...settings, channel: e.target.value as UpdateSettings["channel"] })}
        >
          <option value="stable">Обычные — проверенные</option>
          <option value="beta">Ранние — новое раньше всех, но бывают сбои</option>
        </select>
      </label>

      {found && (
        <p className="ok">
          Есть версия {found.version} (у вас {found.current}).{found.notes ? ` ${found.notes}` : ""}
        </p>
      )}
      {fresh && <p className="muted">У вас свежая версия.</p>}
      {progress && (
        <>
          <p className="small">
            Скачиваю обновление… {formatBytes(progress.done)}
            {progress.total ? ` из ${formatBytes(progress.total)}` : ""}
          </p>
          {progress.total ? <progress max={progress.total} value={progress.done} /> : <progress />}
          <p className="muted small">После установки Ollivo перезапустится сама.</p>
        </>
      )}
      {error && <p className="error">{error}</p>}

      <div className="actions">
        <button className="secondary" onClick={check} disabled={checking || !!progress}>
          {checking ? "Проверяю…" : "Проверить сейчас"}
        </button>
        {found && !progress && <button onClick={install}>Обновить</button>}
      </div>
    </div>
  );
}
