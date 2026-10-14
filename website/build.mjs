// Builds the static site into website/public:
//   - every page in English (/) and Spanish (/es/) from src/pages and
//     src/partials, with the texts in src/en.json and src/es.json
//   - Google AdSense code, ad slots and ads.txt when configured in
//     src/site.json (nothing ad-related is emitted otherwise)
//   - copies the Windows build (../dist/ondalux.exe) to public/downloads
//     with the version in its name; its size and SHA-256 go into the pages
//   - sitemap.xml
//
// Usage: node website/build.mjs        (no dependencies, Node 18+)

import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const src = join(here, "src");
const pub = join(here, "public");
const domain = "https://ondaluxheatmap.com";
const config = JSON.parse(readFileSync(join(src, "site.json"), "utf8"));

// ---- version and download ----
const cargo = readFileSync(join(root, "Cargo.toml"), "utf8");
const version = /^version\s*=\s*"([^"]+)"/m.exec(cargo)[1];
const msrv = (/^rust-version\s*=\s*"([^"]+)"/m.exec(cargo) || [])[1] || "stable";
const exeName = `ondalux-heatmap-${version}-windows-x64.exe`;
const exeSrc = join(root, "dist", "ondalux.exe");
const downloads = join(pub, "downloads");
mkdirSync(downloads, { recursive: true });
for (const f of readdirSync(downloads)) {
  if (f.endsWith(".exe") && f !== exeName) rmSync(join(downloads, f));
}
copyFileSync(exeSrc, join(downloads, exeName));
const exe = {
  name: exeName,
  size: `${(statSync(exeSrc).size / 1024 / 1024).toFixed(1)} MB`,
  sha256: createHash("sha256").update(readFileSync(exeSrc)).digest("hex"),
};

// ---- GitHub ----
const repoUrl = (config.github || "").replace(/\/+$/, "").replace(/\.git$/, "");
const repoName = repoUrl ? repoUrl.split("/").pop() : "ondalux-heatmap";
const github = {
  enabled: Boolean(repoUrl),
  url: repoUrl,
  cloneUrl: repoUrl ? `${repoUrl}.git` : "https://github.com/<user>/ondalux-heatmap.git",
  name: repoName,
};

// ---- Google AdSense ----
const client = (config.ads?.client || "").trim();
if (client && !/^ca-pub-\d{10,20}$/.test(client)) {
  throw new Error(`site.json ads.client must look like ca-pub-1234567890123456, got "${client}"`);
}
const adsEnabled = Boolean(client);
const adsPreview = !adsEnabled && (Boolean(config.ads?.preview) || process.env.ADS_PREVIEW === "1");
const adsHead = adsEnabled
  ? `<meta name="google-adsense-account" content="${client}">
  <script async src="https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js?client=${client}" crossorigin="anonymous"></script>`
  : "";
const adLabel = { en: "Advertisement", es: "Publicidad" };
function adSlot(name, lang) {
  const slot = (config.ads?.slots?.[name] || "").trim();
  const label = `<span class="ad__label">${adLabel[lang]}</span>`;
  if (adsPreview) {
    return `<aside class="ad wrap" aria-label="${adLabel[lang]}">${label}<div class="ad__placeholder">${name}</div></aside>`;
  }
  // Without a slot id, Auto ads (enabled in AdSense) decide placement.
  if (!adsEnabled || !slot) return "";
  return `<aside class="ad wrap" aria-label="${adLabel[lang]}">${label}<ins class="adsbygoogle" style="display:block" data-ad-client="${client}" data-ad-slot="${slot}" data-ad-format="auto" data-full-width-responsive="true"></ins></aside>`;
}
const slotNames = [
  "home_top", "home_middle", "home_bottom", "guide_top", "guide_bottom", "oss_middle",
  "articles_middle", "article_top", "article_bottom",
];
if (adsEnabled) {
  // https://support.google.com/adsense/answer/12171612
  writeFileSync(join(pub, "ads.txt"), `google.com, ${client.replace(/^ca-/, "")}, DIRECT, f08c47fec0942fa0\n`);
} else if (existsSync(join(pub, "ads.txt"))) {
  rmSync(join(pub, "ads.txt"));
}

