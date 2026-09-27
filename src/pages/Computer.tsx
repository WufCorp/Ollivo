import { useEffect, useState } from "react";
import { formatBytes, hardwareInfo, type Hardware } from "../api";
import EngineCard from "../components/EngineCard";
import { t } from "../i18n";

export default function Computer() {
  const [hw, setHw] = useState<Hardware | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    hardwareInfo().then(setHw, (e) => setError(String(e)));
  }, []);

  return (
    <>
      <h2>{t("Компьютер", "Computer")}</h2>
      {error && <p className="error">{error}</p>}
      {!hw && !error && <p className="muted">{t("Смотрю, что за компьютер…", "Looking at the computer…")}</p>}
      {hw && (
        <dl className="facts">
          <dt>{t("Видеокарта", "Graphics card")}</dt>
          <dd>
            {hw.gpu
              ? `${hw.gpu.name}, ${formatBytes(hw.gpu.vram_total)} (${t("свободно", "free")} ${formatBytes(hw.gpu.vram_free)})`
              : t("NVIDIA не найдена — модели пойдут медленнее", "No NVIDIA card found — models will run slower")}
          </dd>

          {hw.gpu && (
            <>
              <dt>{t("Драйвер", "Driver")}</dt>
              <dd>
                {hw.driver}, CUDA {Math.floor(hw.cuda_driver / 1000)}.{(hw.cuda_driver % 1000) / 10}
              </dd>
            </>
          )}

          {/* Сборку движка не пишем: `cuda_build` — какая CUDA подходит карте, а чат по умолчанию
              идёт на Vulkan. Настоящая сборка — в карточке движка ниже. */}
          <dt>{t("Оперативная память", "Memory (RAM)")}</dt>
          <dd>
            {formatBytes(hw.ram_total)} ({t("свободно", "free")} {formatBytes(hw.ram_avail)})
          </dd>

          <dt>{t("Диски", "Disks")}</dt>
          <dd>
            {hw.disks.map((d) => (
              <div key={d.mount}>
                {d.mount} — {t("свободно", "free")} {formatBytes(d.free)} {t("из", "of")} {formatBytes(d.total)}
              </div>
            ))}
          </dd>
        </dl>
      )}

      <h2>{t("Движки", "Engines")}</h2>
      <EngineCard id="llama.cpp" />
    </>
  );
}
