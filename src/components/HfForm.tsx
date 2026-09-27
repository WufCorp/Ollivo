import { useState } from "react";
import { hfCheckToken, type HfSettings, type HfSource, type TokenCheck } from "../api";
import { t } from "../i18n";

interface Props {
  hf: HfSettings;
  onChange: (hf: HfSettings) => void;
  /** undefined — токен не трогали, сохранённый остаётся. */
  token: string | undefined;
  onToken: (t: string) => void;
  hasToken: boolean;
}

export default function HfForm({ hf, onChange, token, onToken, hasToken }: Props) {
  const [check, setCheck] = useState<TokenCheck | null>(null);
  const [checking, setChecking] = useState(false);

  const run = async () => {
    setChecking(true);
    setCheck(null);
    try {
      setCheck(await hfCheckToken(hf, token));
    } catch (e) {
      setCheck({ ok: false, message: String(e) });
    } finally {
      setChecking(false);
    }
  };

  return (
    <>
      <label>
        {t("Откуда качать модели", "Where to download models from")}
        <select
          value={hf.source}
          onChange={(e) => {
            onChange({ ...hf, source: e.target.value as HfSource });
            setCheck(null);
          }}
        >
          <option value="official">HuggingFace (huggingface.co)</option>
          <option value="mirror">{t("Зеркало hf-mirror.com", "Mirror hf-mirror.com")}</option>
          <option value="custom">{t("Своё зеркало", "Custom mirror")}</option>
        </select>
      </label>
      {hf.source === "mirror" && (
        <p className="muted small">
          {t(
            "Зеркало помогает, если huggingface.co не открывается. Из некоторых стран оно само перенаправляет на huggingface.co — тогда поможет только прокси.",
            "A mirror helps if huggingface.co won't open. From some countries it redirects to huggingface.co itself — then only a proxy helps.",
          )}
        </p>
      )}
      {hf.source === "custom" && (
        <label>
          {t("Адрес зеркала", "Mirror address")}
          <input
            value={hf.custom_url}
            placeholder="https://hf.example.ru"
            spellCheck={false}
            onChange={(e) => onChange({ ...hf, custom_url: e.target.value })}
          />
        </label>
      )}

      <label>
        {t("Токен HuggingFace", "HuggingFace token")}
        <input
          type="password"
          value={token ?? ""}
          placeholder={hasToken ? t("сохранён", "saved") : "hf_…"}
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => {
            onToken(e.target.value);
            setCheck(null);
          }}
        />
      </label>
      <p className="muted small">
        {t(
          "Нужен только для закрытых моделей (Llama, Gemma, Flux dev): сначала примите лицензию на странице модели, потом создайте токен «Read» в настройках аккаунта HuggingFace → Access Tokens.",
          "Only needed for gated models (Llama, Gemma, Flux dev): first accept the license on the model page, then create a “Read” token in your HuggingFace account settings → Access Tokens.",
        )}
      </p>
      {(token || hasToken) && (
        <div className="actions">
          <button className="secondary" onClick={run} disabled={checking}>
            {checking ? t("Проверяю…", "Checking…") : t("Проверить токен", "Check token")}
          </button>
          {check && <span className={check.ok ? "ok" : "error"}>{check.message}</span>}
        </div>
      )}
    </>
  );
}
