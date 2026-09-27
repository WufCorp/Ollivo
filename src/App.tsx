import { useEffect, useState } from "react";
import { settingsGet, updateCheck, type UpdateAvailable } from "./api";
import ChatList from "./components/ChatList";
import Icon, { Logo, type IconName } from "./components/Icon";
import { ReportHost, openReport } from "./components/Report";
import StatusBar from "./components/StatusBar";
import Catalog from "./pages/Catalog";
import Chat from "./pages/Chat";
import Computer from "./pages/Computer";
import Models from "./pages/Models";
import Settings from "./pages/Settings";
import Wizard from "./pages/Wizard";
import "./App.css";

const TABS = {
  chat: "Чат",
  catalog: "Каталог",
  models: "Модели",
  computer: "Компьютер",
  settings: "Настройки",
} as const;
type Tab = keyof typeof TABS;

const TAB_ICONS: Record<Tab, IconName> = {
  chat: "chat",
  catalog: "catalog",
  models: "models",
  computer: "computer",
  settings: "settings",
};

export default function App() {
  const [setupDone, setSetupDone] = useState<boolean | null>(null);
  const [tab, setTab] = useState<Tab>("chat");
  const [update, setUpdate] = useState<UpdateAvailable | null>(null);
  /** Открытый разговор; `null` — новый. `chatsKey` заставляет список перечитаться. */
  const [chatId, setChatId] = useState<string | null>(null);
  const [chatsKey, setChatsKey] = useState(0);

  useEffect(() => {
    settingsGet().then((v) => {
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
        {(Object.keys(TABS) as Tab[]).map((t) => (
          <button
            key={t}
            className={t === tab ? "rail-tab active" : "rail-tab"}
            aria-current={t === tab ? "page" : undefined}
            onClick={() => setTab(t)}
          >
            <Icon name={TAB_ICONS[t]} size={20} />
            {TABS[t]}
          </button>
        ))}
        <button className="rail-tab bottom" title="Сообщить о проблеме" onClick={() => openReport()}>
          <Icon name="report" size={20} />
          Отчёт
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
            <span>Есть версия {update.version}. Обновление займёт минуту.</span>
            <button
              onClick={() => {
                setTab("settings");
                setUpdate(null);
              }}
            >
              Посмотреть
            </button>
            <button className="link" onClick={() => setUpdate(null)}>
              Позже
            </button>
          </div>
        )}
        <div className="content">
          {tab === "chat" ? (
            <Chat
              chatId={chatId}
              onSaved={(c) => {
                setChatId(c.id);
                setChatsKey((n) => n + 1);
              }}
              onGoToModels={() => setTab("models")}
              onGo={setTab}
              onNewChat={() => setChatId(null)}
            />
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
