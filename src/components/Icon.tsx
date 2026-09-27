/**
 * Значки программы — линией, цветом текста. Свои, а не эмодзи: эмодзи в Windows
 * рисуются цветными картинками и выглядят чужими рядом с кнопками.
 */
const PATHS = {
  chat: "M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5v9a1.5 1.5 0 0 1-1.5 1.5H9l-5 4z",
  catalog:
    "M5.5 4h3.5A1.5 1.5 0 0 1 10.5 5.5V9A1.5 1.5 0 0 1 9 10.5H5.5A1.5 1.5 0 0 1 4 9V5.5A1.5 1.5 0 0 1 5.5 4zM15 4h3.5A1.5 1.5 0 0 1 20 5.5V9a1.5 1.5 0 0 1-1.5 1.5H15A1.5 1.5 0 0 1 13.5 9V5.5A1.5 1.5 0 0 1 15 4zM5.5 13.5H9a1.5 1.5 0 0 1 1.5 1.5v3.5A1.5 1.5 0 0 1 9 20H5.5A1.5 1.5 0 0 1 4 18.5V15a1.5 1.5 0 0 1 1.5-1.5zM15 13.5h3.5A1.5 1.5 0 0 1 20 15v3.5a1.5 1.5 0 0 1-1.5 1.5H15a1.5 1.5 0 0 1-1.5-1.5V15a1.5 1.5 0 0 1 1.5-1.5z",
  models: "M12 3l8 4.5v9L12 21l-8-4.5v-9zM4 7.5l8 4.5 8-4.5M12 12v9",
  computer: "M4.5 4h15A1.5 1.5 0 0 1 21 5.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 14.5v-9A1.5 1.5 0 0 1 4.5 4zM8 20h8M12 16v4",
  settings: "M4 7h9M17 7h3M4 17h3M11 17h9M15 5a2 2 0 1 1 0 4 2 2 0 0 1 0-4zM9 15a2 2 0 1 1 0 4 2 2 0 0 1 0-4z",
  report: "M5 21V4M5 4h11l-2 4 2 4H5",
  plus: "M12 5v14M5 12h14",
  search: "M11 4.5a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM16 16l4 4",
  clip: "M20 11.5l-8.2 8.2a5 5 0 0 1-7.1-7.1l8.5-8.5a3.3 3.3 0 0 1 4.7 4.7l-8.5 8.5a1.7 1.7 0 0 1-2.4-2.4l7.8-7.8",
  folder:
    "M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2.5h8.5A1.5 1.5 0 0 1 21 9v9.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z",
  mic: "M12 3a3 3 0 0 1 3 3v5a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3zM5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21",
  doc: "M7 3h7l5 5v13H7zM14 3v5h5M10 13h6M10 17h4",
  close: "M6 6l12 12M18 6L6 18",
  down: "M7 10l5 5 5-5",
  send: "M12 19V5M6 11l6-6 6 6",
  warn: "M12 4l9 16H3zM12 10v4M12 17.5v.5",
  gauge: "M4 16a8 8 0 1 1 16 0M12 16l4-5",
  panel: "M4.5 4h15A1.5 1.5 0 0 1 21 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5v-13A1.5 1.5 0 0 1 4.5 4zM15 4v16",
} as const;

export type IconName = keyof typeof PATHS;

export default function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  return (
    <svg
      className="icon-svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}

/** Значок Ollivo — тот же, что у программы и на сайте: «О»-облачко с точкой-моделью. */
export function Logo({ size = 32 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden="true">
      <rect x="24" y="24" width="976" height="976" rx="228" fill="#3F6115" />
      <path
        fill="#F2EFE2"
        fillRule="evenodd"
        d="M510 198a292 292 0 1 1 0 584 292 292 0 0 1 0-584zm0 101a191 191 0 1 0 0 382 191 191 0 0 0 0-382z"
      />
      <path
        d="M722 668 C728 700 744 730 752 758 C760 780 744 790 728 784 C700 772 672 750 636 744 Z"
        fill="#F2EFE2"
        stroke="#F2EFE2"
        strokeWidth="22"
        strokeLinejoin="round"
      />
      <circle cx="510" cy="490" r="75" fill="#BCDA93" />
    </svg>
  );
}
