import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../i18n";

/**
 * Лицензия модели одной фразой: главное, что человеку надо знать, — можно ли
 * пользоваться ей для работы. Коды — как на HuggingFace (`license:` в тегах).
 * Юридических тонкостей не пересказываем: где условия есть, ведём на страницу модели.
 */
type Kind = "free" | "terms" | "personal" | "unknown";

// Коды HF (`apache-2.0`) и полные названия из заголовков файлов
// («CreativeML Open RAIL++-M License») приводим к одному виду: строчные, пробелы → дефис.
// «nc» (non-commercial) проверяем первым: `cc-by-nc-4.0` начинается как свободная `cc-by`.
const PERSONAL = /(^|-)cc-by-nc|non-?commercial|research/;
const FREE = /^(apache|mit(-license)?$|bsd|cc0|cc-by-|unlicense|(l|a)?gpl)/;
// Можно для работы, но автор ставит условия: для кого, для чего, сколько пользователей.
const TERMS = /llama|gemma|rail|deepseek/;

const NAMES: Record<string, string> = {
  "apache-2.0": "Apache 2.0",
  mit: "MIT",
  gemma: "Gemma",
};

const says = (kind: Kind) =>
  ({
    free: t("свободная: можно и дома, и для работы.", "free: fine both at home and for work."),
    terms: t("можно и для работы, но у автора есть условия.", "fine for work too, but the author sets conditions."),
    personal: t("только для себя: зарабатывать с её помощью нельзя.", "personal use only: you can't make money with it."),
    unknown: t(
      "у автора свои условия — прочитайте их, если модель нужна для работы.",
      "the author has their own terms — read them if you need the model for work.",
    ),
  })[kind];

export function licenseKind(code: string): Kind {
  const c = code.trim().toLowerCase().replace(/[\s_]+/g, "-");
  if (PERSONAL.test(c)) return "personal";
  if (FREE.test(c)) return "free";
  if (TERMS.test(c)) return "terms";
  return "unknown";
}

export default function License({ code, repo }: { code: string | null; repo: string | null }) {
  const link = repo && (
    <button className="link" onClick={() => openUrl(`https://huggingface.co/${repo}`).catch(() => {})}>
      {t("Условия на странице модели", "Terms on the model page")}
    </button>
  );
  if (!code) {
    // Без лицензии молчать нельзя, если знаем, где посмотреть; не знаем — не выдумываем.
    return repo ? (
      <p className="muted small">
        {t("Лицензия не указана.", "No license given.")} {link}
      </p>
    ) : null;
  }
  const kind = licenseKind(code);
  return (
    <p className="muted small">
      {t("Лицензия", "License")}{" "}
      {code.toLowerCase() === "other" ? t("особая", "custom") : (NAMES[code.toLowerCase()] ?? code)} — {says(kind)}{" "}
      {kind !== "free" && link}
    </p>
  );
}
