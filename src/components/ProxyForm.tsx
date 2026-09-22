import { useState } from "react";
import { proxyTest, type ProxyKind, type ProxyReport, type ProxySettings } from "../api";

interface Props {
  proxy: ProxySettings;
  onChange: (p: ProxySettings) => void;
  /** undefined — пароль не трогали, сохранённый остаётся. */
  password: string | undefined;
  onPassword: (p: string) => void;
  hasPassword: boolean;
  /** Итог проверки — мастеру, чтобы пустить дальше. */
  onReport?: (r: ProxyReport) => void;
}

export default function ProxyForm({ proxy, onChange, password, onPassword, hasPassword, onReport }: Props) {
  const [report, setReport] = useState<ProxyReport | null>(null);
  const [testing, setTesting] = useState(false);

  const set = (patch: Partial<ProxySettings>) => {
    onChange({ ...proxy, ...patch });
    setReport(null);
  };

  const test = async () => {
    setTesting(true);
    setReport(null);
    try {
      const r = await proxyTest(proxy, password);
      setReport(r);
      onReport?.(r);
    } finally {
      setTesting(false);
    }
  };

  return (
    <>
      <label className="check">
        <input type="checkbox" checked={proxy.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
        Подключаться через прокси
      </label>

      {proxy.enabled && (
        <>
          <div className="row">
            <label>
              Протокол
              <select value={proxy.kind} onChange={(e) => set({ kind: e.target.value as ProxyKind })}>
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
                onChange={(e) => set({ host: e.target.value.trim() })}
              />
            </label>
            <label className="port">
              Порт
              <input
                inputMode="numeric"
                value={proxy.port || ""}
                placeholder={proxy.kind === "socks5" ? "1080" : "8080"}
                onChange={(e) => set({ port: Math.min(65535, Number(e.target.value.replace(/\D/g, "")) || 0) })}
              />
            </label>
          </div>

          <label className="check">
            <input type="checkbox" checked={proxy.auth} onChange={(e) => set({ auth: e.target.checked })} />
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
                  onChange={(e) => set({ username: e.target.value })}
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
                    onPassword(e.target.value);
                    setReport(null);
                  }}
                />
              </label>
            </div>
          )}

          <div className="actions">
            <button className="secondary" onClick={test} disabled={testing}>
              {testing ? "Проверяю…" : "Проверить"}
            </button>
          </div>

          {report && <CheckList checks={report.checks} />}
        </>
      )}
    </>
  );
}

export function CheckList({ checks }: { checks: { name: string; ok: boolean; message: string }[] }) {
  return (
    <ul className="checks">
      {checks.map((c) => (
        <li key={c.name} className={c.ok ? "ok" : "error"}>
          {c.ok ? "✓" : "✗"} {c.name}: {c.message}
        </li>
      ))}
    </ul>
  );
}
