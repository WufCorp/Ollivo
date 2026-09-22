import { useEffect, useState } from "react";
import { formatBytes, hardwareInfo, type Build, type Hardware } from "./api";
import "./App.css";

const BUILD_NAMES: Record<Build, string> = {
  cuda13: "CUDA 13",
  cuda12: "CUDA 12",
  vulkan: "Vulkan",
};

// Временный экран фазы 1: проверяет связку React ↔ Rust-ядро.
// Дальше его место займёт мастер первого запуска.
export default function App() {
  const [hw, setHw] = useState<Hardware | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    hardwareInfo().then(setHw, (e) => setError(String(e)));
  }, []);

  return (
    <main className="page">
      <h1>Ollivo</h1>
      <p className="muted">Проверка компьютера</p>

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

          <dt>Сборка движков</dt>
          <dd>{BUILD_NAMES[hw.cuda_build]}</dd>

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

          {hw.profile_risky && (
            <>
              <dt>Папка пользователя</dt>
              <dd>Есть русские буквы или пробел — программу поставим в отдельную папку на диске.</dd>
            </>
          )}
        </dl>
      )}
    </main>
  );
}
