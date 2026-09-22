import { useState } from "react";
import { hfCheckToken, type HfSettings, type HfSource, type TokenCheck } from "../api";

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
        Откуда качать модели
        <select
          value={hf.source}
          onChange={(e) => {
            onChange({ ...hf, source: e.target.value as HfSource });
            setCheck(null);
          }}
        >
          <option value="official">HuggingFace (huggingface.co)</option>
          <option value="mirror">Зеркало hf-mirror.com</option>
          <option value="custom">Своё зеркало</option>
        </select>
      </label>
      {hf.source === "mirror" && (
        <p className="muted small">
          Зеркало помогает, если huggingface.co не открывается. Из некоторых стран оно само перенаправляет на
          huggingface.co — тогда поможет только прокси.
        </p>
      )}
      {hf.source === "custom" && (
        <label>
          Адрес зеркала
          <input
            value={hf.custom_url}
            placeholder="https://hf.example.ru"
            spellCheck={false}
            onChange={(e) => onChange({ ...hf, custom_url: e.target.value })}
          />
        </label>
      )}

      <label>
        Токен HuggingFace
        <input
          type="password"
          value={token ?? ""}
          placeholder={hasToken ? "сохранён" : "hf_…"}
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => {
            onToken(e.target.value);
            setCheck(null);
          }}
        />
      </label>
      <p className="muted small">
        Нужен только для закрытых моделей (Llama, Gemma, Flux dev): сначала примите лицензию на странице модели, потом
        создайте токен «Read» в настройках аккаунта HuggingFace → Access Tokens.
      </p>
      {(token || hasToken) && (
        <div className="actions">
          <button className="secondary" onClick={run} disabled={checking}>
            {checking ? "Проверяю…" : "Проверить токен"}
          </button>
          {check && <span className={check.ok ? "ok" : "error"}>{check.message}</span>}
        </div>
      )}
    </>
  );
}
