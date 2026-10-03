import { useEffect, useState } from "react";
import { settingsGet, updateCheck, type UpdateAvailable } from "./api";
import { setLang, t, useLang } from "./i18n";
import ChatList from "./components/ChatList";
import Icon, { Logo, type IconName } from "./components/Icon";
import { ReportHost, openReport } from "./components/Report";
import StatusBar from "./components/StatusBar";
import { openSupport } from "./support";
import Catalog from "./pages/Catalog";
import Chat from "./pages/Chat";
import Computer from "./pages/Computer";
import Images from "./pages/Images";
import Models from "./pages/Models";
import Settings from "./pages/Settings";
import Wizard from "./pages/Wizard";
import "./App.css";

const TABS = ["chat", "images", "catalog", "models", "computer", "settings"] as const;
type Tab = (typeof TABS)[number];

const tabName = (tab: Tab) =>
  ({
    chat: t("Чат", "Chat"),
    images: t("Картинки", "Images"),
    catalog: t("Каталог", "Catalog"),
    models: t("Модели", "Models"),
    computer: t("Компьютер", "Computer"),
    settings: t("Настройки", "Settings"),
  })[tab];

const TAB_ICONS: Record<Tab, IconName> = {
  chat: "chat",
  images: "image",
  catalog: "catalog",
  models: "models",
  computer: "computer",
  settings: "settings",
};

export default function App() {
  // Смена языка перерисовывает всё дерево отсюда.
  useLang();
  const [setupDone, setSetupDone] = useState<boolean | null>(null);
  const [tab, setTab] = useState<Tab>("chat");
  const [update, setUpdate] = useState<UpdateAvailable | null>(null);
  /** Открытый разговор; `null` — новый. `chatsKey` заставляет список перечитаться. */
  const [chatId, setChatId] = useState<string | null>(null);
  const [chatsKey, setChatsKey] = useState(0);

  useEffect(() => {
    settingsGet().then((v) => {
      setLang(v.settings.language);
      setSetupDone(v.settings.setup_done);
      // Тихая проверка при запуске, если разрешена. Ошибку не показываем:
      // человек не просил проверять, а интернета может и не быть.
      if (v.settings.setup_done && v.settings.updates.auto_check) {
        updateCheck().then(setUpdate, () => {});
      }
    });
  }, []);

  if (setupDone === null) return null;

  // Мастер первого запуска — на весь экран, без меню: делать больше пока нечего.
  if (!setupDone) {
    return (
      <main className="page">
        <header className="brand">
          <Logo size={40} />
          <h1>Ollivo</h1>
        </header>
        <Wizard onDone={() => setSetupDone(true)} />
        <ReportHost />
      </main>
    );
  }

  return (
    <div className="app">
      {/* Узкая панель значков: разделы всегда под рукой и не отнимают место у разговора. */}
      <nav className="rail">
        <Logo size={32} />
        {TABS.map((id) => (
          <button
            key={id}
            className={id === tab ? "rail-tab active" : "rail-tab"}
            aria-current={id === tab ? "page" : undefined}
            onClick={() => setTab(id)}
          >
            <Icon name={TAB_ICONS[id]} size={20} />
            {tabName(id)}
          </button>
        ))}
        <button className="rail-tab bottom" title={t("Поддержать проект — откроется сайт", "Support the project — opens the website")} onClick={openSupport}>
          <Icon name="heart" size={20} />
          {t("Поддержать", "Support")}
        </button>
        <button className="rail-tab" title={t("Сообщить о проблеме", "Report a problem")} onClick={() => openReport()}>
          <Icon name="report" size={20} />
          {t("Отчёт", "Report")}
        </button>
      </nav>

      {tab === "chat" && (
        <aside className="side">
          <ChatList
            current={chatId}
            refresh={chatsKey}
            onPick={setChatId}
            onNew={() => setChatId(null)}
            onRemoved={(id) => id === chatId && setChatId(null)}
          />
        </aside>
      )}

      <div className="work">
      {/* Чат занимает всё окно: лента прокручивается сама, ввод прибит к низу. */}
      <main className={tab === "chat" ? "main chat" : "main"}>
        {update && (
          <div className="update-bar">
            <span>
              {t(
                `Есть версия ${update.version}. Обновление займёт минуту.`,
                `Version ${update.version} is available. Updating takes a minute.`,
              )}
            </span>
            <button
              onClick={() => {
                setTab("settings");
                setUpdate(null);
              }}
            >
              {t("Посмотреть", "Show")}
            </button>
            <button className="link" onClick={() => setUpdate(null)}>
              {t("Позже", "Later")}
            </button>
          </div>
        )}
        <div className="content">
          {/* Чат не размонтируется на других вкладках: ответ модели идёт, пока человек смотрит
              «Компьютер», и не теряется (раньше вопрос пропадал, а ответ дописывался к прошлому). */}
          <div style={{ display: tab === "chat" ? "contents" : "none" }}>
            <Chat
              active={tab === "chat"}
              chatId={chatId}
              onSaved={(c, open) => {
                if (open) setChatId(c.id);
                setChatsKey((n) => n + 1);
              }}
              onGoToModels={() => setTab("models")}
              onGo={setTab}
              onNewChat={() => setChatId(null)}
            />
          </div>
          {tab === "chat" ? null : tab === "images" ? (
            <Images onGo={setTab} />
          ) : tab === "catalog" ? (
            <Catalog onGoToChat={() => setTab("chat")} />
          ) : tab === "models" ? (
            <Models onGoToChat={() => setTab("chat")} onGo={setTab} />
          ) : tab === "computer" ? (
            <Computer />
          ) : (
            <Settings />
          )}
        </div>
      </main>
      <StatusBar />
      </div>
      <ReportHost />
    </div>
  );
}
