import { useEffect, useState } from "react";
import { llmStart, llmStatus, onLlmState, shareInfo, shareNewKey, shareSet, type LlmState, type ShareInfo, type ShareSettings } from "../api";
import { copyText } from "./Answer";
import { t } from "../i18n";

/** Строка с кнопкой «Копировать»: адрес, ключ, команда. */
function Copyable({ label, value, secret }: { label: string; value: string; secret?: boolean }) {
  const [shown, setShown] = useState(!secret);
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    if (await copyText(value)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  };
  return (
    <div className="copyable">
      <span className="muted small">{label}</span>
      <code>{shown ? value : "•".repeat(24)}</code>
      <div className="actions">
        {secret && (
          <button className="link" onClick={() => setShown(!shown)}>
            {shown ? t("Скрыть", "Hide") : t("Показать", "Show")}
          </button>
        )}
        <button className="link" onClick={copy}>
          {copied ? t("Скопировано", "Copied") : t("Копировать", "Copy")}
        </button>
      </div>
    </div>
  );
}

/**
 * Модель для других программ: SillyTavern, qwen-code и всё, что говорит на языке API OpenAI.
 * Только с этого компьютера (127.0.0.1) и только с ключом. Переключатель действует сразу,
 * мимо «Сохранить»: человек тут же идёт вставлять адрес и ключ в другую программу.
 */
export default function SharePanel({ share, onChange }: { share: ShareSettings; onChange: (s: ShareSettings) => void }) {
  const [port, setPort] = useState(String(share.port));
  const [info, setInfo] = useState<ShareInfo | null>(null);
  const [llm, setLlm] = useState<LlmState | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => shareInfo().then(setInfo, (e) => setError(String(e)));

  useEffect(() => {
    llmStatus().then(setLlm);
    const sub = onLlmState((s) => {
      setLlm(s);
      if (s.state === "ready") refresh();
    });
    return () => {
      sub.then((un) => un());
    };
  }, []);

  useEffect(() => {
    if (share.enabled) refresh();
  }, [share.enabled, share.port]);

  const apply = async (next: ShareSettings) => {
    setError(null);
    try {
      await shareSet(next.enabled, next.port);
      onChange(next);
    } catch (e) {
      setError(String(e));
    }
  };

  const local = llm?.state === "ready" && !llm.remote && llm.model;
  const restart = () => llm?.model && llmStart(llm.model, { lighter: llm.lighter });

  return (
    <div className="card form">
      <label className="check">
        <input type="checkbox" checked={share.enabled} onChange={(e) => apply({ ...share, enabled: e.target.checked })} />
        {t("Открыть модель для других программ", "Open the model to other programs")}
      </label>
      <p className="muted small">
        {t(
          "SillyTavern, qwen-code и другие программы смогут разговаривать с моделью, запущенной в Ollivo. Только с этого компьютера и только с ключом. Пока доступ открыт, модель не выгружается при простое: чужих вопросов Ollivo не видит.",
          "SillyTavern, qwen-code and other programs will be able to talk to the model running in Ollivo. Only from this computer and only with the key. While access is open, the model isn't unloaded when idle: Ollivo doesn't see other programs' questions.",
        )}
      </p>

      {share.enabled && (
        <>
          <label>
            {t("Порт", "Port")}
            <input
              inputMode="numeric"
              value={port}
              onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
              onBlur={() => Number(port) !== share.port && apply({ ...share, port: Number(port) })}
            />
          </label>
          {info && (
            <>
              <Copyable label={t("Адрес", "Address")} value={info.url} />
              <Copyable label={t("Ключ", "Key")} value={info.key} secret />
              <div className="actions">
                <button className="link" onClick={() => shareNewKey().then(setInfo, (e) => setError(String(e)))}>
                  {t("Новый ключ", "New key")}
                </button>
              </div>
              {local && !info.live && (
                <div className="notice">
                  <p className="small">
                    {t(
                      "Запущенная модель ещё на старом адресе — программы её не увидят, пока она не перезапустится.",
                      "The running model is still on the old address — programs won't see it until it restarts.",
                    )}
                  </p>
                  <button className="secondary" onClick={restart}>
                    {t("Перезапустить модель", "Restart the model")}
                  </button>
                </div>
              )}
              {!local && (
                <p className="muted small">
                  {t(
                    "Адрес заработает, когда модель будет запущена в Ollivo — на странице «Модели».",
                    "The address will work once a model is running in Ollivo — on the “Models” page.",
                  )}
                </p>
              )}

              <details className="guide">
                <summary>{t("Как подключить: SillyTavern, qwen-code, другие", "How to connect: SillyTavern, qwen-code, others")}</summary>
                <h4>SillyTavern</h4>
                <ol className="small">
                  <li>{t("Вкладка «API Connections» (значок вилки).", "The “API Connections” tab (the plug icon).")}</li>
                  <li>{t("API — «Chat Completion», источник — «Custom (OpenAI-compatible)».", "API — “Chat Completion”, source — “Custom (OpenAI-compatible)”.")}</li>
                  <li>{t("«Custom Endpoint» — адрес выше, «Custom API Key» — ключ.", "“Custom Endpoint” — the address above, “Custom API Key” — the key.")}</li>
                  <li>{t("«Connect». Модель в списке будет одна — та, что запущена в Ollivo.", "“Connect”. There will be one model in the list — the one running in Ollivo.")}</li>
                </ol>
                <h4>qwen-code</h4>
                <p className="small">
                  {t(
                    "В PowerShell, в папке проекта, — эта строка запускает qwen-code с моделью из Ollivo:",
                    "In PowerShell, in the project folder — this line starts qwen-code with the model from Ollivo:",
                  )}
                </p>
                <Copyable
                  label="PowerShell"
                  value={`$env:OPENAI_BASE_URL="${info.url}"; $env:OPENAI_API_KEY="${info.key}"; $env:OPENAI_MODEL="ollivo"; qwen`}
                  secret
                />
                <p className="muted small">
                  {t(
                    "qwen-code сам открывает и правит файлы — ему нужна модель, которая это умеет: в Ollivo такие работают с папкой проекта. Маленькие модели (до 7B) справляются плохо.",
                    "qwen-code opens and edits files by itself — it needs a model that can do that: in Ollivo such models work with a project folder. Small models (under 7B) do poorly.",
                  )}
                </p>
                <h4>{t("Другие программы", "Other programs")}</h4>
                <p className="small">
                  {t(
                    "Continue, Open WebUI, Python-библиотека openai и всё, где есть «OpenAI-совместимый» сервер: адрес — в поле «Base URL», ключ — в «API Key», имя модели — любое, например ollivo.",
                    "Continue, Open WebUI, the openai Python library and anything with an “OpenAI-compatible” server: the address goes into “Base URL”, the key into “API Key”, the model name can be anything, for example ollivo.",
                  )}
                </p>
              </details>
            </>
          )}
        </>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}
