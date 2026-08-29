/* Chrona demo — extras: Timers (daemon goals), Focus sessions, Bedtime,
   Settings, event wiring + boot. Client-side features persist locally. */

"use strict";

/* ---------- Timers (daily limits on the daemon) ---------- */

async function refreshGoals() {
  const g = await api("goals");
  if (g) state.goals = g;
  renderGoals();
}

function renderGoals() {
  const el = $("goalList");
  if (!el) return;
  $("goalsEmpty").classList.toggle("hidden", state.goals.length > 0);
  // Screen-time (total) goal first — it is the headline limit.
  const goals = [...state.goals].sort((a, b) => (b.kind === "total") - (a.kind === "total"));
  el.innerHTML = goals.map((g) => {
    const label = goalLabel(g);
    const progress = g.limit_seconds > 0 ? Math.min(1, g.used_seconds / g.limit_seconds) : 0;
    const exceeded = g.used_seconds > g.limit_seconds && g.limit_seconds > 0;
    const ic = goalIcon(g);
    return `<div class="goal-row ${g.enabled ? "" : "disabled"}">
      ${ic}
      <div class="goal-main">
        <div class="goal-label-row">
          <span class="goal-label">${esc(label)}</span>
          ${exceeded ? '<span class="badge-over">over limit</span>' : ""}
        </div>
        <div class="hbar-line ${exceeded ? "over" : ""}"><i style="width:${(progress * 100).toFixed(1)}%"></i></div>
        <div class="goal-used">${fmtDur(g.used_seconds)} of ${fmtDur(g.limit_seconds)} today</div>
      </div>
      <div class="switch ${g.enabled ? "on" : ""}" data-goal-toggle="${g.id}" role="switch" tabindex="0"></div>
      <button class="btn text" data-goal-del="${g.id}">Remove</button>
    </div>`;
  }).join("");
  el.querySelectorAll("[data-goal-del]").forEach((b) =>
    b.addEventListener("click", async () => { await apiPost("goal.del", { id: +b.dataset.goalDel }); await refreshGoals(); }));
  el.querySelectorAll("[data-goal-toggle]").forEach((s) =>
    s.addEventListener("click", async () => {
      const g = state.goals.find((x) => x.id === +s.dataset.goalToggle);
      if (!g) return;
      await apiPost("goal.set", { kind: g.kind, key: g.key, limit_seconds: g.limit_seconds, enabled: !g.enabled });
      await refreshGoals();
    }));
}

function fillGoalSuggestions() {
  const kind = $("goalKind").value;
  $("goalKey").style.display = kind === "total" ? "none" : "";
  const opts = kind === "total" ? ["total"]
    : kind === "category"
    ? ["work", "browsers", "communication", "media", "creative", "gaming", "system"]
    : (() => {
      const ids = new Set([
        ...arr(state.day, "apps"), ...arr(state.week, "apps"), ...arr(state.month, "apps"),
      ].map((a) => a.app_id));
      return [...ids].slice(0, 14);
    })();
  $("goalKey").innerHTML = opts.map((o) => `<option value="${esc(o)}">${esc(prettyName(o))}</option>`).join("");
}

/* ---------- Focus (client-side timer + log in localStorage) ---------- */

const FOCUS_KEY = "chrona-focus-state";
const FOCUS_LOG = "chrona-focus-log";

const focus = {
  duration: 25 * 60, remaining: 25 * 60, running: false, paused: false, timer: null, log: [],
};

function focusLoad() {
  try { focus.log = JSON.parse(localStorage.getItem(FOCUS_LOG) || "[]"); } catch { focus.log = []; }
}

function focusSaveLog() {
  localStorage.setItem(FOCUS_LOG, JSON.stringify(focus.log.slice(0, 60)));
}

