/* Chrona demo — core: API, formatting, theme, nav, Today + Stats views.
   Data comes from the real chronad daemon via /api/* (server.py bridges
   the Unix socket). Design: original Ditto look. */

"use strict";

const $ = (id) => document.getElementById(id);
const state = {
  online: false, status: null, day: null, week: null, month: null,
  goals: [], theme: "material", cycle: 0,
  expanded: "", titleCache: {}, page: "today",
  notifFlag: {}, bannerHiddenUntil: 0,
};

/* ---------- formatting ---------- */

function fmtDur(secs) {
  secs = Math.max(0, secs | 0);
  const h = Math.floor(secs / 3600), m = Math.floor((secs % 3600) / 60);
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  if (m > 0) return `${m}m`;
  return `${secs}s`;
}

function prettyName(id) {
  let base = id;
  if (base.includes(".")) {
    const parts = base.split(".");
    base = (parts.length > 2 && ["org", "net", "com", "io", "im"].includes(parts[0]) && parts[1])
      ? parts[1] : parts[parts.length - 1];
  }
  if (base.includes(":")) base = base.split(":").pop(); // "firefox:netflix" -> "netflix"
  return base.replace(/[-_]/g, " ")
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

function chronaLabel(key) {
  return {
    work: "Work & Coding", browsers: "Browsers", communication: "Communication",
    media: "Media & Streaming", creative: "Creative & Design", gaming: "Games",
    system: "System & Files", uncategorised: "Uncategorised",
  }[key] || "Uncategorised";
}

function catColor(key) {
  return getComputedStyle(document.documentElement).getPropertyValue(`--cat-${key}`)?.trim() ||
    getComputedStyle(document.documentElement).getPropertyValue("--cat-uncategorised").trim();
}

function appColor(appId) {
  // stable pastel-ish tile color from the app id (deterministic)
  let h = 0;
  for (const ch of appId) h = (h * 31 + ch.charCodeAt(0)) % 360;
  return `hsl(${h} 45% 45%)`;
}

const hhmm = (ts) => new Date(ts * 1000).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });

function goalLabel(g) {
  return g.kind === "total" ? "Screen time"
    : g.kind === "category" ? chronaLabel(g.key) : prettyName(g.key);
}

function goalIcon(g) {
  if (g.kind === "total") {
    return `<span class="ic total-ic"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/></svg></span>`;
  }
  return g.kind === "category"
    ? `<span class="ic" style="background:${catColor(g.key)}">${esc(goalLabel(g).slice(0, 1))}</span>`
    : renderAppIcon(g.key);
}

function overGoals() {
  return state.goals.filter((g) => g.enabled && g.limit_seconds > 0 && g.used_seconds > g.limit_seconds);
}

/* ---------- real app icons (bundled, offline) + PWA recognition ---------- */

const APP_ICONS = {
  firefox: "firefox.svg", "firefox:netflix": "netflix.svg", code: "code.svg",
  konsole: "konsole.png", kate: "kate.svg", dolphin: "dolphin.svg",
  discord: "discord.svg", "telegram-desktop": "telegram.svg", spotify: "spotify.svg",
  steam: "steam.svg", obs: "obs.svg", krita: "krita.svg", figma: "figma.svg",
  brave: "brave.svg", systemsettings: "systemsettings.svg",
};
// near-black brand marks — inverted for legibility in dark theme (CSS)
const ICON_INVERT_DARK = new Set(["steam.svg", "notion.svg", "obs.svg"]);
// PWAs run inside a browser but get their own window + icon; matched by title
const PWA_ICONS = {
  "youtube music": "youtubemusic.svg", notion: "notion.svg", figma: "figma.svg",
};

function renderAppIcon(appId) {
  const file = APP_ICONS[appId];
  if (file) {
    const inv = ICON_INVERT_DARK.has(file) ? " inv" : "";
    return `<span class="ic icon"><img class="appicon${inv}" src="/icons/${file}" alt="" draggable="false"></span>`;
  }
  return `<span class="ic" style="background:${appColor(appId)}">${esc(prettyName(appId).slice(0, 1) || "A")}</span>`;
}

