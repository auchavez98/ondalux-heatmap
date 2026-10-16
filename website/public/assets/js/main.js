// Ondalux Heatmap website: small, dependency-free interactions.
(() => {
  "use strict";
  const doc = document.documentElement;
  doc.classList.add("js");
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));

  // ---- nav: background on scroll, mobile menu ----
  const nav = $(".nav");
  const onScroll = () => nav.classList.toggle("is-scrolled", window.scrollY > 10);
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });
  const toggle = $(".nav__toggle");
  toggle.addEventListener("click", () => {
    const open = nav.classList.toggle("is-open");
    toggle.setAttribute("aria-expanded", String(open));
  });
  $$(".nav__links a").forEach((a) =>
    a.addEventListener("click", () => {
      nav.classList.remove("is-open");
      toggle.setAttribute("aria-expanded", "false");
    }),
  );

  // ---- reveal on scroll (staggered within each parent) ----
  const groups = new Map();
  $$(".reveal").forEach((el) => {
    const n = groups.get(el.parentElement) || 0;
    groups.set(el.parentElement, n + 1);
    el.style.setProperty("--d", `${Math.min(n, 8) * 0.07}s`);
  });
  const counters = new Set($$("[data-count]"));
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        e.target.classList.add("is-in");
        $$("[data-count]", e.target).concat(e.target.matches("[data-count]") ? [e.target] : []).forEach(count);
        io.unobserve(e.target);
      }
    },
    { threshold: 0.12, rootMargin: "0px 0px -40px 0px" },
  );
  $$(".reveal").forEach((el) => io.observe(el));

  // ---- counters ----
  function count(el) {
    if (!counters.has(el)) return;
    counters.delete(el);
    const target = Number(el.dataset.count);
    const suffix = el.dataset.suffix || "";
    if (reduced || !Number.isFinite(target)) return;
    const start = performance.now();
    const dur = 1400;
    const step = (t) => {
      const k = Math.min(1, (t - start) / dur);
      const eased = 1 - Math.pow(1 - k, 3);
      el.textContent = `${Math.round(target * eased)}${suffix}`;
      if (k < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  // ---- 3D tilt of the hero screenshot ----
  const tilt = $(".tilt");
  if (tilt && !reduced && window.matchMedia("(hover: hover)").matches) {
    const box = tilt.parentElement;
    box.addEventListener("pointermove", (e) => {
      const r = box.getBoundingClientRect();
      const x = (e.clientX - r.left) / r.width - 0.5;
      const y = (e.clientY - r.top) / r.height - 0.5;
      tilt.style.transform = `rotateY(${-9 + x * 10}deg) rotateX(${5 - y * 8}deg)`;
    });
    box.addEventListener("pointerleave", () => (tilt.style.transform = ""));
  }

  // ---- spotlight on feature cards ----
  $$(".card").forEach((card) =>
    card.addEventListener("pointermove", (e) => {
      const r = card.getBoundingClientRect();
      card.style.setProperty("--mx", `${e.clientX - r.left}px`);
      card.style.setProperty("--my", `${e.clientY - r.top}px`);
    }),
  );

  // ---- screenshot gallery ----
  const tabs = $$(".tab");
  const img = $(".gallery__img");
  const caption = $(".gallery__caption");
  const select = (tab, animate) => {
    tabs.forEach((t) => t.setAttribute("aria-selected", String(t === tab)));
    caption.textContent = tab.dataset.caption;
    const src = img.dataset.base + tab.dataset.shot;
    if (img.getAttribute("src") === src) return;
    if (!animate || reduced) {
      img.src = src;
      return;
    }
    img.classList.add("is-swapping");
    const next = new Image();
    next.onload = () => {
      setTimeout(() => {
        img.src = src;
        img.classList.remove("is-swapping");
      }, 180);
    };
    next.src = src;
  };
  if (tabs.length) {
    tabs.forEach((t) => t.addEventListener("click", () => select(t, true)));
    select(tabs[0], false);
  }

  // ---- new version announcement: once per version, not on the changelog ----
  const pop = $(".release-pop");
  if (pop) {
    const key = "ondalux-release-seen";
    const version = pop.dataset.version;
    const onChangelog = /\/(changelog|novedades)\//.test(location.pathname);
    let seen = null;
    try { seen = localStorage.getItem(key); } catch (_) { /* storage unavailable */ }
    if (seen !== version && !onChangelog) {
      const remember = () => {
        pop.hidden = true;
        try { localStorage.setItem(key, version); } catch (_) { /* ignore */ }
      };
      setTimeout(() => { pop.hidden = false; }, 2500);
      $(".release-pop__close", pop).addEventListener("click", remember);
      $("a", pop).forEach((a) => a.addEventListener("click", remember));
      document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !pop.hidden) remember(); });
    }
  }

  // ---- Google AdSense: fill each ad slot; reopen the consent message ----
  if ($("ins.adsbygoogle")) {
    $$("ins.adsbygoogle").forEach(() => {
      try { (window.adsbygoogle = window.adsbygoogle || []).push({}); } catch (_) { /* blocked */ }
    });
  }
  $$("[data-consent]").forEach((a) =>
    a.addEventListener("click", (e) => {
      e.preventDefault();
      // Google's consent management (Privacy & messaging in AdSense).
      window.googlefc = window.googlefc || {};
      window.googlefc.callbackQueue = window.googlefc.callbackQueue || [];
      window.googlefc.callbackQueue.push(() => window.googlefc.showRevocationMessage());
    }),
  );

  // ---- language suggestion ----
  const hint = $(".lang-hint");
  try {
    const pageLang = doc.lang;
    const wants = (navigator.languages || [navigator.language || ""]).map((l) => l.slice(0, 2));
    const other = pageLang === "en" ? "es" : "en";
    if (wants[0] === other && !localStorage.getItem("ondalux-lang-hint")) {
      hint.hidden = false;
      $("button", hint).addEventListener("click", () => {
        hint.hidden = true;
        try { localStorage.setItem("ondalux-lang-hint", "1"); } catch (_) { /* ignore */ }
      });
    }
  } catch (_) { /* storage unavailable: no hint */ }

  // ---- animated heat map behind the hero ----
  const canvas = $(".hero__heat");
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  const W = 160, H = 90;
  canvas.width = W;
  canvas.height = H;
  const image = ctx.createImageData(W, H);
  // Color ramp like the app: strong (green) → weak (red).
  const stops = [
    [-45, [34, 197, 94]],
    [-60, [132, 204, 22]],
    [-67, [250, 204, 21]],
    [-75, [249, 115, 22]],
    [-85, [239, 68, 68]],
  ];
  const ramp = (v) => {
    if (v >= stops[0][0]) return stops[0][1];
    for (let i = 1; i < stops.length; i++) {
      const [v1, c1] = stops[i];
      const [v0, c0] = stops[i - 1];
      if (v >= v1) {
        const t = (v0 - v) / (v0 - v1);
        return c0.map((c, k) => c + (c1[k] - c) * t);
      }
    }
    return null;
  };
  const aps = [
    { x: 0.72, y: 0.3, ax: 0.12, ay: 0.1, s: 0.00021, p: 0 },
    { x: 0.88, y: 0.72, ax: 0.08, ay: 0.12, s: 0.00017, p: 2 },
    { x: 0.55, y: 0.78, ax: 0.1, ay: 0.08, s: 0.00025, p: 4 },
    { x: 0.2, y: 0.25, ax: 0.1, ay: 0.08, s: 0.00019, p: 1 },
    { x: 0.35, y: 0.7, ax: 0.07, ay: 0.1, s: 0.00023, p: 3 },
  ];
  let visible = true;
  new IntersectionObserver((e) => (visible = e[0].isIntersecting)).observe(canvas);
  const draw = (t) => {
    const pos = aps.map((a) => [
      (a.x + Math.sin(t * a.s + a.p) * a.ax) * W,
      (a.y + Math.cos(t * a.s * 1.3 + a.p) * a.ay) * H,
    ]);
    const d = image.data;
    for (let y = 0; y < H; y++) {
      for (let x = 0; x < W; x++) {
        let best = -200;
        for (const [px, py] of pos) {
          const dist = Math.hypot((x - px) * 0.35, (y - py) * 0.35) + 1;
          const v = -32 - 30 * Math.log10(dist);
          if (v > best) best = v;
        }
        const i = (y * W + x) * 4;
        const c = ramp(best);
        if (c) {
          d[i] = c[0];
          d[i + 1] = c[1];
          d[i + 2] = c[2];
          d[i + 3] = 255;
        } else {
          d[i + 3] = 0;
        }
      }
    }
    ctx.putImageData(image, 0, 0);
  };
  if (reduced) {
    draw(0);
    return;
  }
  let last = 0;
  const loop = (t) => {
    if (visible && !document.hidden && t - last > 33) {
      draw(t);
      last = t;
    }
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
})();
