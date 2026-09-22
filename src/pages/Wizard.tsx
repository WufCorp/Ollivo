import { useEffect, useState } from "react";
import {
  formatBytes,
  proxyTest,
  settingsGet,
  settingsSave,
  setupCheck,
  setupChooseDir,
  setupFinish,
  vcredistInstall,
  type CheckStatus,
  type ProxyReport,
  type Settings,
  type SetupInfo,
} from "../api";
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";
import InstallScreen, { type InstallError } from "../components/InstallScreen";
import { useEngine } from "../useEngine";
import ProxyForm, { CheckList } from "../components/ProxyForm";

const STEPS = ["Проверка", "Папка", "Сеть", "Движок"] as const;
const ICONS: Record<CheckStatus, string> = { ok: "✓", warn: "!", fail: "✗" };

/** Мастер первого запуска: проверка ПК → папка → сеть → движок чата → «Всё готово». */
export default function Wizard({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [done, setDone] = useState(false);

  if (done) {
    return (
      <div className="wizard">
        <h2>Всё готово</h2>
        <p>Движок чата установлен. Следующий шаг — выбрать модель, с которой будем разговаривать.</p>
        <button
          onClick={async () => {
            await setupFinish();
            onDone();
          }}
        >
          Начать
        </button>
      </div>
    );
  }

  return (
    <div className="wizard">
      <ol className="steps">
        {STEPS.map((s, i) => (
          <li key={s} className={i === step ? "active" : i < step ? "passed" : ""}>
            {s}
          </li>
        ))}
      </ol>
      {step === 0 && <CheckStep next={() => setStep(1)} />}
      {step === 1 && <DiskStep next={() => setStep(2)} />}
      {step === 2 && <NetStep next={() => setStep(3)} />}
      {step === 3 && <EngineStep next={() => setDone(true)} />}
    </div>
  );
}

function CheckStep({ next }: { next: () => void }) {
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [fixing, setFixing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = () => setupCheck().then(setInfo, (e) => setError(String(e)));
  useEffect(() => {
    load();
  }, []);

  const fixVc = async () => {
    setFixing(true);
    setError(null);
    try {
      await vcredistInstall();
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setFixing(false);
    }
  };

  if (!info) return <p className="muted">Смотрю, что за компьютер…</p>;
  const blocked = info.checks.some((c) => c.status === "fail");

  return (
    <>
      <h2>Проверка компьютера</h2>
      <ul className="setup-checks">
        {info.checks.map((c) => (
          <li key={c.id} className={c.status}>
            <span className="icon">{ICONS[c.status]}</span>
            <div>
              <b>{c.title}</b> — {c.message}
              {c.fix === "vcredist" && (
                <div>
                  <button onClick={fixVc} disabled={fixing}>
                    {fixing ? "Устанавливаю… подтвердите запрос Windows" : "Установить"}
                  </button>
                </div>
              )}
            </div>
          </li>
        ))}
      </ul>
      {error && <p className="error">{error}</p>}
      <div className="actions">
        <button onClick={next} disabled={blocked}>
          Дальше
        </button>
      </div>
    </>
  );
}

function DiskStep({ next }: { next: () => void }) {
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setupCheck().then((i) => {
      setInfo(i);
      setPath(i.disks.find((d) => d.recommended)?.path ?? i.disks[0]?.path ?? null);
    });
  }, []);

  const choose = async () => {
    if (!path) return;
    setError(null);
    try {
      await setupChooseDir(path);
      next();
    } catch (e) {
      setError(String(e));
    }
  };

  if (!info) return <p className="muted">Смотрю диски…</p>;

  return (
    <>
      <h2>Где хранить модели</h2>
      <p className="muted">Одна модель занимает от 1 до 10 ГБ, поэтому лучше выбрать диск, где много места.</p>
      <div className="disks">
        {info.disks.map((d) => (
          <label key={d.path} className={`disk ${path === d.path ? "selected" : ""}`}>
            <input type="radio" name="disk" checked={path === d.path} onChange={() => setPath(d.path)} />
            <div>
              <b>{d.path}</b>
              {d.recommended && <span className="badge">советуем</span>}
              <div className="muted small">
                свободно {formatBytes(d.free)} из {formatBytes(d.total)}
                {!d.enough && " — мало места"}
              </div>
            </div>
          </label>
        ))}
      </div>
      {error && <p className="error">{error}</p>}
      <div className="actions">
        <button onClick={choose} disabled={!path}>
          Дальше
        </button>
      </div>
    </>
  );
}

