import { useEffect, useState } from "react";
import { settingsGet, settingsSave, type Settings as SettingsData } from "../api";
import HfForm from "../components/HfForm";
import ProxyForm from "../components/ProxyForm";
import UpdateCard from "../components/UpdateCard";
import { openReport } from "../components/Report";

/** Через сколько минут простоя выгружать модель; 0 — никогда. */
const UNLOAD: [number, string][] = [
  [5, "через 5 минут"],
  [10, "через 10 минут"],
  [30, "через полчаса"],
  [60, "через час"],
  [0, "никогда"],
];

export default function Settings() {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [dataDir, setDataDir] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [hasToken, setHasToken] = useState(false);
  // undefined — секрет не трогали, сохранённый остаётся.
  const [password, setPassword] = useState<string | undefined>(undefined);
  const [token, setToken] = useState<string | undefined>(undefined);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    settingsGet().then((v) => {
      setSettings(v.settings);
      setDataDir(v.data_dir);
      setHasPassword(v.proxy_has_password);
      setHasToken(v.hf_has_token);
    });
  }, []);

  if (!settings) return <p className="muted">Загружаю настройки…</p>;

  const update = (patch: Partial<SettingsData>) => {
    setSettings({ ...settings, ...patch });
    setStatus(null);
  };

  const save = async () => {
    try {
      // Если логин прокси не нужен, сохранённый пароль тоже не нужен.
      const proxyPassword = settings.proxy.auth ? password : "";
      await settingsSave(settings, { proxyPassword, hfToken: token });
      if (proxyPassword !== undefined) setHasPassword(proxyPassword !== "");
      if (token !== undefined) setHasToken(token.trim() !== "");
      setPassword(undefined);
      setToken(undefined);
      setStatus({ ok: true, text: "Сохранено" });
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    }
  };

  return (
    <>
      <h2>Папка программы</h2>
      <p className="muted">Сюда ставятся движки и скачиваются модели: {dataDir}</p>

      <h2>Сеть</h2>
      <div className="card form">
        <p className="muted small">
          Если модели или программы не скачиваются из-за ограничений в вашем регионе. Через прокси пойдут все загрузки
          Ollivo.
        </p>
        <ProxyForm
          proxy={settings.proxy}
          onChange={(proxy) => update({ proxy })}
          password={password}
          onPassword={setPassword}
          hasPassword={hasPassword}
        />
      </div>

      <h2>HuggingFace</h2>
      <div className="card form">
        <HfForm
          hf={settings.hf}
          onChange={(hf) => update({ hf })}
          token={token}
          onToken={setToken}
          hasToken={hasToken}
        />
      </div>

      <h2>Видеокарта</h2>
      <div className="card form">
        <label>
          Выгружать модель, если ею не пользуются
          <select
            value={settings.models.unload_after}
            onChange={(e) => update({ models: { ...settings.models, unload_after: Number(e.target.value) } })}
          >
            {UNLOAD.map(([min, text]) => (
              <option key={min} value={min}>
                {text}
              </option>
            ))}
          </select>
        </label>
        <p className="muted small">
          Пока модель загружена, она занимает память видеокарты — играм и другим программам её может не хватить.
          Выгруженная модель загрузится снова сама, когда вы зададите вопрос; это займёт несколько секунд.
        </p>
      </div>

      <h2>Обновления Ollivo</h2>
      <UpdateCard settings={settings.updates} onChange={(updates) => update({ updates })} />

      <div className="actions save">
        <button onClick={save}>Сохранить</button>
        {status && <span className={status.ok ? "ok" : "error"}>{status.text}</span>}
      </div>

      <h2>Помощь</h2>
      <div className="card">
        <p className="muted small">
          Что-то не ставится, модель не запускается или работает не так, как вы ждали, — расскажите. Программа сама
          соберёт отчёт о компьютере, и вы увидите его целиком до отправки.
        </p>
        <div className="actions">
          <button className="secondary" onClick={() => openReport()}>
            Сообщить о проблеме
          </button>
        </div>
      </div>
    </>
  );
}
