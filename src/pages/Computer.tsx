import { useEffect, useState } from "react";
import { formatBytes, hardwareInfo, type Hardware } from "../api";
import EngineCard from "../components/EngineCard";

export default function Computer() {
  const [hw, setHw] = useState<Hardware | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    hardwareInfo().then(setHw, (e) => setError(String(e)));
  }, []);

  return (
    <>
      <h2>Компьютер</h2>
      {error && <p className="error">{error}</p>}
      {!hw && !error && <p className="muted">Смотрю, что за компьютер…</p>}
      {hw && (
        <dl className="facts">
          <dt>Видеокарта</dt>
          <dd>
            {hw.gpu
              ? `${hw.gpu.name}, ${formatBytes(hw.gpu.vram_total)} (свободно ${formatBytes(hw.gpu.vram_free)})`
              : "NVIDIA не найдена — модели пойдут медленнее"}
          </dd>

          {hw.gpu && (
            <>
              <dt>Драйвер</dt>
              <dd>
                {hw.driver}, CUDA {Math.floor(hw.cuda_driver / 1000)}.{(hw.cuda_driver % 1000) / 10}
              </dd>
            </>
          )}

          {/* Сборку движка не пишем: `cuda_build` — какая CUDA подходит карте, а чат по умолчанию
              идёт на Vulkan. Настоящая сборка — в карточке движка ниже. */}
          <dt>Оперативная память</dt>
          <dd>
            {formatBytes(hw.ram_total)} (свободно {formatBytes(hw.ram_avail)})
          </dd>

          <dt>Диски</dt>
          <dd>
            {hw.disks.map((d) => (
              <div key={d.mount}>
                {d.mount} — свободно {formatBytes(d.free)} из {formatBytes(d.total)}
              </div>
            ))}
          </dd>
        </dl>
      )}

      <h2>Движки</h2>
      <EngineCard id="llama.cpp" />
    </>
  );
}