function pwaFor(title) {
  const key = String(title || "").split(" — ")[0].trim().toLowerCase();
  const file = PWA_ICONS[key];
  return file ? { file, inv: ICON_INVERT_DARK.has(file) } : null;
}

function ringPoint(deg) {
  const rad = (deg * Math.PI) / 180;
  return [50 + 45 * Math.sin(rad), 50 - 45 * Math.cos(rad)];
}
function arcCmd(startDeg, sweepDeg) {
  const sweep = Math.min(359.9, Math.max(0, sweepDeg));
  if (sweep <= 0.05) return "";
  const [x0, y0] = ringPoint(startDeg);
  const [x1, y1] = ringPoint(startDeg + sweep);
  const large = sweep > 180 ? 1 : 0;
  return `M ${x0.toFixed(2)} ${y0.toFixed(2)} A 45 45 0 ${large} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;
}

const i64 = (v, k) => (v && v[k]) || 0;
const arr = (v, k) => (v && v[k]) || [];
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

function toast(msg) {
  const t = $("toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.remove("show"), 2400);
}

/* ---------- API ---------- */

async function api(cmd, args = {}) {
  const qs = Object.entries(args).map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`).join("&");
  const res = await fetch(`/api/${cmd}${qs ? "?" + qs : ""}`);
  const j = await res.json();
  return j.ok ? j.data : null;
}
async function apiPost(cmd, args = {}) {
  const res = await fetch(`/api/${cmd}`, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(args),
  });
  return res.json();
}

/* ---------- theme (Settings-only switch) ---------- */

async function applyTheme(theme, persist) {
  state.theme = theme === "dark" ? "dark" : "material";
  document.documentElement.dataset.theme = state.theme;
  const sw = $("themeSwitch");
  if (sw) sw.classList.toggle("on", state.theme === "dark");
  localStorage.setItem("chrona-theme", state.theme);
  if (persist) await apiPost("settings.set", { key: "theme", value: state.theme });
  renderToday(); renderStats();   // re-render charts with baked colors
}

/* ---------- navigation ---------- */

const navMql = window.matchMedia("(max-width: 860px)");
let navCollapsed = localStorage.getItem("chrona-nav") === "collapsed";
let drawerOpen = false;

function applyNav() {
  const rail = $("rail"), scrim = $("scrim"), btn = $("navToggle");
  if (navMql.matches) {
    rail.classList.remove("collapsed");
    rail.classList.toggle("open", drawerOpen);
    scrim.classList.toggle("show", drawerOpen);
    btn.setAttribute("aria-expanded", String(drawerOpen));
  } else {
    drawerOpen = false;
    rail.classList.remove("open");
    scrim.classList.remove("show");
    rail.classList.toggle("collapsed", navCollapsed);
    btn.setAttribute("aria-expanded", "true");
  }
}

function setPage(page) {
  state.page = page;
  state.expanded = "";
  document.querySelectorAll(".nav-item").forEach((x) =>
    x.classList.toggle("active", x.dataset.page === page));
  $("gearBtn").classList.toggle("active", page === "settings");
  document.querySelectorAll(".page").forEach((p) => p.classList.add("hidden"));
  $(`page-${page}`).classList.remove("hidden");
  if (drawerOpen) { drawerOpen = false; applyNav(); }
  window.scrollTo({ top: 0 });
  tick();
}

/* ---------- Today ---------- */

function appRowHTML(a, share, timeText, open) {
  return `<div class="appitem" data-app="${esc(a.app_id)}">
    <div class="approw ${open ? "open" : ""}">
      ${renderAppIcon(a.app_id)}
      <span class="name" title="${esc(a.app_id)}">${esc(prettyName(a.app_id))}</span>
      <span class="bar"><i style="width:${(Math.min(1, Math.max(0, share)) * 100).toFixed(1)}%"></i></span>
      <span class="time">${esc(timeText)}</span>
      <span class="chev"><svg viewBox="0 0 24 24"><path d="M6 9l6 6 6-6"/></svg></span>
    </div>
    <div class="winlist ${open ? "openv" : ""}"><div class="winlist-inner"></div></div>
  </div>`;
}

