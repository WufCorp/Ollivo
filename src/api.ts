// Типы и вызовы Rust-ядра. Поля совпадают с serde-структурами в src-tauri/src.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Build = "cuda13" | "cuda12" | "vulkan";

export interface Gpu {
  name: string;
  vram_total: number;
  vram_free: number;
  cc: [number, number];
  vram_bw: number;
}

export interface Disk {
  mount: string;
  total: number;
  free: number;
}

export interface Hardware {
  gpu: Gpu | null;
  driver: string;
  cuda_driver: number;
  cuda_build: Build;
  ram_total: number;
  ram_avail: number;
  disks: Disk[];
  profile_risky: boolean;
}

export interface DownloadRequest {
  urls: string[];
  dest: string;
  sha256?: string;
  connections?: number;
  chunk_size?: number;
}

export interface DownloadProgress {
  id: string;
  phase: "downloading" | "verifying";
  done: number;
  total: number | null;
  speed: number;
}

export interface DownloadFinished {
  id: string;
  /** null — успешно, "paused" — на паузе, иначе текст ошибки. */
  error: string | null;
}

export const hardwareInfo = () => invoke<Hardware>("hardware_info");

export const downloadStart = (id: string, request: DownloadRequest) =>
  invoke<void>("download_start", { id, request });

/** Пауза загрузки или установки (`engine:<id>`). */
export const taskPause = (id: string) => invoke<void>("task_pause", { id });

export const onDownloadProgress = (cb: (p: DownloadProgress) => void): Promise<UnlistenFn> =>
  listen<DownloadProgress>("download://progress", (e) => cb(e.payload));

export const onDownloadFinished = (cb: (f: DownloadFinished) => void): Promise<UnlistenFn> =>
  listen<DownloadFinished>("download://finished", (e) => cb(e.payload));

// --- Настройки и прокси ---

export type ProxyKind = "http" | "socks5";

export interface ProxySettings {
  enabled: boolean;
  kind: ProxyKind;
  host: string;
  port: number;
  auth: boolean;
  username: string;
}

export type HfSource = "official" | "mirror" | "custom";

export interface HfSettings {
  source: HfSource;
  custom_url: string;
}

export interface UpdateSettings {
  auto_check: boolean;
  channel: "stable" | "beta";
}

export interface Settings {
  data_dir: string | null;
  proxy: ProxySettings;
  hf: HfSettings;
  setup_done: boolean;
  updates: UpdateSettings;
}

export interface SettingsView {
  settings: Settings;
  /** Пароль прокси и токен HF сохранены в диспетчере учётных данных Windows. */
  proxy_has_password: boolean;
  hf_has_token: boolean;
  /** Папка данных: выбранная или предложенная по умолчанию. */
  data_dir: string;
}

export interface ProxyCheck {
  name: string;
  ok: boolean;
  message: string;
}

export interface ProxyReport {
  ok: boolean;
  checks: ProxyCheck[];
}

export const settingsGet = () => invoke<SettingsView>("settings_get");

/** Секреты: undefined — не менять сохранённый, "" — удалить. */
export const settingsSave = (settings: Settings, secrets: { proxyPassword?: string; hfToken?: string } = {}) =>
  invoke<void>("settings_save", {
    settings,
    proxyPassword: secrets.proxyPassword ?? null,
    hfToken: secrets.hfToken ?? null,
  });

export interface TokenCheck {
  ok: boolean;
  message: string;
}

/** `token`: undefined — взять сохранённый. */
export const hfCheckToken = (hf: HfSettings, token?: string) =>
  invoke<TokenCheck>("hf_check_token", { hf, token: token ?? null });

// --- Мастер первого запуска ---

export type CheckStatus = "ok" | "warn" | "fail";

export interface SetupCheck {
  id: string;
  title: string;
  status: CheckStatus;
  message: string;
  fix: "vcredist" | null;
}

