import { useEffect, useState } from "react";
import { settingsGet, settingsSave, type Settings as SettingsData } from "../api";
import HfForm from "../components/HfForm";
import ProxyForm from "../components/ProxyForm";
import UpdateCard from "../components/UpdateCard";

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

      <h2>Обновления Ollivo</h2>
      <UpdateCard settings={settings.updates} onChange={(updates) => update({ updates })} />

      <div className="actions save">
        <button onClick={save}>Сохранить</button>
        {status && <span className={status.ok ? "ok" : "error"}>{status.text}</span>}
      </div>
    </>
  );
}
