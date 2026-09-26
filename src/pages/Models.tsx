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
  modelsScan,
  onLlmState,
  type AddedModel,
  type LlmState,
  type Model,
  type ScanReport,
} from "../api";
import License from "../components/License";
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

export default function Models({
  onGoToChat,
  onGo,
}: {
  onGoToChat: () => void;
  onGo: (tab: "catalog" | "computer") => void;
}) {
  const [models, setModels] = useState<Model[] | null>(null);
  const [over, setOver] = useState(false);
  const [busy, setBusy] = useState(false);
  const [rejected, setRejected] = useState<AddedModel[]>([]);
  const [scanned, setScanned] = useState<ScanReport | null>(null);
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

  /** Поиск по известным местам; `dir` — ещё и папка, которую выбрал человек. */
  const search = async (dir?: string) => {
    setBusy(true);
    setRejected([]);
    try {
      setScanned(await modelsScan(dir ? [dir] : []));
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const pickFolder = async () => {
    const dir = await open({ directory: true });
    if (typeof dir === "string") await search(dir);
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
        <div className="actions center">
          <button onClick={pick} disabled={busy}>
            {busy ? "Смотрю…" : "Выбрать файл"}
          </button>
          <button className="secondary" onClick={() => search()} disabled={busy}>
            Найти уже скачанные
          </button>
          <button className="secondary" onClick={pickFolder} disabled={busy}>
            Указать папку
          </button>
        </div>
        <p className="muted small">
          Подходят файлы .gguf и .safetensors — например, скачанные с HuggingFace. «Найти уже скачанные» смотрит
          в папках LM Studio, Ollama и ComfyUI: оттуда модели берутся как есть, второй раз качать не надо.
        </p>
      </div>

      {scanned && (
        <div className="card">
          <p>
            {scanned.added > 0
              ? `Нашла новых моделей: ${scanned.added}.`
              : "Новых моделей не нашлось."}
            {scanned.already > 0 && ` Уже были в списке: ${scanned.already}.`}
          </p>
          {scanned.sources.length > 0 && <p className="muted small">Смотрела: {scanned.sources.join(", ")}.</p>}
          <div className="actions">
            <button className="secondary" onClick={() => setScanned(null)}>
              Понятно
            </button>
          </div>
        </div>
      )}

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

      <RunningModel onGoToChat={onGoToChat} onGo={onGo} onRemoved={refresh} />

      {models?.length === 0 && <p className="muted">Пока ни одной модели.</p>}

      {models?.map((m) => {
        const busyNow = running?.state === "starting";
        const isRunning = running?.model === m.path && running.state !== "stopped";
        return (
          <div className="card model" key={m.path} title={m.path}>
            <p className="model-title">
              <span className="light">{m.missing ? "⚠️" : LIGHTS[m.verdict?.light ?? "none"]}</span>
              {m.title ?? m.file}
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
            {/* Лицензия со страницы модели точнее: в файле её пишут не всегда. */}
            <License code={m.license ?? m.info.license} repo={m.repo} />

            <div className="actions">
              {m.info.kind === "llm" && m.info.engine === "llama_cpp" && !m.missing && (
                <button onClick={() => llmStart(m.path)} disabled={busyNow || isRunning}>
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
