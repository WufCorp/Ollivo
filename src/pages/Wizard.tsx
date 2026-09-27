import { useEffect, useState } from "react";
import {
  formatBytes,
  languageSet,
  proxyTest,
  settingsGet,
  settingsSave,
  setupCheck,
  setupChooseDir,
  setupFinish,
  type CheckStatus,
  type ProxyReport,
  type Settings,
  type SetupInfo,
} from "../api";
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";
import InstallScreen, { type InstallError } from "../components/InstallScreen";
import { useEngine } from "../useEngine";
import ProxyForm, { CheckList } from "../components/ProxyForm";
import { getLang, setLang, t, type Lang } from "../i18n";

const steps = () => [t("Проверка", "Check"), t("Папка", "Folder"), t("Сеть", "Network"), t("Движок", "Engine")];
const ICONS: Record<CheckStatus, string> = { ok: "✓", warn: "!" };

/**
 * Язык — первым делом и на виду: ядро угадало его по Windows, но угадать могло неверно.
 * Названия языков — каждый на своём языке, чтобы найти свой, не понимая остального.
 */
function LangSwitch() {
  const change = async (l: Lang) => {
    await languageSet(l);
    setLang(l);
  };
  return (
    <div className="wizard-lang">
      <select aria-label="Язык · Language" value={getLang()} onChange={(e) => change(e.target.value as Lang)}>
        <option value="ru">Русский</option>
        <option value="en">English</option>
      </select>
    </div>
  );
}

/** Мастер первого запуска: проверка ПК → папка → сеть → движок чата → «Всё готово». */
export default function Wizard({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [done, setDone] = useState(false);

  if (done) {
    return (
      <div className="wizard">
        <h2>{t("Всё готово", "All set")}</h2>
        <p>
          {t(
            "Движок чата установлен. Следующий шаг — выбрать модель, с которой будем разговаривать.",
            "The chat engine is installed. Next step — pick a model to talk to.",
          )}
        </p>
        <button
          onClick={async () => {
            await setupFinish();
            onDone();
          }}
        >
          {t("Начать", "Start")}
        </button>
      </div>
    );
  }

  return (
    <div className="wizard">
      <LangSwitch />
      <ol className="steps">
        {steps().map((s, i) => (
          <li key={i} className={i === step ? "active" : i < step ? "passed" : ""}>
            {s}
          </li>
        ))}
      </ol>
      {/* Сменили язык — проверки и сеть спросим у ядра заново: их тексты пишет оно. */}
      {step === 0 && <CheckStep key={getLang()} next={() => setStep(1)} />}
      {step === 1 && <DiskStep next={() => setStep(2)} />}
      {step === 2 && <NetStep key={getLang()} next={() => setStep(3)} />}
      {step === 3 && <EngineStep next={() => setDone(true)} />}
    </div>
  );
}

function CheckStep({ next }: { next: () => void }) {
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = () => setupCheck().then(setInfo, (e) => setError(String(e)));
  useEffect(() => {
    load();
  }, []);

  if (!info) return <p className="muted">{t("Смотрю, что за компьютер…", "Looking at the computer…")}</p>;

  return (
    <>
      <h2>{t("Проверка компьютера", "Computer check")}</h2>
      <ul className="setup-checks">
        {info.checks.map((c) => (
          <li key={c.id} className={c.status}>
            <span className="icon">{ICONS[c.status]}</span>
            <div>
              <b>{c.title}</b> — {c.message}
            </div>
          </li>
        ))}
      </ul>
      {error && <p className="error">{error}</p>}
      <div className="actions">
        <button onClick={next}>{t("Дальше", "Next")}</button>
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

  if (!info) return <p className="muted">{t("Смотрю диски…", "Looking at the disks…")}</p>;

  return (
    <>
      <h2>{t("Где хранить модели", "Where to keep models")}</h2>
      <p className="muted">
        {t(
          "Одна модель занимает от 1 до 10 ГБ, поэтому лучше выбрать диск, где много места.",
          "One model takes 1 to 10 GB, so it's better to choose a disk with plenty of space.",
        )}
      </p>
      <div className="disks">
        {info.disks.map((d) => (
          <label key={d.path} className={`disk ${path === d.path ? "selected" : ""}`}>
            <input type="radio" name="disk" checked={path === d.path} onChange={() => setPath(d.path)} />
            <div>
              <b>{d.path}</b>
              {d.recommended && <span className="badge">{t("советуем", "recommended")}</span>}
              <div className="muted small">
                {t("свободно", "free")} {formatBytes(d.free)} {t("из", "of")} {formatBytes(d.total)}
                {!d.enough && t(" — мало места", " — low on space")}
              </div>
            </div>
          </label>
        ))}
      </div>
      {error && <p className="error">{error}</p>}
      <div className="actions">
        <button onClick={choose} disabled={!path}>
          {t("Дальше", "Next")}
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

  if (!direct || !settings) return <p className="muted">{t("Проверяю, открываются ли сайты с моделями…", "Checking whether the model sites open…")}</p>;

  if (direct.ok && !settings.proxy.enabled) {
    return (
      <>
        <h2>{t("Интернет", "Internet")}</h2>
        <CheckList checks={direct.checks} />
        <p className="muted">{t("Сайты с моделями открываются напрямую, прокси не нужен.", "The model sites open directly, no proxy needed.")}</p>
        <div className="actions">
          <button onClick={next}>{t("Дальше", "Next")}</button>
        </div>
      </>
    );
  }

  return (
    <>
      <h2>{t("Интернет", "Internet")}</h2>
      {!direct.ok && (
        <>
          <CheckList checks={direct.checks} />
          <p>
            {t(
              "Напрямую сайты с моделями не открываются — возможно, из-за ограничений в вашем регионе. Если у вас есть прокси, укажите его и нажмите «Проверить».",
              "The model sites don't open directly — perhaps because of restrictions in your region. If you have a proxy, enter it and click “Check”.",
            )}
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
          {t("Дальше", "Next")}
        </button>
        {!proxyOk && (
          <button className="secondary" onClick={next}>
            {t("Пропустить", "Skip")}
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
    const timer = setInterval(() => setRetryIn((s) => (s == null ? s : s - 1)), 1000);
    const online = () => setRetryIn(0);
    window.addEventListener("online", online);
    return () => {
      clearInterval(timer);
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

  if (!status) return <p className="muted">{t("Смотрю, что уже установлено…", "Checking what is already installed…")}</p>;

  const err: InstallError | null = error ? { text: error, kind: errorKind, retryIn } : null;

  return (
    <>
      <h2>{t("Движок чата", "Chat engine")}</h2>
      <p className="muted">{t("Программа, которая запускает текстовые модели. Скачается один раз.", "The program that runs text models. It downloads once.")}</p>
      <InstallScreen
        items={[
          {
            id: "llama.cpp",
            title: t("Движок чата", "Chat engine"),
            why: t("запускает текстовые модели на вашей видеокарте", "runs text models on your graphics card"),
            technical: `llama.cpp ${status.version}${status.build ? `, ${t("сборка", "build")} ${status.build}` : ""}`,
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
      {!status.build && (
        <p className="error">
          {t(
            "Для этого компьютера нет подходящей сборки: нужна видеокарта NVIDIA или Vulkan.",
            "There is no suitable build for this computer: an NVIDIA graphics card or Vulkan is required.",
          )}
        </p>
      )}
      <div className="actions">
        <button onClick={next} disabled={!installed}>
          {t("Дальше", "Next")}
        </button>
      </div>
    </>
  );
}
