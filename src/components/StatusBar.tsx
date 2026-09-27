import { useEffect, useState } from "react";
import { formatBytes, hardwareInfo, llmStatus, onLlmState, type Hardware, type LlmState } from "../api";
import { t } from "../i18n";
import { memoryPages } from "../words";

/** Память меняют и другие программы; чаще опрашивать незачем, NVML каждый раз открывается заново. */
const POLL_MS = 10_000;

const stateName = (s: LlmState["state"]) =>
  ({
    ready: t("готова", "ready"),
    starting: t("загружается", "loading"),
    sleeping: t("выгружена, пока вы не пишете", "unloaded until you write"),
    stopped: t("не запущена", "not running"),
    crashed: t("остановилась с ошибкой", "stopped with an error"),
  })[s];

const modelName = (p: string) => (p.split(/[\\/]/).pop() ?? p).replace(/\.gguf$/i, "");

/**
 * Строка внизу окна: какая модель, сколько занято видеопамяти, сколько разговора помнит.
 * Честные цифры всегда на виду, а не спрятаны в настройках.
 */
export default function StatusBar() {
  const [llm, setLlm] = useState<LlmState | null>(null);
  const [hw, setHw] = useState<Hardware | null>(null);

  useEffect(() => {
    const look = () => {
      if (!document.hidden) hardwareInfo().then(setHw, () => {});
    };
    look();
    llmStatus().then(setLlm);
    // Модель загрузилась или выгрузилась — память сразу другая, ждать опроса не надо.
    const sub = onLlmState((s) => {
      setLlm(s);
      look();
    });
    const timer = setInterval(look, POLL_MS);
    return () => {
      clearInterval(timer);
      sub.then((un) => un());
    };
  }, []);

  const gpu = hw?.gpu;
  const on = llm && llm.state !== "stopped" && llm.model;

  return (
    <footer className="status">
      <span>
        <i className={`light ${llm?.state === "ready" ? "green" : llm?.state === "crashed" ? "red" : llm?.state === "starting" ? "yellow" : "none"}`} />
        {on ? (
          <>
            <b className="status-model">{modelName(llm.model!)}</b> {stateName(llm.state)}
          </>
        ) : (
          t("Модель не запущена", "Model not running")
        )}
      </span>

      {hw && (gpu ? <Vram total={gpu.vram_total} free={gpu.vram_free} /> : <span>{t("Видеокарта NVIDIA не найдена", "No NVIDIA graphics card found")}</span>)}

      {llm?.state === "ready" && llm.ctx !== null && <span className="memory">
          {t(`помнит ${memoryPages(llm.ctx)} разговора`, `remembers ${memoryPages(llm.ctx)} of conversation`)}
        </span>}

      {hw && (
        <span className="right">
          {t("Оперативная память: свободно", "Memory: free")} <b>{formatBytes(hw.ram_avail)}</b>
        </span>
      )}
    </footer>
  );
}

/** Видеопамять кубиками: на 8 ГБ — по гигабайту, на больших картах кубик крупнее. */
function Vram({ total, free }: { total: number; free: number }) {
  const gb = total / 2 ** 30;
  const n = Math.max(4, Math.min(12, Math.round(gb)));
  const used = total - free;
  const filled = Math.round((used / total) * n);
  const tight = used / total > 0.9;
  return (
    <span title={t("Сколько видеопамяти занято сейчас: моделью и другими программами", "How much video memory is used now: by the model and other programs")}>
      {t("Видеокарта", "GPU")}
      <span className={tight ? "blocks tight" : "blocks"}>
        {Array.from({ length: n }, (_, i) => (
          <s key={i} className={i < filled ? "on" : ""} />
        ))}
      </span>
      {t("занято", "used")} <b>{formatBytes(used).replace(/ (ГБ|GB)$/, "")}</b> {t("из", "of")} {formatBytes(total)}
    </span>
  );
}