// ---- icons for the feature cards (simple 24×24 stroke icons) ----
const svg = (d) => `<svg viewBox="0 0 24 24" aria-hidden="true">${d}</svg>`;
const icons = {
  predict: svg('<path d="M3 21V9l9-6 9 6v12"/><circle cx="12" cy="13" r="2"/><path d="M8.5 9.5a5 5 0 0 1 7 0M6 7a8.5 8.5 0 0 1 12 0"/>'),
  catalog: svg('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 8h10M7 12h10M7 16h6"/>'),
  mount: svg('<path d="M4 3v18"/><circle cx="9" cy="12" r="2.5"/><path d="M13 7.5a6.5 6.5 0 0 1 0 9M16.5 5a10 10 0 0 1 0 14"/>'),
  walls: svg('<path d="M3 7h18M3 12h18M3 17h18M8 7v5M16 7v5M12 12v5M5 17v4M19 17v4"/><path d="M18 2l1 2 2 1-2 1-1 2-1-2-2-1 2-1z"/>'),
  survey: svg('<path d="M12 21s-7-6.2-7-11a7 7 0 0 1 14 0c0 4.8-7 11-7 11z"/><path d="M9.5 10.5a3.5 3.5 0 0 1 5 0M11 12.5a1.5 1.5 0 0 1 2 0"/>'),
  compare: svg('<path d="M4 19V9M10 19V5M16 19v-7M22 19H2"/><path d="M4 9l6-4 6 7 5-5"/>'),
  metrics: svg('<rect x="3" y="3" width="8" height="8" rx="1.5"/><rect x="13" y="3" width="8" height="8" rx="1.5"/><rect x="3" y="13" width="8" height="8" rx="1.5"/><rect x="13" y="13" width="8" height="8" rx="1.5"/>'),
  report: svg('<path d="M7 3h7l5 5v13H7z"/><path d="M14 3v5h5M10 17v-3M13 17v-5M16 17v-2"/>'),
  portable: svg('<rect x="6" y="2" width="12" height="20" rx="3"/><path d="M10 6h4M12 11v6m0 0-2.5-2.5M12 17l2.5-2.5"/>'),
};

// ---- tiny template engine ----
// {{a.b}} escaped · {{{a.b}}} raw · {{> partial}}
// {{#a.b}}…{{/a.b}}: loop over arrays, enter objects, show if truthy
// {{^a.b}}…{{/a.b}}: show if falsy · {{.}} current array item
const esc = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const get = (ctx, path) =>
  path === "." ? ctx["."] : path.split(".").reduce((o, k) => (o == null ? undefined : o[k]), ctx);