function fmtClock(secs) {
  const m = Math.floor(secs / 60), s = secs % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function renderFocus() {
  const clock = $("focusClock"), st = $("focusState");
  if (!clock) return;
  clock.textContent = fmtClock(focus.running || focus.paused ? focus.remaining : focus.duration);
  $("focusArc").setAttribute("d",
    arcCmd(0, (focus.duration > 0 ? 1 - focus.remaining / focus.duration : 0) * 360));
  st.textContent = focus.running ? "focusing…" : (focus.paused ? "paused" : "ready to focus");
  document.querySelectorAll(".chip-btn").forEach((b) =>
    b.classList.toggle("sel", +b.dataset.mins * 60 === focus.duration && !focus.running && !focus.paused));
  $("focusStart").classList.toggle("hidden", focus.running || focus.paused);
  $("focusPause").classList.toggle("hidden", !focus.running && !focus.paused);
  $("focusPause").textContent = focus.paused ? "Resume" : "Pause";
  $("focusEnd").classList.toggle("hidden", !focus.running && !focus.paused);
}

function focusComplete() {
  focus.running = false; focus.paused = false;
  clearInterval(focus.timer);
  focus.log.unshift({ at: Date.now(), mins: Math.round(focus.duration / 60) });
  focusSaveLog();
  focus.remaining = focus.duration;
  renderFocus(); renderFocusLog();
  toast("Focus session complete.");
}

function focusTickSec() {
  focus.remaining--;
  if (focus.remaining <= 0) { focusComplete(); return; }
  renderFocus();
}

function focusStart() {
  if (focus.paused) { focus.paused = false; focus.timer = setInterval(focusTickSec, 1000); renderFocus(); return; }
  focus.running = true;
  focus.remaining = focus.duration;
  focus.timer = setInterval(focusTickSec, 1000);
  renderFocus();
  toast("Focus session started.");
}

function focusPauseToggle() {
  if (focus.running && !focus.paused) { focus.paused = true; clearInterval(focus.timer); }
  else if (focus.paused) { return focusStart(); }
  renderFocus();
}

function focusEnd() {
  const done = Math.round((focus.duration - focus.remaining) / 60);
  focus.running = false; focus.paused = false;
  clearInterval(focus.timer);
  if (done >= 1) {
    focus.log.unshift({ at: Date.now() - (focus.duration - focus.remaining) * 1000, mins: done });
    focusSaveLog();
  }
  focus.remaining = focus.duration;
  renderFocus(); renderFocusLog();
}

function renderFocusLog() {
  if (!$("focusLog")) return;
  const todayStr = new Date().toDateString();
  const today = focus.log.filter((s) => new Date(s.at).toDateString() === todayStr);
  const totalMin = today.reduce((s, x) => s + x.mins, 0);
  $("focusSummary").textContent = today.length
    ? `today: ${today.length} session${today.length > 1 ? "s" : ""} · ${fmtDur(totalMin * 60)} focused`
    : "no sessions yet today";
  $("focusEmpty").classList.toggle("hidden", focus.log.length > 0);
  $("focusLog").innerHTML = focus.log.slice(0, 10).map((s) => {
    const d = new Date(s.at);
    return `<div class="focus-row">
      <span class="fx"><span class="a">${s.mins} minute${s.mins > 1 ? "s" : ""}</span>
      <span class="b">${d.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" })} · ${d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })}</span></span>
      <span class="len">✓</span></div>`;
  }).join("");
}

/* ---------- Bedtime (client-side schedule in localStorage) ---------- */

const BED_KEY = "chrona-bedtime";
const bed = { enabled: false, start: "22:30", end: "06:30", days: [1, 2, 3, 4, 5], grey: true };
const DAY_LETTERS = ["M", "T", "W", "T", "F", "S", "S"];

function bedLoad() {
  try { Object.assign(bed, JSON.parse(localStorage.getItem(BED_KEY) || "{}")); } catch {}
}

function bedSave() {
  localStorage.setItem(BED_KEY, JSON.stringify(bed));
  renderBedtime();
}

function bedInRange(now) {
  if (!bed.enabled || !bed.days.length) return false;
  const day = ((now.getDay() + 6) % 7) + 1; // 1=Mon
  if (!bed.days.includes(day)) return false;
  const [sh, sm] = bed.start.split(":").map(Number);
  const [eh, em] = bed.end.split(":").map(Number);
  const mins = now.getHours() * 60 + now.getMinutes();
  const s = sh * 60 + sm, e = eh * 60 + em;
  return s <= e ? (mins >= s && mins < e) : (mins >= s || mins < e);
}

function renderBedtime() {
  if (!$("bedEnable")) return;
  $("bedEnable").classList.toggle("on", bed.enabled);
  $("bedStart").value = bed.start;
  $("bedEnd").value = bed.end;
  $("bedGrey").classList.toggle("on", bed.grey);
  $("bedDays").innerHTML = DAY_LETTERS.map((l, i) =>
    `<button class="day-chip ${bed.days.includes(i + 1) ? "on" : ""}" data-day="${i + 1}">${l}</button>`).join("");
  $("bedDays").querySelectorAll(".day-chip").forEach((b) =>
    b.addEventListener("click", () => {
      const d = +b.dataset.day;
      bed.days = bed.days.includes(d) ? bed.days.filter((x) => x !== d) : [...bed.days, d].sort();
      bedSave();
    }));
  const active = bedInRange(new Date());
  $("bedWhen").textContent = !bed.enabled ? "Bedtime is off"
    : active ? "Wind-down active" : `Bedtime · ${bed.start} → ${bed.end}`;
  const dl = bed.days.length === 7 ? "every day"
    : bed.days.length ? bed.days.map((d) => DAY_LETTERS[d - 1]).join(" ") : "no days picked";
  $("bedSummary").textContent = `${bed.start} → ${bed.end} · ${dl}`;
  const preview = $("bedPreview");
  preview.classList.toggle("grey", bed.enabled && bed.grey && active);
}

/* ---------- Settings ---------- */

function renderSettings() {
  const s = state.status;
  if (!s) return;
  $("setWatcher").innerHTML = `Watcher: <span class="mono-font">${esc(s.watcher || "?")}</span>`;
  $("setIdle").innerHTML = `Idle detection: <span class="mono-font">${esc(s.idle_provider || "?")}</span>`;
  $("setPrivacy").innerHTML =
    `All data stays local in <span class="mono-font">${esc(s.db_path || "~/.local/share/chrona/chrona.db")}</span> — no network access.`;
  $("aboutVersion").textContent = `Chrona ${s.version || "?"} · web demo`;
  $("themeSwitch").classList.toggle("on", state.theme === "dark");
  $("pauseSwitch").classList.toggle("on", !!s.paused);
  syncNotifySwitch();
}

function syncNotifySwitch() {
  const on = localStorage.getItem("chrona-notify") === "1" &&
    "Notification" in window && Notification.permission === "granted";
  $("notifySwitch").classList.toggle("on", on);
}

/* ---------- wiring ---------- */

function wire() {
  document.querySelectorAll(".nav-item").forEach((b) =>
    b.addEventListener("click", () => setPage(b.dataset.page)));
  $("gearBtn").addEventListener("click", () =>
    setPage(state.page === "settings" ? "today" : "settings"));

  $("navToggle").addEventListener("click", () => {
    if (navMql.matches) drawerOpen = !drawerOpen;
    else {
      navCollapsed = !navCollapsed;
      localStorage.setItem("chrona-nav", navCollapsed ? "collapsed" : "expanded");
    }
    applyNav();
  });
  $("scrim").addEventListener("click", () => { drawerOpen = false; applyNav(); });
  navMql.addEventListener("change", applyNav);

  // theme switch (Settings only)
  $("themeSwitch").addEventListener("click", () =>
    applyTheme(state.theme === "dark" ? "material" : "dark", true));

  // request notification permission on first interaction if enabled
  document.addEventListener("pointerdown", () => {
    if ("Notification" in window && Notification.permission === "default" &&
        localStorage.getItem("chrona-notify") === "1") {
      Notification.requestPermission();
    }
  }, { once: true });

  // today: expandable app rows (event delegation)
  $("todayApps").addEventListener("click", (e) => {
    const row = e.target.closest(".approw");
    if (row) toggleApp(row.parentElement);
  });

  // timers
  $("goalKind").addEventListener("change", fillGoalSuggestions);
  const syncRange = () => {
    const v = +$("goalRange").value;
    $("goalRangeLabel").textContent = `${v} min / day`;
    $("goalMins").value = v;
  };
  $("goalRange").addEventListener("input", syncRange);
  $("goalMins").addEventListener("input", () => {
    const v = Math.min(240, Math.max(5, +$("goalMins").value || 60));
    $("goalRange").value = Math.round(v / 5) * 5;
    $("goalRangeLabel").textContent = `${v} min / day`;
  });
  $("goalAdd").addEventListener("click", async () => {
    const mins = parseInt($("goalMins").value, 10);
    if (!mins || mins <= 0) return;
    const kind = $("goalKind").value;
    await apiPost("goal.set", {
      kind,
      key: kind === "total" ? "total" : $("goalKey").value,
      limit_seconds: mins * 60, enabled: true,
    });
    toast("Timer added.");
    refreshGoals();
  });

  // over-limit banner dismiss (delegated — content is re-rendered)
  $("limitBanner").addEventListener("click", (e) => {
    if (e.target.closest("#bannerDismiss")) {
      state.bannerHiddenUntil = Date.now() + 120000;
      $("limitBanner").classList.add("hidden");
    }
  });

  // pause tracking (ActivityWatch parity — real daemon-side switch)
  $("pauseSwitch").addEventListener("click", async () => {
    const p = !(state.status && state.status.paused);
    const r = await apiPost("pause.set", { paused: p });
    if (!r.ok) { toast(r.error || "daemon unreachable"); return; }
    if (state.status) state.status.paused = p;
    $("pauseSwitch").classList.toggle("on", p);
    renderStatus();
    toast(p ? "Tracking paused." : "Tracking resumed.");
  });

  // browser limit alerts
  $("notifySwitch").addEventListener("click", async () => {
    const want = localStorage.getItem("chrona-notify") !== "1";
    if (want && "Notification" in window && Notification.permission !== "granted") {
      const perm = await Notification.requestPermission();
      if (perm !== "granted") { toast("Notifications blocked by the browser."); syncNotifySwitch(); return; }
    }
    localStorage.setItem("chrona-notify", want ? "1" : "0");
    syncNotifySwitch();
    toast(want ? "Limit alerts on." : "Limit alerts off.");
  });

  // CSV export (spreadsheets — ActivityWatch parity)
  $("csvBtn").addEventListener("click", async () => {
    const res = await fetch("/api/export");
    const j = await res.json();
    const ev = (j.data && j.data.events) || [];
    const rows = [["start", "end", "app", "title", "seconds"].join(",")];
    for (const e of ev) {
      rows.push([e.start, e.end, JSON.stringify(e.app_id), JSON.stringify(e.title), e.end - e.start].join(","));
    }
    const blob = new Blob([rows.join("\n") + "\n"], { type: "text/csv" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "chrona-export.csv";
    a.click();
    URL.revokeObjectURL(a.href);
    toast("CSV exported.");
  });

  // focus
  document.querySelectorAll(".chip-btn").forEach((b) =>
    b.addEventListener("click", () => {
      if (focus.running || focus.paused) return;
      focus.duration = +b.dataset.mins * 60;
      focus.remaining = focus.duration;
      renderFocus();
    }));
  $("focusStart").addEventListener("click", focusStart);
  $("focusPause").addEventListener("click", focusPauseToggle);
  $("focusEnd").addEventListener("click", focusEnd);

  // bedtime
  $("bedEnable").addEventListener("click", () => { bed.enabled = !bed.enabled; bedSave(); });
  $("bedGrey").addEventListener("click", () => { bed.grey = !bed.grey; bedSave(); });
  $("bedStart").addEventListener("change", () => { bed.start = $("bedStart").value; bedSave(); });
  $("bedEnd").addEventListener("change", () => { bed.end = $("bedEnd").value; bedSave(); });

  // settings misc
  $("kwinBtn").addEventListener("click", () => {
    $("kwinNote").textContent = "Native app only — see docs/WATCHERS.md";
  });
}

/* ---------- boot ---------- */

async function boot() {
  wire();
  applyNav();
  focusLoad(); bedLoad();
  const url = new URLSearchParams(location.search).get("theme");
  if (url) await applyTheme(url, false);
  else {
    const saved = localStorage.getItem("chrona-theme");
    if (saved) await applyTheme(saved, false);
    else {
      const s = await api("settings.get", { key: "theme" });
      await applyTheme(s && s.value === "dark" ? "dark" : "material", false);
    }
  }
  await tick();
  fillGoalSuggestions();
  renderFocus(); renderFocusLog(); renderBedtime();
  setInterval(() => { if (document.visibilityState === "visible" && !focus.running) tick(); }, 5000);
  focus.timerTick = setInterval(() => { if (focus.running) renderFocus(); }, 1000);
}

boot();
