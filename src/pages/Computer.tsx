import { useEffect, useState } from "react";
import {
  BUILD_NAMES,
  engineInstall,
  engineStatus,
  formatBytes,
  hardwareInfo,
  onEngineFinished,
  onEngineProgress,
  taskPause,
  type EngineProgress,
  type EngineStatus,
  type Hardware,
} from "../api";

const CHAT_ENGINE = "llama.cpp";

const STAGE_NAMES: Record<EngineProgress["stage"], string> = {
  download: "Скачиваю",
  verify: "Проверяю файл",
  unpack: "Распаковываю",
};

// Временный экран фазы 1: проверка ПК и установка движка чата.
// Дальше его место займёт мастер первого запуска.
export default function Computer() {
  const [hw, setHw] = useState<Hardware | null>(null);
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [progress, setProgress] = useState<EngineProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => engineStatus(CHAT_ENGINE).then(setEngine, (e) => setError(String(e)));

  useEffect(() => {
    hardwareInfo().then(setHw, (e) => setError(String(e)));
    refresh();
    const subs = [
      onEngineProgress((p) => p.id === CHAT_ENGINE && setProgress(p)),
      onEngineFinished((f) => {
        if (f.id !== CHAT_ENGINE) return;
        setProgress(null);
        setError(f.error && f.error !== "paused" ? f.error : null);
        refresh();
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  const install = () => {
    setError(null);
    setProgress({ id: CHAT_ENGINE, stage: "download", done: 0, total: engine?.size ?? 0, speed: 0 });
    engineInstall(CHAT_ENGINE).catch((e) => {
      setProgress(null);
      setError(String(e));
    });
  };

  const installed = engine?.installed.find((i) => i.version === engine.version);

  return (
    <>
      <h2>Компьютер</h2>
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

      <h2>Движок чата</h2>
      {engine && (
        <div className="card">
          {installed ? (
            <p>
              ✓ Установлен: llama.cpp {installed.version}, {BUILD_NAMES[installed.build]}
            </p>
          ) : progress ? (
            <>
              <p>
                {STAGE_NAMES[progress.stage]}… {formatBytes(progress.done)} из {formatBytes(progress.total)}
                {progress.stage === "download" && progress.speed > 0 && `, ${formatBytes(progress.speed)}/с`}
              </p>
              <progress max={progress.total || 1} value={progress.done} />
              {progress.stage === "download" && (
                <button className="secondary" onClick={() => taskPause(`engine:${CHAT_ENGINE}`)}>
                  Пауза
                </button>
              )}
            </>
          ) : (
            <>
              <p className="muted">
                llama.cpp {engine.version}
                {engine.build && `, сборка ${BUILD_NAMES[engine.build]}, ${formatBytes(engine.size)}`}
              </p>
              <button onClick={install} disabled={!engine.build}>
                Установить
              </button>
            </>
          )}
        </div>
      )}
      {error && <p className="error">{error}</p>}
    </>
  );
}
