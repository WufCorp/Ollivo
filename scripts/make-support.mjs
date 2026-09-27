// Собирает всё про поддержку проекта из одного файла site/support/support.json:
// QR-коды и кнопки (site/support/*.svg), страницу DONATE.md, блок в README.md,
// раздел «Поддержать» на сайте (site/index.html) и .github/FUNDING.yml (кнопка «Sponsor» на GitHub).
// Адреса живут только в support.json: если их подменят, это видно в одном диффе,
// а не в пяти файлах, где легко пропустить один.
//
//   node scripts/make-support.mjs

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import QRCode from "qrcode";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dir = join(root, "site", "support");
const data = JSON.parse(readFileSync(join(dir, "support.json"), "utf8"));
const repo = "https://github.com/WufCorp/Ollivo";
const generated = "Собрано scripts/make-support.mjs из site/support/support.json — правьте там.";

const unfilled = JSON.stringify(data).includes("ВСТАВИТЬ");
if (unfilled) console.warn("⚠ В support.json остались заглушки «ВСТАВИТЬ» — не выкладывайте так.");

// Сверяем контрольную сумму адреса: одна потерянная при копировании буква — и донаты уходят в никуда.
// Все три формата несут её в себе, так что опечатку видно без обращения к сети.
const sha256 = (b) => createHash("sha256").update(b).digest();
const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

function base58check(addr, version) {
  let n = 0n;
  for (const ch of addr) {
    const i = B58.indexOf(ch);
    if (i < 0) return false;
    n = n * 58n + BigInt(i);
  }
  const bytes = [];
  for (; n > 0n; n >>= 8n) bytes.unshift(Number(n & 0xffn));
  for (const ch of addr) if (ch === "1") bytes.unshift(0); else break;
  const buf = Buffer.from(bytes);
  if (buf.length !== 25 || buf[0] !== version) return false;
  return sha256(sha256(buf.subarray(0, 21))).subarray(0, 4).equals(buf.subarray(21));
}

// Адрес TON: 36 байт в base64url — флаги, workchain, хеш и CRC16-XMODEM первых 34 байт.
function tonAddress(addr) {
  const buf = Buffer.from(addr.replace(/-/g, "+").replace(/_/g, "/"), "base64");
  if (addr.length !== 48 || buf.length !== 36) return false;
  let crc = 0;
  for (const byte of buf.subarray(0, 34)) {
    crc ^= byte << 8;
    for (let i = 0; i < 8; i++) crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
  }
  return buf.readUInt16BE(34) === crc;
}

const checks = {
  tron: (a) => base58check(a, 0x41),
  btc: (a) => (/^bc1/.test(a) ? null : base58check(a, a.startsWith("3") ? 0x05 : 0x00)),
  ton: tonAddress,
};
for (const c of data.crypto) {
  if (c.address.includes("ВСТАВИТЬ")) continue;
  const ok = checks[c.check]?.(c.address);
  if (ok === false) throw new Error(`Адрес ${c.coin} (${c.address}) не сходится по контрольной сумме — скопируйте заново`);
  if (ok == null) console.warn(`⚠ Адрес ${c.coin} не проверен: нет проверки «${c.check}»`);
}

const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

