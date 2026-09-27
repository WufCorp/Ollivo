import { useEffect, useState } from "react";
import {
  chatsOpenFolder,
  settingsGet,
  settingsReset,
  settingsSave,
  type Settings as SettingsData,
  type SettingsView,
} from "../api";
import HfForm from "../components/HfForm";
import ProxyForm from "../components/ProxyForm";
import UpdateCard from "../components/UpdateCard";
import StorageCard from "../components/StorageCard";
import { openReport } from "../components/Report";
import { openSupport } from "../support";

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
  const [hasPassword, setHasPassword] = useState(false);
  const [hasToken, setHasToken] = useState(false);
  // undefined — секрет не трогали, сохранённый остаётся.
  const [password, setPassword] = useState<string | undefined>(undefined);
  const [token, setToken] = useState<string | undefined>(undefined);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const [chatsDir, setChatsDir] = useState("");
  /** «Сбросить настройки»: `ask` — ждём подтверждения, `done` — сбросили (итог пишем у кнопки:
   *  «Сохранено» наверху, у «Сохранить», человек внизу страницы не увидит). */
  const [resetting, setResetting] = useState<"ask" | "done" | null>(null);
  const [resetError, setResetError] = useState<string | null>(null);

  const show = (v: SettingsView) => {
    setSettings(v.settings);
    setChatsDir(v.chats_dir);
    setHasPassword(v.proxy_has_password);
    setHasToken(v.hf_has_token);
    setPassword(undefined);
    setToken(undefined);
  };

  useEffect(() => {
    settingsGet().then(show);
  }, []);

  if (!settings) return <p className="muted">Загружаю настройки…</p>;

  const reset = async () => {
    setResetError(null);
    try {
      show(await settingsReset());
      setStatus(null);
      setResetting("done");
    } catch (e) {
      setResetError(String(e));
      setResetting(null);
    }
  };

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
      <StorageCard />

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

      <h2>Оформление</h2>
      <div className="card form">
        <label>
          Цвет окна
          <select
            value={settings.theme}
            onChange={(e) => update({ theme: e.target.value as SettingsData["theme"] })}
          >
            <option value="dark">Тёмное</option>
            <option value="light">Светлое</option>
          </select>
        </label>
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

      <h2>Приватность</h2>
      <div className="card">
        <p>
          Ollivo работает на этом компьютере. Разговоры, файлы, фото и записи никуда не отправляются — модель считает их
          здесь же. Программа не собирает статистику о вас.
        </p>
        <p className="muted small">В интернет Ollivo выходит, только чтобы:</p>
        <ul className="muted small">
          <li>найти и скачать модель — с HuggingFace или выбранного выше зеркала;</li>
          <li>скачать движки и компоненты Windows — с GitHub и сайта Microsoft;</li>
          <li>проверить и скачать обновление Ollivo — если проверка включена;</li>
          <li>проверить прокси или токен — когда вы нажимаете «Проверить».</li>
        </ul>
        <p className="muted small">
          Отчёт о проблеме уходит, только если вы сами нажмёте «Отправить», и перед этим вы видите его целиком.
        </p>
        <p className="muted small">Разговоры хранятся здесь: {chatsDir}</p>
        <div className="actions">
          <button className="secondary" onClick={() => chatsOpenFolder()}>
            Открыть папку с разговорами
          </button>
        </div>
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

        {resetting === "ask" ? (
          <>
            <p className="apart">
              Сеть, HuggingFace, оформление, видеокарта и обновления вернутся к исходным. Пароль прокси и токен
              HuggingFace удалятся. Модели, разговоры и папка программы останутся.
            </p>
            <div className="actions">
              <button onClick={reset}>Сбросить</button>
              <button className="secondary" onClick={() => setResetting(null)}>
                Отмена
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="muted small apart">Если после экспериментов с настройками что-то перестало работать.</p>
            <div className="actions">
              <button className="secondary" onClick={() => setResetting("ask")}>
                Сбросить настройки
              </button>
              {resetting === "done" && <span className="ok">Сброшено — всё как после установки</span>}
            </div>
          </>
        )}
        {resetError && <p className="error">{resetError}</p>}
      </div>

      <h2>Поддержать Ollivo</h2>
      <div className="card">
        <p>
          Ollivo делает один человек. Программа бесплатная и без рекламы. Если она вам пригодилась — поддержите её
          развитие: подпиской на Boosty, разово через ЮMoney или криптовалютой.
        </p>
        <p className="muted small">Откроется страница на сайте Ollivo, в браузере.</p>
        <div className="actions">
          <button onClick={openSupport}>Поддержать проект</button>
        </div>
      </div>
    </>
  );
}
