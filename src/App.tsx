import { useEffect, useState } from "react";
import { settingsGet, updateCheck, type UpdateAvailable } from "./api";
import Chat from "./pages/Chat";
import Computer from "./pages/Computer";
import Models from "./pages/Models";
import Settings from "./pages/Settings";
import Wizard from "./pages/Wizard";
import "./App.css";

const TABS = { chat: "Чат", models: "Модели", computer: "Компьютер", settings: "Настройки" } as const;
type Tab = keyof typeof TABS;

export default function App() {
  const [setupDone, setSetupDone] = useState<boolean | null>(null);
  const [tab, setTab] = useState<Tab>("chat");
  const [update, setUpdate] = useState<UpdateAvailable | null>(null);

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

  return (
    <main className="page">
      <header>
        <h1>Ollivo</h1>
        {setupDone && (
          <nav>
            {(Object.keys(TABS) as Tab[]).map((t) => (
              <button key={t} className={t === tab ? "tab active" : "tab"} onClick={() => setTab(t)}>
                {TABS[t]}
              </button>
            ))}
          </nav>
        )}
      </header>
      {update && setupDone && (
        <div className="update-bar">
          <span>
            Есть версия {update.version}. Обновление займёт минуту.
          </span>
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
      {!setupDone ? (
        <Wizard onDone={() => setSetupDone(true)} />
      ) : tab === "chat" ? (
        <Chat onGoToModels={() => setTab("models")} />
      ) : tab === "models" ? (
        <Models onGoToChat={() => setTab("chat")} />
      ) : tab === "computer" ? (
        <Computer />
      ) : (
        <Settings />
      )}
    </main>
  );
}