function renderToday() {
  const d = state.day;
  if (!d) return;
  const total = i64(d, "total_seconds");
  $("todayTotal").textContent = fmtDur(total);
  $("todayUnlocks").textContent = i64(d, "unlocks");
  $("todayLongest").textContent = fmtDur(i64(d, "longest_session"));
  $("todayAway").textContent = fmtDur(i64(d, "afk_seconds"));

  // Ring: today vs usual day (prev-week daily average), else 8h budget
  let ratio = 0;
  const w = state.week;
  if (w && i64(w, "prev_total_seconds") > 0) {
    ratio = total / Math.max(1, Math.floor(i64(w, "prev_total_seconds") / 7));
  } else {
    ratio = total / (8 * 3600);
  }
  $("ringArc").setAttribute("d", arcCmd(0, Math.min(1, Math.max(0, ratio)) * 360));

  // top-category badge
  const cats = arr(d, "categories");
  const topCat = cats.reduce((a, b) => (!a || b.seconds > a.seconds ? b : a), null);
  $("todayTopCat").innerHTML = topCat
    ? `mostly <b>${esc(topCat.label)}</b> · ${Math.round((topCat.seconds / Math.max(1, total)) * 100)}%`
    : "";

  // app list (expandable)
  const apps = arr(d, "apps").slice(0, 8);
  $("todayAppsEmpty").classList.toggle("hidden", apps.length > 0);
  const max = apps.reduce((m, a) => Math.max(m, a.seconds), 0);
  $("todayApps").innerHTML = apps.map((a) =>
    appRowHTML(a, max > 0 ? a.seconds / max : 0, fmtDur(a.seconds), a.app_id === state.expanded)).join("");

  renderHourly();
  renderStrip(d);
  renderBanner();
}

/* Day timeline strip: one colored segment per active span, Digital
   Wellbeing-style. Positions are percentages of the tracked day. */
function renderStrip(d) {
  const el = $("dayStrip");
  if (!el) return;
  const segs = arr(d, "timeline").filter((s) => s.end > s.start);
  const from = i64(d, "from"), span = Math.max(1, i64(d, "to") - from);
  el.innerHTML = segs.map((s) => {
    const l = ((s.start - from) / span) * 100;
    const w = Math.max(0.18, ((s.end - s.start) / span) * 100);
    const app = s.app_id || "?";
    return `<i style="left:${l.toFixed(2)}%;width:${w.toFixed(2)}%;background:${appColor(app)}"
      title="${hhmm(s.start)} – ${hhmm(s.end)} · ${esc(prettyName(app))} · ${fmtDur(s.end - s.start)}"></i>`;
  }).join("");
}

/* Over-limit banner + browser notifications (once per goal until it
   drops back under the limit). */
function renderBanner() {
  const el = $("limitBanner");
  if (!el) return;
  const over = overGoals();
  if (!over.length || Date.now() < state.bannerHiddenUntil) {
    el.classList.add("hidden");
    return;
  }
  el.classList.remove("hidden");
  el.innerHTML = `<div class="banner-head">
      <svg viewBox="0 0 24 24" class="banner-ic"><path d="M12 9v4m0 4h.01M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/></svg>
      <span>Over daily limit</span>
      <button class="banner-x" id="bannerDismiss" aria-label="Dismiss" title="Dismiss">
        <svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18"/></svg>
      </button>
    </div>` +
    over.slice(0, 3).map((g) => `<div class="banner-row">${goalIcon(g)}
      <span>${esc(goalLabel(g))} — ${fmtDur(g.used_seconds)} of ${fmtDur(g.limit_seconds)}</span></div>`).join("");
}

