import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Лицензия модели одной фразой: главное, что человеку надо знать, — можно ли
 * пользоваться ей для работы. Коды — как на HuggingFace (`license:` в тегах).
 * Юридических тонкостей не пересказываем: где условия есть, ведём на страницу модели.
 */
type Kind = "free" | "terms" | "personal" | "unknown";

const FREE = /^(apache-2\.0|mit|bsd(-\d-clause)?|cc0-1\.0|cc-by-4\.0|cc-by-sa-4\.0|unlicense|(l|a)?gpl(-\d\.\d)?)$/;
// Можно для работы, но автор ставит условия: для кого, для чего, сколько пользователей.
const TERMS = /^(llama\d(\.\d)?|gemma|(creativeml-|bigscience-)?openrail(\+\+|-m)?|deepseek)$/;
// «nc» — non-commercial: зарабатывать нельзя.
const PERSONAL = /^cc-by-nc(-sa|-nd)?-\d\.\d$/;

const NAMES: Record<string, string> = {
  "apache-2.0": "Apache 2.0",
  mit: "MIT",
  gemma: "Gemma",
  other: "своя",
};

const SAYS: Record<Kind, string> = {
  free: "свободная: можно и дома, и для работы.",
  terms: "можно и для работы, но у автора есть условия.",
  personal: "только для себя: зарабатывать с её помощью нельзя.",
  unknown: "у автора свои условия — прочитайте их, если модель нужна для работы.",
};

export function licenseKind(code: string): Kind {
  const c = code.toLowerCase();
  if (FREE.test(c)) return "free";
  if (TERMS.test(c)) return "terms";
  if (PERSONAL.test(c)) return "personal";
  return "unknown";
}

export default function License({ code, repo }: { code: string | null; repo: string | null }) {
  const link = repo && (
    <button className="link" onClick={() => openUrl(`https://huggingface.co/${repo}`).catch(() => {})}>
      Условия на странице модели
    </button>
  );
  if (!code) {
    // Без лицензии молчать нельзя, если знаем, где посмотреть; не знаем — не выдумываем.
    return repo ? (
      <p className="muted small">
        Лицензия не указана. {link}
      </p>
    ) : null;
  }
  const kind = licenseKind(code);
  return (
    <p className="muted small">
      Лицензия {NAMES[code.toLowerCase()] ?? code} — {SAYS[kind]} {kind !== "free" && link}
    </p>
  );
}
