import { useEffect, useState } from "react";
import { settingsGet } from "./api";
import Computer from "./pages/Computer";
import Settings from "./pages/Settings";
import Wizard from "./pages/Wizard";
import "./App.css";

const TABS = { computer: "Компьютер", settings: "Настройки" } as const;
type Tab = keyof typeof TABS;

export default function App() {
  const [setupDone, setSetupDone] = useState<boolean | null>(null);
  const [tab, setTab] = useState<Tab>("computer");

  useEffect(() => {
    settingsGet().then((v) => setSetupDone(v.settings.setup_done));
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
      {!setupDone ? (
        <Wizard onDone={() => setSetupDone(true)} />
      ) : tab === "computer" ? (
        <Computer />
      ) : (
        <Settings />
      )}
    </main>
  );
}