function checkLimitNotifications() {
  if (!("Notification" in window) || Notification.permission !== "granted") return;
  if (localStorage.getItem("chrona-notify") !== "1") return;
  for (const g of state.goals) {
    const over = g.enabled && g.limit_seconds > 0 && g.used_seconds > g.limit_seconds;
    if (over && !state.notifFlag[g.id]) {
      new Notification("Time limit reached", {
        body: `${goalLabel(g)} — ${fmtDur(g.used_seconds)} of ${fmtDur(g.limit_seconds)} today.`,
      });
    }
    state.notifFlag[g.id] = over;
  }
}

function renderHourly() {
  const hours = arr(state.day, "hourly");
  const has = hours.some((v) => v > 0);
  $("hourlyEmpty").classList.toggle("hidden", has);
  const max = hours.reduce((m, v) => Math.max(m, v), 0);
  $("hours").innerHTML = hours.map((v, h) =>
    `<div class="hbar" title="${String(h).padStart(2, "0")}:00 — ${fmtDur(v)}"><i style="height:${max > 0 ? Math.max(3, (v / max) * 100) : 0}%"></i></div>`).join("");
  $("hourLabels").innerHTML = hours.map((_, h) =>
    `<span>${h % 3 === 0 ? String(h).padStart(2, "0") : ""}</span>`).join("");
  const peak = hours.indexOf(max);
  $("todayPeak").textContent = has && peak >= 0 ? `peak ${String(peak).padStart(2, "0")}:00` : "";
}

async function toggleApp(item) {
  const appId = item.dataset.app;
  const winlist = item.querySelector(".winlist");
  const inner = winlist.querySelector(".winlist-inner");
  const opening = appId !== state.expanded;
  state.expanded = opening ? appId : "";
  document.querySelectorAll(".appitem").forEach((it) => {
    it.querySelector(".approw").classList.toggle("open", it.dataset.app === state.expanded);
    it.querySelector(".winlist").classList.toggle("openv", it.dataset.app === state.expanded);
  });
  if (!opening) return;
  inner.innerHTML = `<div class="winrow"><span class="t" style="color:var(--dim)">loading windows…</span></div>`;
  let titles = state.titleCache[appId];
  if (!titles) {
    const data = await api("app", { app_id: appId, days: 1 });
    titles = data ? arr(data, "titles") : [];
    state.titleCache[appId] = titles;
  }
  if (appId !== state.expanded) return;
  inner.innerHTML = titles.length
    ? titles.slice(0, 5).map((t) => {
        const pwa = pwaFor(t.title);
        const ico = pwa
          ? `<img class="winicon${pwa.inv ? " inv" : ""}" src="/icons/${pwa.file}" alt="">` : "";
        const badge = pwa ? ` <span class="pwa-badge">PWA</span>` : "";
        return `<div class="winrow">${ico}<span class="t" title="${esc(t.title)}">${esc(t.title || "(untitled window)")}${badge}</span>
        <span class="m">${fmtDur(t.seconds)} · ${t.sessions}×</span></div>`;
      }).join("")
    : `<div class="winrow"><span class="t" style="color:var(--dim)">no titled windows today</span></div>`;
}

async function renderNowLine() {
  let line = "";
  if (state.status && state.status.current_window) {
    const cw = state.status.current_window;
    line = `Right now: ${prettyName(cw.app_id || "")} — ${cw.title || ""}`;
  } else {
    const d = await api("settings.get", { key: "demo.current_window" });
    const v = d && d.value;
    if (v) line = v.startsWith("AFK") ? "Right now: AFK" : `Right now: ${prettyName(v.split(" — ")[0])} — ${v.split(" — ").slice(1).join(" — ")}`;
  }
  $("nowLine").textContent = line;
}

/* ---------- Stats ---------- */

function weekCats(week) {
  const sum = {};
  for (const day of arr(week, "days"))
    for (const c of day.categories || []) sum[c.key] = (sum[c.key] || 0) + c.seconds;
  return Object.entries(sum).map(([key, seconds]) => ({ key, seconds, label: chronaLabel(key) }))
    .sort((a, b) => b.seconds - a.seconds);
}