function NetStep({ next }: { next: () => void }) {
  const [direct, setDirect] = useState<ProxyReport | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [hasPassword, setHasPassword] = useState(false);
  const [password, setPassword] = useState<string | undefined>(undefined);
  const [proxyOk, setProxyOk] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    settingsGet().then((v) => {
      setSettings(v.settings);
      setHasPassword(v.proxy_has_password);
      // Сначала — напрямую, без прокси.
      proxyTest({ ...v.settings.proxy, enabled: false }).then(setDirect);
    });
  }, []);

  const saveAndNext = async () => {
    if (!settings) return;
    setError(null);
    try {
      await settingsSave(settings, { proxyPassword: settings.proxy.auth ? password : "" });
      next();
    } catch (e) {
      setError(String(e));
    }
  };

  if (!direct || !settings) return <p className="muted">Проверяю, открываются ли сайты с моделями…</p>;

  if (direct.ok && !settings.proxy.enabled) {
    return (
      <>
        <h2>Интернет</h2>
        <CheckList checks={direct.checks} />
        <p className="muted">Сайты с моделями открываются напрямую, прокси не нужен.</p>
        <div className="actions">
          <button onClick={next}>Дальше</button>
        </div>
      </>
    );
  }

  return (
    <>
      <h2>Интернет</h2>
      {!direct.ok && (
        <>
          <CheckList checks={direct.checks} />
          <p>
            Напрямую сайты с моделями не открываются — возможно, из-за ограничений в вашем регионе. Если у вас есть
            прокси, укажите его и нажмите «Проверить».
          </p>
        </>
      )}
      <div className="card form">
        <ProxyForm
          proxy={settings.proxy}
          onChange={(proxy) => {
            setSettings({ ...settings, proxy });
            setProxyOk(false);
          }}
          password={password}
          onPassword={setPassword}
          hasPassword={hasPassword}
          onReport={(r) => setProxyOk(r.ok)}
        />
      </div>
      {error && <p className="error">{error}</p>}
      <div className="actions">
        <button onClick={saveAndNext} disabled={settings.proxy.enabled && !proxyOk}>
          Дальше
        </button>
        {!proxyOk && (
          <button className="secondary" onClick={next}>
            Пропустить
          </button>
        )}
      </div>
    </>
  );
}

/** Через сколько секунд сами повторяем загрузку, если пропал интернет. */
const NET_RETRY = 15;

function EngineStep({ next }: { next: () => void }) {
  const { status, installed, progress, last, paused, error, errorKind, install, pause } = useEngine("llama.cpp");
  const [started, setStarted] = useState(false);
  const [retryIn, setRetryIn] = useState<number | null>(null);

  const start = () => {
    setStarted(true);
    setRetryIn(null);
    install();
  };

  // Пропал интернет — повторяем сами (загрузка продолжится с того же места).
  useEffect(() => {
    if (errorKind !== "net" || !error) return setRetryIn(null);
    setRetryIn(NET_RETRY);
    const t = setInterval(() => setRetryIn((s) => (s == null ? s : s - 1)), 1000);
    const online = () => setRetryIn(0);
    window.addEventListener("online", online);
    return () => {
      clearInterval(t);
      window.removeEventListener("online", online);
    };
  }, [error, errorKind]);
  useEffect(() => {
    if (retryIn !== null && retryIn <= 0) start();
  }, [retryIn]);

  // Прогресс на кнопке в панели задач Windows — видно, даже если окно свёрнуто.
  useEffect(() => {
    const win = getCurrentWindow();
    const total = progress?.total ?? 0;
    const bar = installed
      ? { status: ProgressBarStatus.None }
      : error
        ? { status: ProgressBarStatus.Error, progress: 100 }
        : paused
          ? { status: ProgressBarStatus.Paused, progress: total ? Math.round((progress!.done / total) * 100) : 0 }
          : progress && total && progress.stage === "download"
            ? { status: ProgressBarStatus.Normal, progress: Math.round((progress.done / total) * 100) }
            : progress
              ? { status: ProgressBarStatus.Indeterminate }
              : { status: ProgressBarStatus.None };
    win.setProgressBar(bar).catch(() => {});
  }, [progress, paused, error, installed]);

  if (!status) return <p className="muted">Смотрю, что уже установлено…</p>;

  const err: InstallError | null = error ? { text: error, kind: errorKind, retryIn } : null;

  return (
    <>
      <h2>Движок чата</h2>
      <p className="muted">Программа, которая запускает текстовые модели. Скачается один раз.</p>
      <InstallScreen
        items={[
          {
            id: "llama.cpp",
            title: "Движок чата",
            why: "запускает текстовые модели на вашей видеокарте",
            technical: `llama.cpp ${status.version}${status.build ? `, сборка ${status.build}` : ""}`,
            size: status.size,
            state: installed ? "done" : error ? "error" : progress ? "active" : "waiting",
            // На паузе и после ошибки — последний прогресс: сколько уже скачано.
            progress: progress ?? last,
          },
        ]}
        started={started || !!progress}
        paused={paused}
        error={err}
        onStart={start}
        onPause={pause}
      />
      {!status.build && <p className="error">Для этого компьютера нет подходящей сборки: нужна видеокарта NVIDIA или Vulkan.</p>}
      <div className="actions">
        <button onClick={next} disabled={!installed}>
          Дальше
        </button>
      </div>
    </>
  );
}