const partials = Object.fromEntries(
  readdirSync(join(src, "partials")).map((f) => [f.replace(/\.html$/, ""), readFileSync(join(src, "partials", f), "utf8")]),
);
function render(tpl, ctx) {
  tpl = tpl.replace(/\{\{>\s*([\w-]+)\s*\}\}/g, (_, name) => {
    if (!(name in partials)) throw new Error(`missing partial: ${name}`);
    return partials[name];
  });
  tpl = tpl.replace(/\{\{([#^])([\w.]+)\}\}([\s\S]*?)\{\{\/\2\}\}/g, (_, kind, path, inner) => {
    const v = get(ctx, path);
    const truthy = Array.isArray(v) ? v.length > 0 : Boolean(v);
    if (kind === "^") return truthy ? "" : render(inner, ctx);
    if (Array.isArray(v)) {
      return v
        .map((item) => render(inner, item !== null && typeof item === "object" ? { ...ctx, ...item } : { ...ctx, ".": item }))
        .join("");
    }
    if (v && typeof v === "object") return render(inner, { ...ctx, ...v });
    return v ? render(inner, ctx) : "";
  });
  tpl = tpl.replace(/\{\{\{([\w.]+)\}\}\}/g, (_, p) => {
    const v = get(ctx, p);
    if (v === undefined) throw new Error(`missing key: ${p}`);
    return v;
  });
  return tpl.replace(/\{\{([\w.]+)\}\}/g, (_, p) => {
    const v = get(ctx, p);
    if (v === undefined) throw new Error(`missing key: ${p}`);
    return esc(v);
  });
}

// ---- pages ----
const langs = ["en", "es"];
const strings = Object.fromEntries(
  langs.map((l) => [l, JSON.parse(readFileSync(join(src, `${l}.json`), "utf8"))]),
);
const build = Date.now().toString(36);
const year = new Date().getFullYear();
const today = new Date().toISOString().slice(0, 10);
const contact = config.contact || "";

// Articles: src/articles.json lists them; the body of each is in
// src/articles/<key>.<lang>.html.
const articles = JSON.parse(readFileSync(join(src, "articles.json"), "utf8"));
const articleRoute = (a, lang) => `${strings[lang].routes.articles}${a.slug[lang]}/`;
const dateText = (iso, lang) =>
  new Intl.DateTimeFormat(lang === "en" ? "en-US" : "es-MX", { dateStyle: "long", timeZone: "UTC" }).format(
    new Date(`${iso}T00:00:00Z`),
  );

// Every page: key, template, route per language and per-page context.
const corePages = ["home", "guide", "privacy", "opensource", "articles"];
const pages = [
  ...corePages.map((key) => ({
    key,
    nav: key === "guide" ? "articles" : key,
    template: key,
    route: (lang) => strings[lang].routes[key],
    meta: (lang) => strings[lang].pages[key],
    lastmod: today,
  })),
  ...articles.map((a) => ({
    key: `article-${a.key}`,
    nav: "articles",
    template: "article",
    route: (lang) => articleRoute(a, lang),
    meta: (lang) => ({ title: `${a.title[lang]} — Ondalux Heatmap`, description: a.description[lang] }),
    article: a,
    lastmod: a.date,
  })),
];
const navKeys = ["home", "articles", "opensource", "privacy"];

const postsFor = (lang, except) =>
  articles
    .filter((a) => a.key !== except)
    .map((a) => ({
      href: articleRoute(a, lang),
      tag: a.tag[lang],
      title: a.title[lang],
      description: a.description[lang],
      minutes: a.minutes,
      dateText: dateText(a.date, lang),
    }));

const publisher = { "@type": "Organization", name: "Ondalux Heatmap", logo: `${domain}/assets/img/logo.png` };

for (const lang of langs) {
  const s = strings[lang];
  const otherLang = lang === "en" ? "es" : "en";
  for (const f of s.features.items) f.icon = icons[f.icon] || f.icon;
  for (const p of pages) {
    const tplFile = [join(src, "pages", `${p.template}.${lang}.html`), join(src, "pages", `${p.template}.html`)].find(
      existsSync,
    );
    const path = p.route(lang);
    const meta = p.meta(lang);
    const page = {
      key: p.key,
      path,
      title: meta.title,
      description: meta.description,
      en: p.route("en"),
      es: p.route("es"),
      other: p.route(otherLang),
    };
    const ads = Object.fromEntries(slotNames.map((n) => [n, adSlot(n, lang)]));
    const current = Object.fromEntries(navKeys.map((k) => [k, k === p.nav ? ' aria-current="page"' : ""]));
    let article = null;
    if (p.article) {
      const a = p.article;
      article = {
        title: a.title[lang],
        description: a.description[lang],
        tag: a.tag[lang],
        minutes: a.minutes,
        dateText: dateText(a.date, lang),
        body: readFileSync(join(src, "articles", `${a.key}.${lang}.html`), "utf8"),
      };
    }
    const ld = (obj) => `<script type="application/ld+json">${JSON.stringify({ "@context": "https://schema.org", ...obj })}</script>`;
    const jsonld =
      p.key === "home"
        ? ld({
            "@type": "SoftwareApplication",
            name: "Ondalux Heatmap",
            applicationCategory: "BusinessApplication",
            operatingSystem: "Windows 10, Windows 11",
            softwareVersion: version,
            description: page.description,
            url: domain + path,
            downloadUrl: `${domain}/downloads/${exe.name}`,
            image: `${domain}/assets/shots/${lang}/03-ap-selected.png`,
            license: "https://opensource.org/licenses/MIT",
            offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
          })
        : p.key === "guide" || p.article
          ? ld({
              "@type": "TechArticle",
              headline: p.article ? p.article.title[lang] : page.title,
              description: page.description,
              inLanguage: lang,
              datePublished: p.article ? p.article.date : undefined,
              url: domain + path,
              author: publisher,
              publisher,
            })
          : "";
    const contactHtml = contact
      ? lang === "en"
        ? `write to <a href="mailto:${contact}">${contact}</a>.`
        : `escríbenos a <a href="mailto:${contact}">${contact}</a>.`
      : lang === "en"
        ? "use the contact details on this website."
        : "usa los datos de contacto de este sitio.";
    const ctx = {
      ...s,
      lang,
      site: domain,
      home: s.routes.home,
      page,
      version,
      msrv,
      exe,
      build,
      year,
      updated: today,
      contact,
      contactHtml,
      github,
      adsEnabled,
      adsHead,
      ads,
      current,
      jsonld,
      article,
      posts: postsFor(lang, null),
      latest: postsFor(lang, null).slice(0, 3),
      related: p.article ? postsFor(lang, p.article.key).slice(0, 3) : [],
    };
    const html = render(readFileSync(tplFile, "utf8"), ctx);
    const out = join(pub, ...path.split("/").filter(Boolean), "index.html");
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, html);
    console.log(`wrote ${path}`);
  }
}

writeFileSync(
  join(pub, "sitemap.xml"),
  `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
${langs
  .flatMap((lang) =>
    pages.map(
      (p) => `  <url>
    <loc>${domain}${p.route(lang)}</loc>
    <lastmod>${p.lastmod}</lastmod>
    <xhtml:link rel="alternate" hreflang="en" href="${domain}${p.route("en")}"/>
    <xhtml:link rel="alternate" hreflang="es" href="${domain}${p.route("es")}"/>
  </url>`,
    ),
  )
  .join("\n")}
</urlset>
`,
);
console.log(`download: ${exe.name} (${exe.size}, sha256 ${exe.sha256})`);
console.log(`ads: ${adsEnabled ? `AdSense ${client}` : adsPreview ? "preview placeholders" : "off"} · github: ${github.url || "not set"}`);