function renderStats() {
  const w = state.week, m = state.month;
  if (!w) return;

  // week header
  const total = i64(w, "total_seconds"), prev = i64(w, "prev_total_seconds");
  $("weekTotal").textContent = fmtDur(total);
  $("weekAvg").textContent = `${fmtDur(Math.floor(total / 7))} average per day`;
  const delta = total - prev;
  if (prev > 0) {
    $("weekDelta").textContent = `${delta >= 0 ? "+" : ""}${fmtDur(Math.abs(delta))} vs last week`;
    $("weekDelta").className = "sub-line " + (delta > 0 ? "bad" : "good");
  } else {
    $("weekDelta").textContent = "first tracked week";
    $("weekDelta").className = "sub-line";
  }

  // category donut (week)
  const cats = weekCats(w);
  const totalC = cats.reduce((s, c) => s + c.seconds, 0) || 1;
  let start = 0, arcs = "";
  for (const c of cats) {
    const share = c.seconds / totalC;
    const cmd = arcCmd(start, share * 360);
    if (cmd) arcs += `<path d="${cmd}" fill="none" stroke="${catColor(c.key)}" stroke-width="11"/>`;
    start += share * 360;
  }
  $("donutArcs").innerHTML = arcs;

  // insights
  const days = arr(w, "days");
  const busiest = days.reduce((a, b) => (!a || b.seconds > a.seconds ? b : a), null);
  const topApp = arr(w, "apps")[0];
  const todayISO = new Date().toISOString().slice(0, 10);
  const yest = days.filter((x) => x.date !== todayISO).reduce((a, b) => (!a || b.seconds > a.seconds ? b : a), null);
  const ins = [];
  if (busiest) {
    const lbl = new Date(busiest.date + "T12:00:00").toLocaleDateString("en-GB", { weekday: "long" });
    ins.push(`<b>${lbl}</b> was your heaviest day at <b>${fmtDur(busiest.seconds)}</b>.`);
  }
  if (topApp) ins.push(`<b>${esc(prettyName(topApp.app_id))}</b> leads the week with <b>${fmtDur(topApp.seconds)}</b>.`);
  if (cats[0]) ins.push(`<b>${esc(cats[0].label)}</b> took <b>${Math.round((cats[0].seconds / totalC) * 100)}%</b> of your screen time.`);
  if (prev > 0) ins.push(delta > 0
    ? `Up <b>${fmtDur(delta)}</b> vs last week.`
    : `Down <b>${fmtDur(-delta)}</b> vs last week.`);
  $("insights").innerHTML = ins.map((t) => `<div class="insight"><span class="dot"></span><span class="tx">${t}</span></div>`).join("");

  // per-day stacked columns
  $("weekEmpty").classList.toggle("hidden", days.length > 0);
  const max = days.reduce((mx, x) => Math.max(mx, x.seconds), 0);
  $("weekCols").innerHTML = days.map((day) => {
    const dcs = day.categories || [];
    const tot = dcs.reduce((s, c) => s + c.seconds, 0);
    let y = 0, segs = "";
    for (const c of dcs) {
      const v = max > 0 ? c.seconds / max : 0;
      segs += `<i style="bottom:${(y * 100).toFixed(1)}%;height:${(v * 100).toFixed(1)}%;background:${catColor(c.key)}"></i>`;
      y += v;
    }
    const lbl = new Date(day.date + "T12:00:00").toLocaleDateString("en-GB", { weekday: "short", day: "2-digit" });
    return `<div class="col ${day.date === todayISO ? "today" : ""}" title="${esc(day.date)}: ${fmtDur(day.seconds)}">
      <div class="stack">${segs}</div>
      <div class="dlabel">${esc(lbl)}</div><div class="dtime">${fmtDur(day.seconds)}</div></div>`;
  }).join("");

  // usage calendar (month)
  if (m) {
    $("monthLabel").textContent = `Usage calendar — ${m.label || "this month"}`;
    const all = arr(m, "days");
    const mmax = all.reduce((mx, x) => Math.max(mx, x.seconds), 0);
    const byDate = new Map(all.map((x) => [x.date, mmax > 0 ? x.seconds / mmax : 0]));
    const [r, g, b] = (() => {
      const v = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
      const mm = v.match(/^#(..)(..)(..)$/);
      return mm ? [parseInt(mm[1], 16), parseInt(mm[2], 16), parseInt(mm[3], 16)] : [13, 148, 136];
    })();
    let cols = "", col = "";
    const wd = (ds) => ((new Date(ds + "T12:00:00").getDay() + 6) % 7) + 1;
    const first = [...byDate.keys()][0];
    if (first) for (let i = 1; i < wd(first); i++) col += `<div class="heat-cell" style="background:transparent"></div>`;
    for (const [date, v] of byDate) {
      const alpha = 0.12 + 0.88 * Math.min(1, v);
      col += `<div class="heat-cell ${date === todayISO ? "today" : ""}" title="${esc(date)}: ${fmtDur(v * mmax)}"
        style="${v > 0 ? `background:rgba(${r},${g},${b},${alpha.toFixed(2)})` : ""}"></div>`;
      if (col.split("</div>").length - 1 === 7) { cols += `<div class="heat-col">${col}</div>`; col = ""; }
    }
    if (col) cols += `<div class="heat-col">${col}</div>`;
    $("heatWeeks").innerHTML = cols;
  }

  // apps: week + month merged rows
  const wk = new Map(arr(w, "apps").map((a) => [a.app_id, fmtDur(a.seconds)]));
  const mo = new Map(m ? arr(m, "apps").map((a) => [a.app_id, fmtDur(a.seconds)]) : []);
  const merged = (m ? arr(m, "apps") : arr(w, "apps")).slice(0, 8);
  $("statAppsEmpty").classList.toggle("hidden", merged.length > 0);
  const mx = merged.reduce((mm, a) => Math.max(mm, a.seconds), 0);
  $("statApps").innerHTML = merged.map((a) => `<div class="approw" style="cursor:default">
      ${renderAppIcon(a.app_id)}
      <span class="name" title="${esc(a.app_id)}">${esc(prettyName(a.app_id))}</span>
      <span class="bar"><i style="width:${(mx > 0 ? a.seconds / mx : 0) * 100}%"></i></span>
      <span class="time">${wk.get(a.app_id) || "—"} · ${mo.get(a.app_id) || "—"}</span>
    </div>`).join("");
}

/* ---------- status / data cycle ---------- */

function renderStatus() {
  const ok = state.online && state.status;
  const paused = !!(ok && state.status.paused);
  $("recDot").classList.toggle("on", ok && !paused);
  $("recDot").classList.toggle("paused", paused);
  $("recText").textContent = paused ? "paused" : ok ? "recording" : "daemon offline";
  $("recChip").title = paused ? "tracking paused — resume in Settings"
    : ok ? `watcher: ${state.status.watcher}` : "start it: systemctl --user start chrona";
  $("topDate").textContent = new Date().toLocaleDateString("en-GB",
    { weekday: "long", day: "numeric", month: "long" });
}

async function tick() {
  state.cycle++;
  try {
    const s = await api("status");
    state.status = s; state.online = !!s;
  } catch { state.online = false; }
  renderStatus();

  const jobs = [api("day"), api("week")];
  if (state.page === "stats" || state.cycle % 6 === 0) jobs.push(api("month"));
  if (state.page === "today" || state.page === "timers") {
    jobs.push(api("goals").then((g) => { if (g) { state.goals = g; checkLimitNotifications(); } }));
  }
  const [day, week, month] = await Promise.all(jobs);
  if (day) state.day = day;
  if (week) state.week = week;
  if (month) state.month = month;

  if (state.page === "today") { renderToday(); renderNowLine(); }
  else if (state.page === "stats") renderStats();
  else if (state.page === "timers") refreshGoals();
  else if (state.page === "focus") renderFocusLog();
  else if (state.page === "bedtime") renderBedtime();
  else if (state.page === "settings") renderSettings();
}
