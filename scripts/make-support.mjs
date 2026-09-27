// Собирает всё про поддержку проекта из одного файла site/support/support.json, на двух языках:
// QR-коды и кнопки (site/support/*.svg), страницы DONATE.md (англ.) и DONATE.ru.md, блок в README.md
// и README.ru.md, раздел «Поддержать» на сайте (site/index.html, site/en/index.html)
// и .github/FUNDING.yml (кнопка «Sponsor» на GitHub).
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

// Кнопки — свои SVG, а не значки shields.io: крупнее, на нужном языке и без стороннего сервиса.
function button(title, subtitle, fill) {
  const font = "Segoe UI Variable, Segoe UI, -apple-system, Helvetica, Arial, sans-serif";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="280" height="64" viewBox="0 0 280 64" role="img" aria-label="${esc(title)}: ${esc(subtitle)}">
  <rect width="280" height="64" rx="14" fill="${fill}"/>
  <text x="140" y="30" text-anchor="middle" font-family="${font}" font-size="20" font-weight="700" fill="#fff">${esc(title)}</text>
  <text x="140" y="49" text-anchor="middle" font-family="${font}" font-size="13" fill="#fff" fill-opacity=".88">${esc(subtitle)}</text>
</svg>
`;
}

// Всё, что видит человек, — на двух языках. Русская страница поддержки — DONATE.ru.md,
// английская — DONATE.md (её GitHub показывает по ссылке «Sponsor»); на сайте — index.html и en/index.html.
const L = {
  ru: {
    boosty: "подписка на развитие проекта",
    yoomoney: "разово, картой любого банка",
    boostyAlt: "Boosty — подписка на развитие",
    yoomoneyAlt: "ЮMoney — разово, картой любого банка",
    qr: "QR-код",
    min: "Минимум —",
    network: (c) => c.network,
    note: (c) => c.note,
    title: "Поддержать Ollivo",
    lead: "Ollivo делает один человек. Программа бесплатная, без рекламы и слежки,\nа переписка не уходит с вашего компьютера.\nЕсли она вам пригодилась — помогите ей расти.",
    other: "**Русский** · [English](DONATE.md)",
    crypto: "Криптовалюта",
    warnMd: "> ⚠️ **Важно.** На адрес отправляйте **только указанную монету и только в указанной сети** — иначе перевод не дойдёт, и вернуть его не получится.\n> После вставки сверьте первые и последние 4 символа адреса.",
    goalsHead: "На что пойдут деньги",
    helpsHead: "Помочь можно и без денег",
    thanks: "Спасибо! ♥",
    goals: [
      ["Видеокарты AMD и Intel для проверки.", "Сейчас программа работает только с NVIDIA."],
      ["Сервер и трафик.", "Обновления программы раздаются с нашего сервера."],
      ["Домен ollivo.ru.", "Короткий адрес для сайта и обновлений."],
      ["Время.", "Чем больше поддержки, тем быстрее выходят картинки, голос и видео."],
    ],
    helps: (link) => [
      "Расскажите о программе знакомым, которым хотелось попробовать ИИ у себя на компьютере.",
      `Нашли ошибку — нажмите в программе «Сообщить о проблеме» или ${link("напишите на GitHub", `${repo}/issues/new/choose`)}.`,
      `Поставьте звезду ${link("репозиторию на GitHub", repo)} — так его чаще находят.`,
    ],
    readmeHead: "Поддержать проект",
    readmeLead: "Ollivo делает один человек. Если программа пригодилась — поддержите её развитие:",
    readmeCrypto: (coins) => `Криптовалюта (${coins}) — адреса и QR-коды на странице **[Поддержать](DONATE.ru.md)**.`,
    eyebrow: "Поддержать",
    siteHead: "Ollivo делает один человек",
    siteLead: "Программа бесплатная, без рекламы и слежки. Если она вам пригодилась — помогите ей расти.",
    boostyCard: "Подписка на развитие проекта — каждый месяц",
    yoomoneyCard: "Разово, любой суммой, картой любого банка",
    yoomoneyMark: "Ю",
    yoomoneyName: "ЮMoney",
    warnHtml: "Отправляйте <b>только указанную монету и только в указанной сети</b> — иначе перевод не дойдёт,\n        и вернуть его не получится. После вставки сверьте первые и последние 4 символа адреса.",
    copy: "Скопировать адрес",
  },
  en: {
    boosty: "a subscription for the project",
    yoomoney: "one-time, by card (Russia)",
    boostyAlt: "Boosty — a subscription for the project",
    yoomoneyAlt: "YooMoney — one-time, by card (Russia)",
    qr: "QR code",
    min: "Minimum —",
    network: (c) => c.network_en,
    note: (c) => c.note_en,
    title: "Support Ollivo",
    lead: "Ollivo is made by one person. The app is free, with no ads and no tracking,\nand your conversations never leave your computer.\nIf it has been useful to you, help it grow.",
    other: "[Русский](DONATE.ru.md) · **English**",
    crypto: "Crypto",
    warnMd: "> ⚠️ **Important.** Send **only the listed coin and only on the listed network** — otherwise the transfer won't arrive and can't be returned.\n> After pasting, check the first and last 4 characters of the address.",
    goalsHead: "Where the money goes",
    helpsHead: "Helping without money",
    thanks: "Thank you! ♥",
    goals: [
      ["AMD and Intel graphics cards for testing.", "Right now the app works only with NVIDIA."],
      ["Server and traffic.", "App updates are served from our own server."],
      ["The ollivo.ru domain.", "A short address for the site and updates."],
      ["Time.", "The more support, the sooner images, voice and video arrive."],
    ],
    helps: (link) => [
      "Tell friends who wanted to try AI on their own computer about the app.",
      `Found a bug — click “Report a problem” in the app or ${link("write on GitHub", `${repo}/issues/new/choose`)}.`,
      `Star the ${link("repository on GitHub", repo)} — it helps others find it.`,
    ],
    readmeHead: "Support the project",
    readmeLead: "Ollivo is made by one person. If the app has been useful, support its development:",
    readmeCrypto: (coins) => `Crypto (${coins}) — addresses and QR codes on the **[Support](DONATE.md)** page.`,
    eyebrow: "Support",
    siteHead: "Ollivo is made by one person",
    siteLead: "The app is free, with no ads and no tracking. If it has been useful to you, help it grow.",
    boostyCard: "A monthly subscription for the project",
    yoomoneyCard: "One-time, any amount, by card — convenient in Russia",
    yoomoneyMark: "Y",
    yoomoneyName: "YooMoney",
    warnHtml: "Send <b>only the listed coin and only on the listed network</b> — otherwise the transfer won't arrive\n        and can't be returned. After pasting, check the first and last 4 characters of the address.",
    copy: "Copy address",
  },
};
for (const c of data.crypto) {
  if (!c.network_en || (c.note && !c.note_en)) throw new Error(`У ${c.coin} в support.json нет network_en или note_en`);
}

// Кнопки: русские — boosty.svg, yoomoney.svg; английские — с суффиксом -en.
for (const [lang, sfx] of [["ru", ""], ["en", "-en"]]) {
  writeFileSync(join(dir, `boosty${sfx}.svg`), button("Boosty", L[lang].boosty, "#F15F2C"));
  writeFileSync(join(dir, `yoomoney${sfx}.svg`), button(L[lang].yoomoneyName, L[lang].yoomoney, "#8B3FFD"));
}

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

const buttons = (lang, h) => {
  const t = L[lang], sfx = lang === "en" ? "-en" : "";
  return (
    `<a href="${data.boosty}"><img src="site/support/boosty${sfx}.svg" alt="${t.boostyAlt}" height="${h}"></a>&nbsp;&nbsp;` +
    `<a href="${data.yoomoney}"><img src="site/support/yoomoney${sfx}.svg" alt="${t.yoomoneyAlt}" height="${h}"></a>`
  );
};

const extras = (lang, c) => [L[lang].note(c), c.min && `${L[lang].min} ${c.min}.`].filter(Boolean);

// Предупреждение — обычной цитатой, а не плашкой [!WARNING]: у плашки заголовок всегда английский.
// Адрес — в блоке кода: у такого блока GitHub сам показывает кнопку «скопировать».
const cryptoRows = (lang) =>
  data.crypto
    .map(
      (c) => `<tr>
<td width="176"><img src="site/support/${c.id}.svg" alt="${L[lang].qr} ${esc(c.coin)}" width="160"></td>
<td>

**${c.coin}** · ${L[lang].network(c)}${extras(lang, c).map((t) => `<br>
<sub>${t}</sub>`).join("")}

\`\`\`text
${c.address}
\`\`\`

</td>
</tr>`,
    )
    .join("\n");