export interface DiskChoice {
  mount: string;
  path: string;
  free: number;
  total: number;
  enough: boolean;
  recommended: boolean;
}

export interface SetupInfo {
  hardware: Hardware;
  checks: SetupCheck[];
  disks: DiskChoice[];
  setup_done: boolean;
}

export const setupCheck = () => invoke<SetupInfo>("setup_check");
export const setupChooseDir = (path: string) => invoke<void>("setup_choose_dir", { path });
export const setupFinish = () => invoke<void>("setup_finish");
export const vcredistInstall = () => invoke<void>("vcredist_install");

/** `password`: undefined — взять сохранённый. */
export const proxyTest = (proxy: ProxySettings, password?: string) =>
  invoke<ProxyReport>("proxy_test", { proxy, password: password ?? null });

// --- Движки ---

export interface InstalledEngine {
  id: string;
  version: string;
  build: Build;
  dir: string;
  exe: string;
}

export interface EngineStatus {
  id: string;
  title: string;
  version: string;
  installed: InstalledEngine[];
  build: Build | null;
  size: number;
}

export interface EngineProgress {
  id: string;
  stage: "download" | "verify" | "unpack";
  done: number;
  total: number;
  speed: number;
}

/** Итог «Починить»: установленный движок + что было не так. */
export interface EngineRepair extends InstalledEngine {
  broken: string[];
  reinstalled: boolean;
}

export interface EngineFinished {
  id: string;
  error: string | null;
  /** `net` | `disk` | `broken` | `other`. */
  kind: string | null;
  /** После «Починить» — `EngineRepair`. */
  result: (InstalledEngine & Partial<Omit<EngineRepair, keyof InstalledEngine>>) | null;
}

export const engineStatus = (id: string) => invoke<EngineStatus>("engine_status", { id });

export const engineInstall = (id: string, build?: Build) =>
  invoke<void>("engine_install", { id, build: build ?? null });

export const engineRepair = (id: string) => invoke<void>("engine_repair", { id });

export const onEngineProgress = (cb: (p: EngineProgress) => void): Promise<UnlistenFn> =>
  listen<EngineProgress>("engine://progress", (e) => cb(e.payload));

export const onEngineFinished = (cb: (f: EngineFinished) => void): Promise<UnlistenFn> =>
  listen<EngineFinished>("engine://finished", (e) => cb(e.payload));

export const BUILD_NAMES: Record<Build, string> = {
  cuda13: "CUDA 13",
  cuda12: "CUDA 12",
  vulkan: "Vulkan",
};

export function formatBytes(b: number): string {
  const gb = b / 2 ** 30;
  if (gb >= 1000) return `${(gb / 1024).toFixed(1).replace(".", ",")} ТБ`;
  if (gb >= 1) return `${gb.toFixed(1).replace(".", ",")} ГБ`;
  return `${Math.round(b / 2 ** 20)} МБ`;
}

// --- Библиотека моделей ---

export type ModelKind =
  | "llm"
  | "projector"
  | "image"
  | "video"
  | "speech_to_text"
  | "vae"
  | "lora"
  | "control_net"
  | "text_encoder"
  | "upscaler"
  | "unknown";

/** Чем запускать. `needs_conversion` — формат не наш, нужна другая версия модели. */
export type ModelEngine = "llama_cpp" | "comfy_ui" | "whisper_cpp" | "needs_conversion" | "none";

/** Размеры текстовой модели — по ним считается память. */
export interface LlmDims {
  layers: number;
  ctx_train: number;
  embd: number;
  heads: number;
  heads_kv: number;
  head_dim: number;
  vocab: number;
  layer_bytes: number;
  other_bytes: number;
}

export interface ModelInfo {
  format: string;
  kind: ModelKind;
  engine: ModelEngine;
  family: string;
  name: string | null;
  license: string | null;
  params: number;
  weights_bytes: number;
  core_params: number;
  core_bytes: number;
  precision: string;
  llm: LlmDims | null;
  contains: string[];
  needs: string[];
  notes: string[];
}

