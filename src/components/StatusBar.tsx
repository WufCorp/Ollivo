import { useEffect, useState } from "react";
import { formatBytes, hardwareInfo, llmStatus, onLlmState, type Hardware, type LlmState } from "../api";
import { memoryPages } from "../words";

/** Память меняют и другие программы; чаще опрашивать незачем, NVML каждый раз открывается заново. */
const POLL_MS = 10_000;

const STATE: Record<LlmState["state"], string> = {
  ready: "готова",
  starting: "загружается",
  sleeping: "выгружена, пока вы не пишете",
  stopped: "не запущена",
  crashed: "остановилась с ошибкой",
};

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
            <b className="status-model">{modelName(llm.model!)}</b> {STATE[llm.state]}
          </>
        ) : (
          "Модель не запущена"
        )}
      </span>

      {hw && (gpu ? <Vram total={gpu.vram_total} free={gpu.vram_free} /> : <span>Видеокарта NVIDIA не найдена</span>)}

      {llm?.state === "ready" && llm.ctx !== null && <span className="memory">помнит {memoryPages(llm.ctx)} разговора</span>}

      {hw && (
        <span className="right">
          Оперативная память: свободно <b>{formatBytes(hw.ram_avail)}</b>
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
    <span title="Сколько видеопамяти занято сейчас: моделью и другими программами">
      Видеокарта
      <span className={tight ? "blocks tight" : "blocks"}>
        {Array.from({ length: n }, (_, i) => (
          <s key={i} className={i < filled ? "on" : ""} />
        ))}
      </span>
      занято <b>{formatBytes(used).replace(" ГБ", "")}</b> из {formatBytes(total)}
    </span>
  );
}
