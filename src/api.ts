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

export const downloadPause = (id: string) => invoke<void>("download_pause", { id });

export const onDownloadProgress = (cb: (p: DownloadProgress) => void): Promise<UnlistenFn> =>
  listen<DownloadProgress>("download://progress", (e) => cb(e.payload));

export const onDownloadFinished = (cb: (f: DownloadFinished) => void): Promise<UnlistenFn> =>
  listen<DownloadFinished>("download://finished", (e) => cb(e.payload));

export function formatBytes(b: number): string {
  const gb = b / 2 ** 30;
  if (gb >= 1) return `${gb.toFixed(1).replace(".", ",")} ГБ`;
  return `${Math.round(b / 2 ** 20)} МБ`;
}
