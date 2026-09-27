import type { Verdict } from "../api";

/** Точка «светофора»: цвет — из темы, чтобы в тёмной и светлой читался одинаково. */
export function Light({ light }: { light: Verdict["light"] | "missing" }) {
  return <i className={`light ${light}`} aria-hidden="true" />;
}

/** Строка про память: её показываем подписью к шкале, а не отдельной строкой. */
const MEMORY = /^(нужно|занято будет) ~/;

/**
 * Подробности вердикта со шкалой «влезет ли в видеокарту»: полоса — сколько нужно
 * от свободного. Числа человек видит до того, как скачает, — ради этого шкала и есть.
 */
export default function Fit({ verdict }: { verdict: Verdict }) {
  const { need, room } = verdict;
  const gauge = need !== null && room !== null && room > 0;
  const caption = gauge ? verdict.details.find((d) => MEMORY.test(d)) : undefined;
  const rest = verdict.details.filter((d) => d !== caption);
  // Больше свободного — полоса целиком, цвет «светофора» скажет остальное.
  const k = gauge ? Math.min(1, need / room) : 0;

  return (
    <>
      {gauge && (
        <div className="gauge">
          <span className="gauge-label">{caption ?? "видеокарта"}</span>
          <div className="bar" role="img" aria-label={caption ?? "сколько нужно видеопамяти"}>
            <i className={verdict.light} style={{ transform: `scaleX(${k})` }} />
          </div>
        </div>
      )}
      {rest.map((d) => (
        <p key={d} className="muted small">
          {d}
        </p>
      ))}
    </>
  );
}
