// Builds the static site into website/public:
//   - index.html (English) and es/index.html (Spanish) from src/template.html
//   - copies the Windows build (../dist/ondalux.exe) to public/downloads with
//     the version in its name, and puts its size and SHA-256 in the pages
//   - sitemap.xml
//
// Usage: node website/build.mjs        (no dependencies, Node 18+)

import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const pub = join(here, "public");
const site = "https://ondaluxheatmap.com";

// ---- version and download ----
const cargo = readFileSync(join(root, "Cargo.toml"), "utf8");
const version = /^version\s*=\s*"([^"]+)"/m.exec(cargo)[1];
const exeName = `ondalux-heatmap-${version}-windows-x64.exe`;
const exeSrc = join(root, "dist", "ondalux.exe");
const downloads = join(pub, "downloads");
mkdirSync(downloads, { recursive: true });
// Keep only the current build next to the sample reports.
for (const f of readdirSync(downloads)) {
  if (f.endsWith(".exe") && f !== exeName) rmSync(join(downloads, f));
}
copyFileSync(exeSrc, join(downloads, exeName));
const exeBytes = readFileSync(exeSrc);
const exe = {
  name: exeName,
  size: `${(statSync(exeSrc).size / 1024 / 1024).toFixed(1)} MB`,
  sha256: createHash("sha256").update(exeBytes).digest("hex"),
};

// ---- icons (simple stroke icons, 24×24) ----
const svg = (d) =>
  `<svg viewBox="0 0 24 24" aria-hidden="true">${d}</svg>`;
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

// ---- tiny template engine: {{a.b}} (escaped), {{{a.b}}} (raw),
// {{#a.b}}…{{/a.b}} (loop over arrays / show if truthy), {{.}} (item) ----
const esc = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const get = (ctx, path) => {
  if (path === ".") return ctx["."];
  return path.split(".").reduce((o, k) => (o == null ? undefined : o[k]), ctx);
};
function render(tpl, ctx) {
  tpl = tpl.replace(/\{\{#([\w.]+)\}\}([\s\S]*?)\{\{\/\1\}\}/g, (_, path, inner) => {
    const v = get(ctx, path);
    if (Array.isArray(v)) {
      return v
        .map((item) =>
          render(inner, typeof item === "object" ? { ...ctx, ...item } : { ...ctx, ".": item }),
        )
        .join("");
    }
    return v ? render(inner, ctx) : "";
  });
  tpl = tpl.replace(/\{\{\{([\w.]+)\}\}\}/g, (_, p) => get(ctx, p) ?? "");
  return tpl.replace(/\{\{([\w.]+)\}\}/g, (_, p) => {
    const v = get(ctx, p);
    if (v === undefined) throw new Error(`missing key: ${p}`);
    return esc(v);
  });
}

const template = readFileSync(join(here, "src", "template.html"), "utf8");
const build = Date.now().toString(36);
const year = new Date().getFullYear();
for (const lang of ["en", "es"]) {
  const strings = JSON.parse(readFileSync(join(here, "src", `${lang}.json`), "utf8"));
  for (const f of strings.features.items) f.icon = icons[f.icon];
  const html = render(template, { ...strings, site, version, exe, build, year });
  const out = lang === "en" ? join(pub, "index.html") : join(pub, lang, "index.html");
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, html);
  console.log(`wrote ${out}`);
}

const today = new Date().toISOString().slice(0, 10);
writeFileSync(
  join(pub, "sitemap.xml"),
  `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
${["/", "/es/"]
  .map(
    (p) => `  <url>
    <loc>${site}${p}</loc>
    <lastmod>${today}</lastmod>
    <xhtml:link rel="alternate" hreflang="en" href="${site}/"/>
    <xhtml:link rel="alternate" hreflang="es" href="${site}/es/"/>
  </url>`,
  )
  .join("\n")}
</urlset>
`,
);
console.log(`download: ${exe.name} (${exe.size}, sha256 ${exe.sha256})`);
