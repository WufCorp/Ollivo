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

export interface Settings {
  data_dir: string | null;
  proxy: ProxySettings;
  hf: HfSettings;
  setup_done: boolean;
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

export interface EngineFinished {
  id: string;
  error: string | null;
  result: InstalledEngine | null;
}

export const engineStatus = (id: string) => invoke<EngineStatus>("engine_status", { id });

export const engineInstall = (id: string, build?: Build) =>
  invoke<void>("engine_install", { id, build: build ?? null });

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
