import { useEffect, useState } from "react";
import {
  proxyTest,
  settingsGet,
  settingsSave,
  type ProxyKind,
  type ProxyReport,
  type ProxySettings,
  type Settings as SettingsData,
} from "../api";

export default function Settings() {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [dataDir, setDataDir] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  // undefined — пароль не трогали, сохранённый остаётся.
  const [password, setPassword] = useState<string | undefined>(undefined);
  const [report, setReport] = useState<ProxyReport | null>(null);
  const [testing, setTesting] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    settingsGet().then((v) => {
      setSettings(v.settings);
      setDataDir(v.data_dir);
      setHasPassword(v.proxy_has_password);
    });
  }, []);

  if (!settings) return <p className="muted">Загружаю настройки…</p>;

  const proxy = settings.proxy;
  const setProxy = (patch: Partial<ProxySettings>) => {
    setSettings({ ...settings, proxy: { ...proxy, ...patch } });
    setReport(null);
    setStatus(null);
  };

  const test = async () => {
    setTesting(true);
    setReport(null);
    try {
      setReport(await proxyTest(proxy, password));
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    try {
      // Если логин не нужен, сохранённый пароль больше не нужен тоже.
      const pw = proxy.auth ? password : "";
      await settingsSave(settings, pw);
      if (pw !== undefined) setHasPassword(pw !== "");
      setPassword(undefined);
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
        <label className="check">
          <input type="checkbox" checked={proxy.enabled} onChange={(e) => setProxy({ enabled: e.target.checked })} />
          Подключаться через прокси
        </label>
        <p className="muted small">
          Если модели или программы не скачиваются из-за ограничений в вашем регионе. Через прокси пойдут все
          загрузки Ollivo.
        </p>

        {proxy.enabled && (
          <>
            <div className="row">
              <label>
                Протокол
                <select value={proxy.kind} onChange={(e) => setProxy({ kind: e.target.value as ProxyKind })}>
                  <option value="http">HTTP</option>
                  <option value="socks5">SOCKS5</option>
                </select>
              </label>
              <label className="grow">
                Адрес
                <input
                  value={proxy.host}
                  placeholder="127.0.0.1"
                  spellCheck={false}
                  onChange={(e) => setProxy({ host: e.target.value.trim() })}
                />
              </label>
              <label className="port">
                Порт
                <input
                  inputMode="numeric"
                  value={proxy.port || ""}
                  placeholder={proxy.kind === "socks5" ? "1080" : "8080"}
                  onChange={(e) => setProxy({ port: Math.min(65535, Number(e.target.value.replace(/\D/g, "")) || 0) })}
                />
              </label>
            </div>

            <label className="check">
              <input type="checkbox" checked={proxy.auth} onChange={(e) => setProxy({ auth: e.target.checked })} />
              Нужен логин и пароль
            </label>

            {proxy.auth && (
              <div className="row">
                <label className="grow">
                  Логин
                  <input
                    value={proxy.username}
                    spellCheck={false}
                    autoComplete="off"
                    onChange={(e) => setProxy({ username: e.target.value })}
                  />
                </label>
                <label className="grow">
                  Пароль
                  <input
                    type="password"
                    value={password ?? ""}
                    placeholder={hasPassword ? "сохранён" : ""}
                    autoComplete="off"
                    onChange={(e) => {
                      setPassword(e.target.value);
                      setReport(null);
                    }}
                  />
                </label>
              </div>
            )}
          </>
        )}

        <div className="actions">
          {proxy.enabled && (
            <button className="secondary" onClick={test} disabled={testing}>
              {testing ? "Проверяю…" : "Проверить"}
            </button>
          )}
          <button onClick={save}>Сохранить</button>
          {status && <span className={status.ok ? "ok" : "error"}>{status.text}</span>}
        </div>

        {report && (
          <ul className="checks">
            {report.checks.map((c) => (
              <li key={c.name} className={c.ok ? "ok" : "error"}>
                {c.ok ? "✓" : "✗"} {c.name}: {c.message}
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}