// Кнопки — свои SVG, а не значки shields.io: крупнее, по-русски и без стороннего сервиса.
function button(title, subtitle, fill) {
  const font = "Segoe UI Variable, Segoe UI, -apple-system, Helvetica, Arial, sans-serif";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="280" height="64" viewBox="0 0 280 64" role="img" aria-label="${esc(title)}: ${esc(subtitle)}">
  <rect width="280" height="64" rx="14" fill="${fill}"/>
  <text x="140" y="30" text-anchor="middle" font-family="${font}" font-size="20" font-weight="700" fill="#fff">${esc(title)}</text>
  <text x="140" y="49" text-anchor="middle" font-family="${font}" font-size="13" fill="#fff" fill-opacity=".88">${esc(subtitle)}</text>
</svg>
`;
}

writeFileSync(join(dir, "boosty.svg"), button("Boosty", "подписка на развитие проекта", "#F15F2C"));
writeFileSync(join(dir, "yoomoney.svg"), button("ЮMoney", "разово, картой любого банка", "#8B3FFD"));

// В QR — голый адрес, без «bitcoin:» и «ton://»: так его понимает любой кошелёк.
// Белое поле вокруг кода — чтобы сканировался и в тёмной теме GitHub.
for (const c of data.crypto) {
  const svg = await QRCode.toString(c.address, {
    type: "svg",
    margin: 2,
    errorCorrectionLevel: "M",
    color: { dark: "#111111", light: "#ffffff" },
  });
  writeFileSync(join(dir, `${c.id}.svg`), svg);
}

const buttons = (h) =>
  `<a href="${data.boosty}"><img src="site/support/boosty.svg" alt="Boosty — подписка на развитие" height="${h}"></a>&nbsp;&nbsp;` +
  `<a href="${data.yoomoney}"><img src="site/support/yoomoney.svg" alt="ЮMoney — разово, картой любого банка" height="${h}"></a>`;

// Предупреждение — обычной цитатой, а не плашкой [!WARNING]: у плашки заголовок всегда английский.
// Адрес — в блоке кода: у такого блока GitHub сам показывает кнопку «скопировать».
const cryptoRows = data.crypto
  .map(
    (c) => `<tr>
<td width="176"><img src="site/support/${c.id}.svg" alt="QR-код ${esc(c.coin)}" width="160"></td>
<td>

**${c.coin}** · ${c.network}${[c.note, c.min && `Минимум — ${c.min}.`].filter(Boolean).map((t) => `<br>
<sub>${t}</sub>`).join("")}

\`\`\`text
${c.address}
\`\`\`

</td>
</tr>`,
  )
  .join("\n");

const coins = data.crypto.map((c) => c.coin).join(", ");

// Общий текст для GitHub и сайта. link — как оформить ссылку в нужном формате.
const goals = [
  ["Видеокарты AMD и Intel для проверки.", "Сейчас программа работает только с NVIDIA."],
  ["Сервер и трафик.", "Обновления программы раздаются с нашего сервера."],
  ["Домен ollivo.ru.", "Короткий адрес для сайта и обновлений."],
  ["Время.", "Чем больше поддержки, тем быстрее выходят картинки, голос и видео."],
];
const helps = (link) => [
  "Расскажите о программе знакомым, которым хотелось попробовать ИИ у себя на компьютере.",
  `Нашли ошибку — нажмите в программе «Сообщить о проблеме» или ${link("напишите на GitHub", `${repo}/issues/new/choose`)}.`,
  `Поставьте звезду ${link("репозиторию на GitHub", repo)} — так его чаще находят.`,
];
const mdLink = (t, u) => `[${t}](${u})`;
const htmlLink = (t, u) => `<a href="${u}">${t}</a>`;

const donate = `<!-- ${generated} -->

<div align="center">

<img src="site/logo.svg" alt="" width="72">

# Поддержать Ollivo

Ollivo делает один человек. Программа бесплатная, без рекламы и слежки,
а переписка не уходит с вашего компьютера.
Если она вам пригодилась — помогите ей расти.

${buttons(64)}

</div>

## Криптовалюта

> ⚠️ **Важно.** На адрес отправляйте **только указанную монету и только в указанной сети** — иначе перевод не дойдёт, и вернуть его не получится.
> После вставки сверьте первые и последние 4 символа адреса.

<table>
${cryptoRows}
</table>

## На что пойдут деньги

${goals.map(([t, d]) => `- **${t}** ${d}`).join("\n")}

## Помочь можно и без денег

${helps(mdLink).map((t) => `- ${t}`).join("\n")}

<div align="center">

Спасибо! ♥

</div>
`;
writeFileSync(join(root, "DONATE.md"), donate);

