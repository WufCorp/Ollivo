// Общий скрипт сайта для русской и английской страниц. Язык страницы — из <html lang>.
const EN = document.documentElement.lang === "en";
const T = EN
  ? {
      copied: "Copied ✓",
      copy: "Copy address",
      soon: "First build — coming soon",
      soonLine: "The first public build is being prepared. The releases page on GitHub will open as soon as it's out — the button leads there.",
      download: (ver, mb) => `Download ${ver} · ${mb} MB`,
    }
  : {
      copied: "Скопировано ✓",
      copy: "Скопировать адрес",
      soon: "Первая сборка — скоро",
      soonLine: "Первая публичная сборка готовится. Страница выпусков на GitHub откроется, как только она выйдет, — туда ведёт кнопка.",
      download: (ver, mb) => `Скачать ${ver} · ${mb} МБ`,
    };

// Язык, переданный программой (`?lang=`), — тоже выбор: русская страница решает это ещё в <head>.
try {
  const q = new URLSearchParams(location.search).get("lang");
  if (q === "ru" || q === "en") localStorage.setItem("ollivo-lang", q);
} catch { /* без хранилища — не запоминаем */ }

// Переключатель языка: выбор запоминается, и русская страница больше не решает за человека.
document.querySelectorAll("[data-lang]").forEach((a) =>
  a.addEventListener("click", () => {
    try { localStorage.setItem("ollivo-lang", a.dataset.lang); } catch { /* без хранилища — просто переход */ }
  }),
);

// Шапка с линией, когда страницу прокрутили.
const top_ = document.getElementById("top");
const onScroll = () => top_.classList.toggle("scrolled", scrollY > 8);
addEventListener("scroll", onScroll, { passive: true });
onScroll();

// Блоки проявляются при прокрутке.
const io = new IntersectionObserver((entries) => {
  for (const e of entries) if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
}, { rootMargin: "0px 0px -8% 0px" });
document.querySelectorAll(".reveal").forEach((el) => io.observe(el));

// «Скопировать адрес»: без буфера обмена (старый браузер, http) адрес выделяется целиком — Ctrl+C.
document.querySelectorAll(".copy").forEach((b) => b.addEventListener("click", async () => {
  const text = b.dataset.copy;
  try {
    await navigator.clipboard.writeText(text);
    b.textContent = T.copied;
    b.classList.add("done");
    setTimeout(() => { b.textContent = T.copy; b.classList.remove("done"); }, 2000);
  } catch {
    const code = b.parentElement.querySelector(".addr");
    getSelection().selectAllChildren(code);
  }
}));

// Кнопка «Скачать» — прямо на установщик последней версии. Нет выпуска или GitHub недоступен —
// кнопка остаётся ссылкой на страницу выпусков, это честнее, чем прятать её.
(async () => {
  try {
    const r = await fetch("https://api.github.com/repos/WufCorp/Ollivo/releases/latest", { headers: { Accept: "application/vnd.github+json" } });
    if (r.status === 404) {
      document.querySelectorAll(".js-download-text").forEach((el) => (el.textContent = T.soon));
      const line = document.querySelector(".js-release-line");
      if (line) line.textContent = T.soonLine;
      return;
    }
    if (!r.ok) return;
    const rel = await r.json();
    const exe = (rel.assets || []).find((a) => /setup\.exe$/i.test(a.name)) || (rel.assets || []).find((a) => /\.exe$/i.test(a.name));
    if (!exe) return;
    const mb = (exe.size / 1048576).toFixed(0);
    const ver = String(rel.tag_name || "").replace(/^v/, "");
    document.querySelectorAll(".js-download").forEach((a) => (a.href = exe.browser_download_url));
    document.querySelectorAll(".js-download-text").forEach((el) => (el.textContent = T.download(ver, mb)));
    // GitHub сам считает SHA256 файлов выпуска (поле digest) — показываем его, если есть.
    if (exe.digest && exe.digest.startsWith("sha256:")) {
      document.querySelector(".js-hash-value").textContent = exe.digest.slice(7);
      document.querySelector(".js-hash").hidden = false;
    }
  } catch { /* нет сети или лимит API — остаётся ссылка на выпуски */ }
})();