const coins = data.crypto.map((c) => c.coin).join(", ");
const mdLink = (t, u) => `[${t}](${u})`;
const htmlLink = (t, u) => `<a href="${u}">${t}</a>`;

for (const [lang, file] of [["ru", "DONATE.ru.md"], ["en", "DONATE.md"]]) {
  const t = L[lang];
  writeFileSync(
    join(root, file),
    `<!-- ${generated} -->

<div align="center">

${t.other}

<img src="site/logo.svg" alt="" width="72">

# ${t.title}

${t.lead}

${buttons(lang, 64)}

</div>

## ${t.crypto}

${t.warnMd}

<table>
${cryptoRows(lang)}
</table>

## ${t.goalsHead}

${t.goals.map(([a, b]) => `- **${a}** ${b}`).join("\n")}

## ${t.helpsHead}

${t.helps(mdLink).map((x) => `- ${x}`).join("\n")}

<div align="center">

${t.thanks}

</div>
`,
  );
}

// Блок между метками заменяется целиком; меток нет — падаем, а не дописываем куда попало.
function replaceBlock(path, content, start, end) {
  const text = readFileSync(path, "utf8");
  const re = new RegExp(`${start}[\\s\\S]*?${end}`);
  if (!re.test(text)) throw new Error(`В ${path} нет меток ${start} … ${end}`);
  writeFileSync(path, text.replace(re, () => content));
}