/** «Светофор»: пойдёт ли модель на этом ПК. `none` — это дополнение, а не модель. */
export interface Verdict {
  light: "green" | "yellow" | "red" | "none";
  headline: string;
  details: string[];
  gpu_layers: number | null;
  ctx: number | null;
}

export interface Model {
  path: string;
  size: number;
  /** Дата изменения файла, unix-секунды. */
  mtime: number;
  /** Когда добавили, unix-секунды. */
  added: number;
  /** Понятное имя, если по имени файла не разобрать (у Ollama файлы — по хешу). */
  title: string | null;
  info: ModelInfo;
  file: string;
  kind_ru: string;
  /** Файла нет на месте. */
  missing: boolean;
  verdict: Verdict | null;
}

/** Итог добавления одного файла: `error` — почему не взяли. */
export interface AddedModel {
  file: string;
  error: string | null;
}

/** Итог поиска моделей по папкам. */
export interface ScanReport {
  added: number;
  /** Уже были в списке. */
  already: number;
  /** Не наш формат или файл не открылся. */
  skipped: number;
  /** Известные места, которые нашлись на этом ПК: «LM Studio», «Ollama»… */
  sources: string[];
}

export const modelsList = () => invoke<Model[]>("models_list");

/** Ищет модели в известных местах (LM Studio, Ollama, ComfyUI) и в указанных папках. */
export const modelsScan = (dirs: string[] = []) => invoke<ScanReport>("models_scan", { dirs });

export const modelsAdd = (paths: string[]) => invoke<AddedModel[]>("models_add", { paths });

/** Убирает из списка; файл на диске остаётся. */
export const modelsRemove = (path: string) => invoke<void>("models_remove", { path });

export const LIGHTS: Record<Verdict["light"], string> = {
  green: "🟢",
  yellow: "🟡",
  red: "🔴",
  none: "⚪",
};

// --- Текстовая модель (llama-server) ---

export interface LlmState {
  state: "starting" | "ready" | "stopped" | "crashed";
  model: string | null;
  port: number | null;
  /** Секунд от запуска до готовности. */
  started_in: number | null;
  error: string | null;
}

export interface LlmAnswer {
  text: string;
  tokens: number;
  /** Токенов в секунду. */
  speed: number;
  prompt_ms: number;
}

export const llmStatus = () => invoke<LlmState>("llm_status");
export const llmStart = (model: string, ctx?: number) =>
  invoke<void>("llm_start", { config: ctx ? { model, ctx } : { model } });
export const llmStop = () => invoke<void>("llm_stop");
export const llmAsk = (prompt: string) => invoke<LlmAnswer>("llm_ask", { prompt });

export const onLlmState = (cb: (s: LlmState) => void): Promise<UnlistenFn> =>
  listen<LlmState>("llm://state", (e) => cb(e.payload));

// --- Обновления программы ---

export interface UpdateAvailable {
  version: string;
  current: string;
  notes: string | null;
  date: string | null;
}

export interface UpdateProgress {
  done: number;
  total: number | null;
}

/** null — установлена свежая версия. `channel` — выбранный в настройках, ещё не сохранённый. */
export const updateCheck = (channel?: string) =>
  invoke<UpdateAvailable | null>("update_check", { channel: channel ?? null });

/** Ставит найденное обновление; по окончании программа перезапустится сама. */
export const updateInstall = () => invoke<void>("update_install");

export const onUpdateProgress = (cb: (p: UpdateProgress) => void): Promise<UnlistenFn> =>
  listen<UpdateProgress>("update://progress", (e) => cb(e.payload));

export const onUpdateFailed = (cb: (error: string) => void): Promise<UnlistenFn> =>
  listen<string>("update://failed", (e) => cb(e.payload));
