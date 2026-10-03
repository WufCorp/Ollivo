import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import {
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
import Fit, { Light } from "../components/Fit";
import License from "../components/License";
import RemoteForm from "../components/RemoteForm";
import RunningModel from "../components/RunningModel";
import { decimal, t } from "../i18n";

const EXTENSIONS = ["gguf", "safetensors", "bin", "pt", "ckpt", "pth"];

function params(n: number): string {
  if (n >= 1e9) return `${decimal(n / 1e9)}B ${t("параметров", "parameters")}`;
  return `${Math.round(n / 1e6)}M ${t("параметров", "parameters")}`;
}

/** Строка под именем файла: что это, какого семейства, сколько весит. */
function summary(m: Model): string {
  const i = m.info;
  return [m.kind_name, i.family, i.precision, i.params > 0 ? params(i.params) : "", formatBytes(m.size)]
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
    const picked = await open({ multiple: true, filters: [{ name: t("Модели", "Models"), extensions: EXTENSIONS }] });
    if (Array.isArray(picked)) await add(picked);
  };

  const remove = async (m: Model) => {
    await modelsRemove(m.path);
    await refresh();
  };

  return (
    <>
      <h2>{t("Модели", "Models")}</h2>

      <div className={over ? "dropzone over" : "dropzone"}>
        <p>
          {t(
            "Перетащите сюда файл модели — я сама разберусь, что это и пойдёт ли она на вашем компьютере.",
            "Drag a model file here — I'll figure out what it is and whether it will run on your computer.",
          )}
        </p>
        <div className="actions center">
          <button onClick={pick} disabled={busy}>
            {busy ? t("Смотрю…", "Looking…") : t("Выбрать файл", "Choose a file")}
          </button>
          <button className="secondary" onClick={() => search()} disabled={busy}>
            {t("Найти уже скачанные", "Find already downloaded")}
          </button>
          <button className="secondary" onClick={pickFolder} disabled={busy}>
            {t("Указать папку", "Choose a folder")}
          </button>
        </div>
        <p className="muted small">
          {t(
            "Подходят файлы .gguf и .safetensors — например, скачанные с HuggingFace. «Найти уже скачанные» смотрит в папках LM Studio, Ollama и ComfyUI: оттуда модели берутся как есть, второй раз качать не надо.",
            "Files .gguf and .safetensors work — for example, downloaded from HuggingFace. “Find already downloaded” looks in the LM Studio, Ollama and ComfyUI folders: models are taken from there as is, no need to download them twice.",
          )}
        </p>
      </div>

      {scanned && (
        <div className="card">
          <p>
            {scanned.added > 0
              ? t(`Нашла новых моделей: ${scanned.added}.`, `New models found: ${scanned.added}.`)
              : t("Новых моделей не нашлось.", "No new models found.")}
            {scanned.already > 0 && t(` Уже были в списке: ${scanned.already}.`, ` Already in the list: ${scanned.already}.`)}
          </p>
          {scanned.sources.length > 0 && (
            <p className="muted small">{t(`Смотрела: ${scanned.sources.join(", ")}.`, `Looked in: ${scanned.sources.join(", ")}.`)}</p>
          )}
          <div className="actions">
            <button className="secondary" onClick={() => setScanned(null)}>
              {t("Понятно", "OK")}
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
              {t("Понятно", "OK")}
            </button>
          </div>
        </div>
      )}

      <RemoteForm />

      <RunningModel onGoToChat={onGoToChat} onGo={onGo} onRemoved={refresh} />

      {models?.length === 0 && <p className="muted">{t("Пока ни одной модели.", "No models yet.")}</p>}

      {models?.map((m) => {
        const busyNow = running?.state === "starting";
        // Выгруженная после простоя в видеокарте не сидит — её можно запустить заново.
        const isRunning = running?.model === m.path && running.state !== "stopped" && running.state !== "sleeping";
        return (
          <div className="card model" key={m.path} title={m.path}>
            <p className="model-title">
              <Light light={m.missing ? "missing" : (m.verdict?.light ?? "none")} />
              {m.title ?? m.file}
            </p>
            <p className="muted small">{summary(m)}</p>

            {m.missing ? (
              <p className="error">
                {t("Файла нет на месте — его переместили или диск отключён.", "The file is missing — it was moved or the disk is disconnected.")}
              </p>
            ) : (
              <>
                <p className="variant-headline">{m.verdict?.headline}</p>
                {m.verdict && <Fit verdict={m.verdict} />}
              </>
            )}
            {m.info.notes.map((n) => (
              <p key={n} className="muted small">
                {n}
              </p>
            ))}
            {m.info.needs.length > 0 && (
              <p className="muted small">
                {t("Ещё нужно скачать:", "Also needs to be downloaded:")} {m.info.needs.join(", ")}
              </p>
            )}
            {/* Лицензия со страницы модели точнее: в файле её пишут не всегда. */}
            <License code={m.license ?? m.info.license} repo={m.repo} />

            <div className="actions">
              {m.info.kind === "llm" && m.info.engine === "llama_cpp" && !m.missing && (
                <button onClick={() => llmStart(m.path)} disabled={busyNow || isRunning}>
                  {isRunning ? t("Запущена", "Running") : t("Запустить", "Start")}
                </button>
              )}
              <button className="secondary" onClick={() => remove(m)}>
                {t("Убрать из списка", "Remove from the list")}
              </button>
            </div>
          </div>
        );
      })}
    </>
  );
}
