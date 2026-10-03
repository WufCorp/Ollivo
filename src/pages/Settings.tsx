import { useEffect, useState } from "react";
import {
  chatsOpenFolder,
  languageSet,
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
import SharePanel from "../components/SharePanel";
import { openReport } from "../components/Report";
import { openSupport } from "../support";
import { setLang, t, type Lang } from "../i18n";

/** Через сколько минут простоя выгружать модель; 0 — никогда. */
const unload = (): [number, string][] => [
  [5, t("через 5 минут", "after 5 minutes")],
  [10, t("через 10 минут", "after 10 minutes")],
  [30, t("через полчаса", "after half an hour")],
  [60, t("через час", "after an hour")],
  [0, t("никогда", "never")],
];

/** Предел скорости загрузок, байт/с. Тарифы пишут в мегабитах — их и показываем рядом. */
const speeds = (): [number, string][] => [
  [0, t("без ограничения", "no limit")],
  ...[1, 2, 5, 10, 25].map((mb): [number, string] => [
    mb * 1_000_000,
    t(`${mb} МБ/с (≈ ${mb * 8} Мбит/с)`, `${mb} MB/s (≈ ${mb * 8} Mbit/s)`),
  ]),
];

/** Названия языков — каждый на своём языке: выбравший по ошибке чужой найдёт дорогу назад. */
const LANGUAGES: [Lang, string][] = [
  ["ru", "Русский"],
  ["en", "English"],
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

  if (!settings) return <p className="muted">{t("Загружаю настройки…", "Loading settings…")}</p>;

  /** Язык — сразу и сам по себе, без «Сохранить»: окно на непонятном языке — тупик. */
  const changeLanguage = async (language: Lang) => {
    await languageSet(language);
    setSettings({ ...settings, language });
    setLang(language);
  };

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
      setStatus({ ok: true, text: t("Сохранено", "Saved") });
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    }
  };

  return (
    <>
      <h2>{t("Язык", "Language")}</h2>
      <div className="card form">
        <label>
          Язык · Language
          <select value={settings.language} onChange={(e) => changeLanguage(e.target.value as Lang)}>
            {LANGUAGES.map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <p className="muted small">
          {t(
            "Язык окна и подсказок модели. Сама модель отвечает на том языке, на котором вы спрашиваете.",
            "The language of the window and of the model's instructions. The model itself answers in the language you ask in.",
          )}
        </p>
      </div>

      <h2>{t("Папка программы", "Program folder")}</h2>
      <StorageCard />

      <h2>{t("Сеть", "Network")}</h2>
      <div className="card form">
        <p className="muted small">
          {t(
            "Если модели или программы не скачиваются из-за ограничений в вашем регионе. Через прокси пойдут все загрузки Ollivo.",
            "If models or programs don't download because of restrictions in your region. All Ollivo downloads will go through the proxy.",
          )}
        </p>
        <ProxyForm
          proxy={settings.proxy}
          onChange={(proxy) => update({ proxy })}
          password={password}
          onPassword={setPassword}
          hasPassword={hasPassword}
        />
        <label>
          {t("Скорость загрузок", "Download speed")}
          <select
            value={settings.downloads.limit}
            onChange={(e) => update({ downloads: { limit: Number(e.target.value) } })}
          >
            {/* Значение из файла, которого нет в списке, не теряем — показываем как есть. */}
            {!speeds().some(([v]) => v === settings.downloads.limit) && (
              <option value={settings.downloads.limit}>
                {(settings.downloads.limit / 1_000_000).toFixed(1)} {t("МБ/с", "MB/s")}
              </option>
            )}
            {speeds().map(([v, text]) => (
              <option key={v} value={v}>
                {text}
              </option>
            ))}
          </select>
        </label>
        <p className="muted small">
          {t(
            "Чтобы загрузка модели не забирала весь интернет у остальных в доме. После «Сохранить» действует и на уже идущие загрузки.",
            "So that a model download doesn't take all the internet from everyone else at home. After “Save” it also applies to downloads already running.",
          )}
        </p>
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

      <h2>{t("Оформление", "Appearance")}</h2>
      <div className="card form">
        <label>
          {t("Цвет окна", "Window color")}
          <select
            value={settings.theme}
            onChange={(e) => update({ theme: e.target.value as SettingsData["theme"] })}
          >
            <option value="dark">{t("Тёмное", "Dark")}</option>
            <option value="light">{t("Светлое", "Light")}</option>
          </select>
        </label>
      </div>

      <h2>{t("Видеокарта", "Graphics card")}</h2>
      <div className="card form">
        <label>
          {t("Выгружать модель, если ею не пользуются", "Unload the model when it isn't used")}
          <select
            value={settings.models.unload_after}
            onChange={(e) => update({ models: { ...settings.models, unload_after: Number(e.target.value) } })}
          >
            {unload().map(([min, text]) => (
              <option key={min} value={min}>
                {text}
              </option>
            ))}
          </select>
        </label>
        <p className="muted small">
          {t(
            "Пока модель загружена, она занимает память видеокарты — играм и другим программам её может не хватить. Выгруженная модель загрузится снова сама, когда вы зададите вопрос; это займёт несколько секунд.",
            "While the model is loaded, it takes up graphics card memory — games and other programs may run short of it. An unloaded model loads again by itself when you ask a question; this takes a few seconds.",
          )}
        </p>
      </div>

      <h2>{t("Для других программ", "For other programs")}</h2>
      <SharePanel share={settings.share} onChange={(share) => setSettings({ ...settings, share })} />

      <h2>{t("Обновления Ollivo", "Ollivo updates")}</h2>
      <UpdateCard settings={settings.updates} onChange={(updates) => update({ updates })} />

      <div className="actions save">
        <button onClick={save}>{t("Сохранить", "Save")}</button>
        {status && <span className={status.ok ? "ok" : "error"}>{status.text}</span>}
      </div>

      <h2>{t("Приватность", "Privacy")}</h2>
      <div className="card">
        <p>
          {t(
            "Ollivo работает на этом компьютере. Разговоры, файлы, фото и записи никуда не отправляются — модель считает их здесь же. Программа не собирает статистику о вас.",
            "Ollivo runs on this computer. Conversations, files, photos and recordings aren't sent anywhere — the model processes them right here. The program collects no statistics about you.",
          )}
        </p>
        <p className="muted small">{t("В интернет Ollivo выходит, только чтобы:", "Ollivo goes online only to:")}</p>
        <ul className="muted small">
          <li>{t("найти и скачать модель — с HuggingFace или выбранного выше зеркала;", "find and download a model — from HuggingFace or the mirror chosen above;")}</li>
          <li>{t("скачать движки и компоненты Windows — с GitHub и сайта Microsoft;", "download engines and Windows components — from GitHub and the Microsoft site;")}</li>
          <li>{t("проверить и скачать обновление Ollivo — если проверка включена;", "check for and download an Ollivo update — if checking is on;")}</li>
          <li>{t("проверить прокси или токен — когда вы нажимаете «Проверить».", "check the proxy or token — when you click “Check”.")}</li>
        </ul>
        <p className="muted small">
          {t(
            "Отчёт о проблеме уходит, только если вы сами нажмёте «Отправить», и перед этим вы видите его целиком.",
            "A problem report is sent only if you click “Send” yourself, and before that you see it in full.",
          )}
        </p>
        <p className="muted small">
          {t("Разговоры хранятся здесь:", "Conversations are stored here:")} {chatsDir}
        </p>
        <div className="actions">
          <button className="secondary" onClick={() => chatsOpenFolder()}>
            {t("Открыть папку с разговорами", "Open the conversations folder")}
          </button>
        </div>
      </div>

      <h2>{t("Помощь", "Help")}</h2>
      <div className="card">
        <p className="muted small">
          {t(
            "Что-то не ставится, модель не запускается или работает не так, как вы ждали, — расскажите. Программа сама соберёт отчёт о компьютере, и вы увидите его целиком до отправки.",
            "Something won't install, a model won't start or works differently than you expected — tell us. The program puts together a report about the computer by itself, and you'll see it in full before sending.",
          )}
        </p>
        <div className="actions">
          <button className="secondary" onClick={() => openReport()}>
            {t("Сообщить о проблеме", "Report a problem")}
          </button>
        </div>

        {resetting === "ask" ? (
          <>
            <p className="apart">
              {t(
                "Сеть (прокси и скорость загрузок), HuggingFace, оформление, видеокарта и обновления вернутся к исходным. Пароль прокси и токен HuggingFace удалятся. Модели, разговоры и папка программы останутся.",
                "Network, HuggingFace, appearance, graphics card and updates go back to defaults. The proxy password and HuggingFace token are deleted. Models, conversations, the program folder and the language stay.",
              )}
            </p>
            <div className="actions">
              <button onClick={reset}>{t("Сбросить", "Reset")}</button>
              <button className="secondary" onClick={() => setResetting(null)}>
                {t("Отмена", "Cancel")}
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="muted small apart">
              {t("Если после экспериментов с настройками что-то перестало работать.", "If something stopped working after experimenting with settings.")}
            </p>
            <div className="actions">
              <button className="secondary" onClick={() => setResetting("ask")}>
                {t("Сбросить настройки", "Reset settings")}
              </button>
              {resetting === "done" && <span className="ok">{t("Сброшено — всё как после установки", "Reset — everything is as after installing")}</span>}
            </div>
          </>
        )}
        {resetError && <p className="error">{resetError}</p>}
      </div>

      <h2>{t("Поддержать Ollivo", "Support Ollivo")}</h2>
      <div className="card">
        <p>
          {t(
            "Ollivo делает один человек. Программа бесплатная и без рекламы. Если она вам пригодилась — поддержите её развитие: подпиской на Boosty, разово через ЮMoney или криптовалютой.",
            "Ollivo is made by one person. The program is free and has no ads. If it has been useful to you, support its development: with a Boosty subscription, a one-time YooMoney payment, or crypto.",
          )}
        </p>
        <p className="muted small">{t("Откроется страница на сайте Ollivo, в браузере.", "A page on the Ollivo website opens in the browser.")}</p>
        <div className="actions">
          <button onClick={openSupport}>{t("Поддержать проект", "Support the project")}</button>
        </div>
      </div>
    </>
  );
}
