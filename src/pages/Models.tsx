import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import {
  LIGHTS,
  formatBytes,
  llmStart,
  llmStatus,
  modelsAdd,
  modelsList,
  modelsRemove,
  onLlmState,
  type AddedModel,
  type LlmState,
  type Model,
} from "../api";
import RunningModel from "../components/RunningModel";

const EXTENSIONS = ["gguf", "safetensors", "bin", "pt", "ckpt", "pth"];

function params(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1).replace(".", ",")}B параметров`;
  return `${Math.round(n / 1e6)}M параметров`;
}

/** Строка под именем файла: что это, какого семейства, сколько весит. */
function summary(m: Model): string {
  const i = m.info;
  return [m.kind_ru, i.family, i.precision, i.params > 0 ? params(i.params) : "", formatBytes(m.size)]
    .filter(Boolean)
    .join(" · ");
}

export default function Models() {
  const [models, setModels] = useState<Model[] | null>(null);
  const [over, setOver] = useState(false);
  const [busy, setBusy] = useState(false);
  const [rejected, setRejected] = useState<AddedModel[]>([]);
  const [running, setRunning] = useState<LlmState | null>(null);

  const refresh = () => modelsList().then(setModels);

  useEffect(() => {
    refresh();
    llmStatus().then(setRunning);
    const llm = onLlmState(setRunning);
    // Перетаскивание файла в окно — то же, что «Выбрать файл».
    const drop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "over") setOver(true);
      else if (e.payload.type === "drop") {
        setOver(false);
        add(e.payload.paths);
      } else setOver(false);
    });
    return () => {
      llm.then((un) => un());
      drop.then((un) => un());
    };
  }, []);

  const add = async (paths: string[]) => {
    if (!paths.length) return;
    setBusy(true);
    try {
      const report = await modelsAdd(paths);
      setRejected(report.filter((r) => r.error));
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const pick = async () => {
    const picked = await open({ multiple: true, filters: [{ name: "Модели", extensions: EXTENSIONS }] });
    if (Array.isArray(picked)) await add(picked);
  };

  const remove = async (m: Model) => {
    await modelsRemove(m.path);
    await refresh();
  };

  return (
    <>
      <h2>Модели</h2>

      <div className={over ? "dropzone over" : "dropzone"}>
        <p>Перетащите сюда файл модели — я сама разберусь, что это и пойдёт ли она на вашем компьютере.</p>
        <button onClick={pick} disabled={busy}>
          {busy ? "Смотрю файл…" : "Выбрать файл"}
        </button>
        <p className="muted small">Подходят файлы .gguf и .safetensors — например, скачанные с HuggingFace.</p>
      </div>

      {rejected.length > 0 && (
        <div className="card">
          {rejected.map((r) => (
            <p key={r.file} className="error">
              {r.file}: {r.error}
            </p>
          ))}
          <div className="actions">
            <button className="secondary" onClick={() => setRejected([])}>
              Понятно
            </button>
          </div>
        </div>
      )}

      <RunningModel />

      {models?.length === 0 && <p className="muted">Пока ни одной модели.</p>}

      {models?.map((m) => {
        const busyNow = running?.state === "starting";
        const isRunning = running?.model === m.path && running.state !== "stopped";
        return (
          <div className="card model" key={m.path}>
            <p className="model-title">
              <span className="light">{m.missing ? "⚠️" : LIGHTS[m.verdict?.light ?? "none"]}</span>
              {m.file}
            </p>
            <p className="muted small">{summary(m)}</p>

            {m.missing ? (
              <p className="error">Файла нет на месте — его переместили или диск отключён.</p>
            ) : (
              <>
                <p>{m.verdict?.headline}</p>
                {m.verdict?.details.map((d) => (
                  <p key={d} className="muted small">
                    {d}
                  </p>
                ))}
              </>
            )}
            {m.info.notes.map((n) => (
              <p key={n} className="muted small">
                {n}
              </p>
            ))}
            {m.info.needs.length > 0 && <p className="muted small">Ещё нужно скачать: {m.info.needs.join(", ")}</p>}
            {m.info.license && <p className="muted small">Лицензия: {m.info.license}</p>}

            <div className="actions">
              {m.info.kind === "llm" && m.info.engine === "llama_cpp" && !m.missing && (
                <button onClick={() => llmStart(m.path, m.verdict?.ctx ?? undefined)} disabled={busyNow || isRunning}>
                  {isRunning ? "Запущена" : "Запустить"}
                </button>
              )}
              <button className="secondary" onClick={() => remove(m)}>
                Убрать из списка
              </button>
            </div>
          </div>
        );
      })}
    </>
  );
}