// Блок между метками заменяется целиком; меток нет — падаем, а не дописываем куда попало.
function replaceBlock(path, content, start, end) {
  const text = readFileSync(path, "utf8");
  const re = new RegExp(`${start}[\\s\\S]*?${end}`);
  if (!re.test(text)) throw new Error(`В ${path} нет меток ${start} … ${end}`);
  writeFileSync(path, text.replace(re, () => content));
}

const readmeBlock = `<!-- support:start — ${generated} -->
## Поддержать проект

Ollivo делает один человек. Если программа пригодилась — поддержите её развитие:

${buttons(56)}

Криптовалюта (${coins}) — адреса и QR-коды на странице **[Поддержать](DONATE.md)**.
<!-- support:end -->`;
replaceBlock(join(root, "README.md"), readmeBlock, "<!-- support:start", "<!-- support:end -->");

// Раздел сайта — готовым HTML, а не скриптом из support.json: работает и без JS,
// а адреса видны в том же диффе, что и в support.json.
const coinCards = data.crypto
  .map(
    (c) => `        <article class="coin">
          <img src="support/${c.id}.svg" alt="QR-код ${esc(c.coin)}" width="148" height="148" loading="lazy">
          <h4>${esc(c.coin)} <span>${esc(c.network)}</span></h4>
${[c.note, c.min && `Минимум — ${c.min}.`].filter(Boolean).map((t) => `          <p class="coin-note">${esc(t)}</p>`).join("\n")}
          <code class="addr">${esc(c.address)}</code>
          <button class="btn btn-ghost copy" type="button" data-copy="${esc(c.address)}">Скопировать адрес</button>
        </article>`,
  )
  .join("\n");

const siteBlock = `<!-- support:start — ${generated} -->
<section id="support" class="support">
  <div class="wrap">
    <div class="section-head reveal">
      <span class="eyebrow">Поддержать</span>
      <h2>Ollivo делает один человек</h2>
      <p>Программа бесплатная, без рекламы и слежки. Если она вам пригодилась — помогите ей расти.</p>
    </div>
    <div class="give">
      <a class="give-card reveal" href="${data.boosty}" style="--brand:#F15F2C">
        <span class="give-mark" aria-hidden="true">B</span>
        <span><b>Boosty</b><span>Подписка на развитие проекта — каждый месяц</span></span>
        <span class="give-go" aria-hidden="true">→</span>
      </a>
      <a class="give-card reveal" href="${data.yoomoney}" style="--brand:#8B3FFD">
        <span class="give-mark" aria-hidden="true">Ю</span>
        <span><b>ЮMoney</b><span>Разово, любой суммой, картой любого банка</span></span>
        <span class="give-go" aria-hidden="true">→</span>
      </a>
    </div>
    <div class="coins reveal">
      <h3>Криптовалюта</h3>
      <p class="coins-warn">Отправляйте <b>только указанную монету и только в указанной сети</b> — иначе перевод не дойдёт,
        и вернуть его не получится. После вставки сверьте первые и последние 4 символа адреса.</p>
      <div class="coin-grid">
${coinCards}
      </div>
    </div>
    <div class="give-more">
      <article class="card reveal">
        <h3>На что пойдут деньги</h3>
        <ul>
${goals.map(([t, d]) => `          <li><b>${t}</b> ${d}</li>`).join("\n")}
        </ul>
      </article>
      <article class="card reveal">
        <h3>Помочь можно и без денег</h3>
        <ul>
${helps(htmlLink).map((t) => `          <li>${t}</li>`).join("\n")}
        </ul>
      </article>
    </div>
  </div>
</section>
<!-- support:end -->`;
replaceBlock(join(root, "site", "index.html"), siteBlock, "<!-- support:start", "<!-- support:end -->");

// В «Sponsor» у GitHub нет Boosty и ЮMoney среди своих площадок — только ссылки custom (до четырёх).
const funding = `# ${generated}
custom:
  - ${data.boosty}
  - ${data.yoomoney}
  - ${repo}/blob/main/DONATE.md
`;
writeFileSync(join(root, ".github", "FUNDING.yml"), funding);

console.log(`Готово: ${data.crypto.length} QR, DONATE.md, README.md, site/index.html, .github/FUNDING.yml`);