for (const [lang, file] of [["ru", "README.ru.md"], ["en", "README.md"]]) {
  const t = L[lang];
  replaceBlock(
    join(root, file),
    `<!-- support:start — ${generated} -->
## ${t.readmeHead}

${t.readmeLead}

${buttons(lang, 56)}

${t.readmeCrypto(coins)}
<!-- support:end -->`,
    "<!-- support:start",
    "<!-- support:end -->",
  );
}

// Раздел сайта — готовым HTML, а не скриптом из support.json: работает и без JS,
// а адреса видны в том же диффе, что и в support.json. `up` — путь от страницы до site/.
for (const [lang, page, up] of [["ru", "index.html", ""], ["en", "en/index.html", "../"]]) {
  const t = L[lang];
  const coinCards = data.crypto
    .map(
      (c) => `        <article class="coin">
          <img src="${up}support/${c.id}.svg" alt="${t.qr} ${esc(c.coin)}" width="148" height="148" loading="lazy">
          <h4>${esc(c.coin)} <span>${esc(t.network(c))}</span></h4>
${extras(lang, c).map((x) => `          <p class="coin-note">${esc(x)}</p>`).join("\n")}
          <code class="addr">${esc(c.address)}</code>
          <button class="btn btn-ghost copy" type="button" data-copy="${esc(c.address)}">${t.copy}</button>
        </article>`,
    )
    .join("\n");
  replaceBlock(
    join(root, "site", page),
    `<!-- support:start — ${generated} -->
<section id="support" class="support">
  <div class="wrap">
    <div class="section-head reveal">
      <span class="eyebrow">${t.eyebrow}</span>
      <h2>${t.siteHead}</h2>
      <p>${t.siteLead}</p>
    </div>
    <div class="give">
      <a class="give-card reveal" href="${data.boosty}" style="--brand:#F15F2C">
        <span class="give-mark" aria-hidden="true">B</span>
        <span><b>Boosty</b><span>${t.boostyCard}</span></span>
        <span class="give-go" aria-hidden="true">→</span>
      </a>
      <a class="give-card reveal" href="${data.yoomoney}" style="--brand:#8B3FFD">
        <span class="give-mark" aria-hidden="true">${t.yoomoneyMark}</span>
        <span><b>${t.yoomoneyName}</b><span>${t.yoomoneyCard}</span></span>
        <span class="give-go" aria-hidden="true">→</span>
      </a>
    </div>
    <div class="coins reveal">
      <h3>${t.crypto}</h3>
      <p class="coins-warn">${t.warnHtml}</p>
      <div class="coin-grid">
${coinCards}
      </div>
    </div>
    <div class="give-more">
      <article class="card reveal">
        <h3>${t.goalsHead}</h3>
        <ul>
${t.goals.map(([a, b]) => `          <li><b>${a}</b> ${b}</li>`).join("\n")}
        </ul>
      </article>
      <article class="card reveal">
        <h3>${t.helpsHead}</h3>
        <ul>
${t.helps(htmlLink).map((x) => `          <li>${x}</li>`).join("\n")}
        </ul>
      </article>
    </div>
  </div>
</section>
<!-- support:end -->`,
    "<!-- support:start",
    "<!-- support:end -->",
  );
}

// В «Sponsor» у GitHub нет Boosty и ЮMoney среди своих площадок — только ссылки custom (до четырёх).
const funding = `# ${generated}
custom:
  - ${data.boosty}
  - ${data.yoomoney}
  - ${repo}/blob/main/DONATE.md
`;
writeFileSync(join(root, ".github", "FUNDING.yml"), funding);

console.log(`Готово: ${data.crypto.length} QR, DONATE.md и DONATE.ru.md, README.md и README.ru.md, site/index.html и site/en/index.html, .github/FUNDING.yml`);
