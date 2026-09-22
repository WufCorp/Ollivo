import { useState } from "react";
import Computer from "./pages/Computer";
import Settings from "./pages/Settings";
import "./App.css";

const TABS = { computer: "Компьютер", settings: "Настройки" } as const;
type Tab = keyof typeof TABS;

export default function App() {
  const [tab, setTab] = useState<Tab>("computer");

  return (
    <main className="page">
      <header>
        <h1>Ollivo</h1>
        <nav>
          {(Object.keys(TABS) as Tab[]).map((t) => (
            <button key={t} className={t === tab ? "tab active" : "tab"} onClick={() => setTab(t)}>
              {TABS[t]}
            </button>
          ))}
        </nav>
      </header>
      {tab === "computer" ? <Computer /> : <Settings />}
    </main>
  );
}
