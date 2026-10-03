import { useEffect, useState } from "react";
import { remoteConnect, remoteGet, remoteModels, type RemoteView } from "../api";
import { t } from "../i18n";

/**
 * Модель на другом компьютере или сервере: Ollivo — только окно к ней. Подходит всё,
 * что говорит на языке API OpenAI: Ollama, LM Studio, llama-server, vLLM, облака.
 * Спрятано за кнопкой: новичку с одной видеокартой это ни к чему.
 */
export default function RemoteForm() {
  const [saved, setSaved] = useState<RemoteView | null>(null);
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  // undefined — ключ не трогали, остаётся сохранённый.
  const [key, setKey] = useState<string | undefined>(undefined);
  const [models, setModels] = useState<string[] | null>(null);
  const [model, setModel] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  useEffect(() => {
    remoteGet().then((r) => {
      setSaved(r);
      setUrl(r.url);
      setModel(r.model);
    });
  }, []);

  const check = async () => {
    setBusy(true);
    setError(null);
    setDone(false);
    try {
      const found = await remoteModels(url, key);
      setUrl(found.url);
      setModels(found.models);
      if (!found.models.includes(model)) setModel(found.models[0]);
    } catch (e) {
      setModels(null);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const connect = async () => {
    setBusy(true);
    setError(null);
    try {
      await remoteConnect(url, model, key);
      setDone(true);
      setSaved({ url, model, has_key: key === undefined ? !!saved?.has_key : key.trim() !== "" });
      setKey(undefined);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <div className="actions">
        <button className="link" onClick={() => setOpen(true)}>
          {t("Модель на другом компьютере или сервере…", "A model on another computer or server…")}
        </button>
      </div>
    );
  }

  return (
    <div className="card form">
      <p>
        {t(
          "Ollivo может разговаривать с моделью, запущенной в другом месте: в Ollama или LM Studio на соседнем компьютере, на своём сервере или в облаке. Считает тогда тот компьютер, а этот не нагружается.",
          "Ollivo can talk to a model running elsewhere: in Ollama or LM Studio on another computer, on your own server or in the cloud. That computer does the work, and this one isn't loaded.",
        )}
      </p>
      <label>
        {t("Адрес", "Address")}
        <input
          value={url}
          placeholder="http://192.168.1.5:11434/v1"
          onChange={(e) => {
            setUrl(e.target.value);
            setModels(null);
          }}
        />
      </label>
      <label>
        {t("Ключ, если сервер его просит", "Key, if the server asks for one")}
        <input
          type="password"
          value={key ?? ""}
          placeholder={saved?.has_key && key === undefined ? t("сохранён", "saved") : ""}
          onChange={(e) => {
            setKey(e.target.value);
            setModels(null);
          }}
        />
      </label>
      <p className="muted small">
        {t(
          "Ollama — адрес вида http://компьютер:11434, LM Studio — http://компьютер:1234. Сервер должен принимать подключения не только от себя: в Ollama — переменная OLLAMA_HOST=0.0.0.0, в LM Studio — «Serve on Local Network». Ключ хранится в диспетчере учётных данных Windows.",
          "Ollama — an address like http://computer:11434, LM Studio — http://computer:1234. The server must accept connections not only from itself: in Ollama — the OLLAMA_HOST=0.0.0.0 variable, in LM Studio — “Serve on Local Network”. The key is kept in the Windows Credential Manager.",
        )}
      </p>
      {models && (
        <label>
          {t("Модель", "Model")}
          <select value={model} onChange={(e) => setModel(e.target.value)}>
            {models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="actions">
        {models ? (
          <button onClick={connect} disabled={busy || !model}>
            {busy ? t("Подключаю…", "Connecting…") : t("Подключить", "Connect")}
          </button>
        ) : (
          <button onClick={check} disabled={busy || !url.trim()}>
            {busy ? t("Проверяю…", "Checking…") : t("Проверить", "Check")}
          </button>
        )}
        <button className="secondary" onClick={() => setOpen(false)}>
          {t("Свернуть", "Collapse")}
        </button>
      </div>
      {done && (
        <p className="ok">
          ✓ {t("Подключено — можно разговаривать в чате.", "Connected — you can talk in the chat.")}
        </p>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}
