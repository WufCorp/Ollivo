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
import { t } from "../i18n";

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
      {version && <p className="muted small">{t(`У вас версия ${version}`, `You have version ${version}`)}</p>}
      <label className="check">
        <input
          type="checkbox"
          checked={settings.auto_check}
          onChange={(e) => onChange({ ...settings, auto_check: e.target.checked })}
        />
        {t("Проверять обновления автоматически", "Check for updates automatically")}
      </label>
      <p className="muted small">
        {t(
          "Если выключить, Ollivo не будет выходить в интернет сама — проверяйте кнопкой ниже.",
          "If turned off, Ollivo won't go online by itself — check with the button below.",
        )}
      </p>

      <label>
        {t("Версии", "Versions")}
        <select
          value={settings.channel}
          onChange={(e) => onChange({ ...settings, channel: e.target.value as UpdateSettings["channel"] })}
        >
          <option value="stable">{t("Обычные — проверенные", "Regular — tested")}</option>
          <option value="beta">{t("Ранние — новое раньше всех, но бывают сбои", "Early — new things first, but glitches happen")}</option>
        </select>
      </label>

      {found && (
        <p className="ok">
          {t(`Есть версия ${found.version} (у вас ${found.current}).`, `Version ${found.version} is available (you have ${found.current}).`)}
          {found.notes ? ` ${found.notes}` : ""}
        </p>
      )}
      {fresh && <p className="muted">{t("У вас свежая версия.", "You have the latest version.")}</p>}
      {progress && (
        <>
          <p className="small">
            {t("Скачиваю обновление…", "Downloading the update…")} {formatBytes(progress.done)}
            {progress.total ? ` ${t("из", "of")} ${formatBytes(progress.total)}` : ""}
          </p>
          {progress.total ? <progress max={progress.total} value={progress.done} /> : <progress />}
          <p className="muted small">{t("После установки Ollivo перезапустится сама.", "Ollivo will restart by itself after installing.")}</p>
        </>
      )}
      {error && <p className="error">{error}</p>}

      <div className="actions">
        <button className="secondary" onClick={check} disabled={checking || !!progress}>
          {checking ? t("Проверяю…", "Checking…") : t("Проверить сейчас", "Check now")}
        </button>
        {found && !progress && <button onClick={install}>{t("Обновить", "Update")}</button>}
      </div>
    </div>
  );
}
