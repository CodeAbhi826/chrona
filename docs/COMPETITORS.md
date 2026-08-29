# How Chrona compares

Chrona is deliberately not a clone of any single product: it takes the
*wellbeing layer* of Google Digital Wellbeing (goals, focus, bedtime) and
merges it with the *local-first tracking honesty* of ActivityWatch, on a
native Rust + Slint stack. This page tracks where we stand against the
tools people actually compare us to, and where we intentionally do not
compete.

Status legend: ✅ shipped · 🟡 partial · ❌ not yet (roadmap) · 🚫 non-goal.

## Why this niche is open

"Wellbeing layers" are phone-first: Google's is Android-only, Apple's is
iOS/macOS-only. On the Linux desktop the honest trackers (ActivityWatch)
have no goals layer, and the goal tools (RescueTime, Cold Turkey) are
closed-source and cloud-bound. Users literally ask for this combination —
["Why is there no modern screen time tracker on KDE?"](https://www.reddit.com/r/kde/)
is a recurring thread. Chrona is that app: goals + focus + bedtime over
honest, AFK-aware, fully local tracking.

## Feature matrix

| | Chrona | Google Digital Wellbeing | ActivityWatch | RescueTime | Apple Screen Time | Cold Turkey |
|---|---|---|---|---|---|---|
| Per-app screen time | ✅ | ✅ | ✅ | ✅ | ✅ | n/a (blocker) |
| Real app names + icons from the OS | ✅ `.desktop` + icon theme | ✅ | 🟡 app-dependent | ✅ | ✅ | n/a |
| PWA tracking (installed web apps) | ✅ first-class: own name, icon, stats | ✅ | 🟡 looks like browser time | 🟡 | 🟡 | n/a |
| Category breakdown | ✅ rules, query-time | ✅ fixed | ✅ tags, query-time | ✅ fixed | ✅ fixed | n/a |
| AFK/idle subtraction | ✅ + away time shown | n/a (phone) | ✅ shown | ✅ | n/a | n/a |
| Day timeline visual | ✅ app strip + hourly bars | ✅ strip | ✅ vertical timeline | ✅ | 🟡 bars | n/a |
| Unlocks counter | ✅ | ✅ | ❌ | ❌ | ✅ pickups | n/a |
| **Overall daily screen-time goal** | ✅ | ✅ | ❌ | ❌ | 🟡 (per-app only) | ❌ |
| Per-app / per-category daily limits | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ (block allowances) |
| Escalating limit notifications | ✅ 90 % heads-up → critical → 15-min nags | 🟡 at-limit only | ❌ | ✅ alerts | 🟡 at-limit only | 🟡 |
| Over-limit enforcement (blocking) | ❌ roadmap (soft-block) | ✅ pause app | ❌ | ✅ premium | ✅ | ✅ hard block, frozen |
| Focus sessions | 🟡 timer + log (DND roadmap) | ✅ pause apps | ❌ | ✅ blocks + soundtracks | ✅ | ✅ |
| Bedtime / wind-down | 🟡 schedule (enforce roadmap) | ✅ grayscale + DND | ❌ | ❌ | ✅ Downtime | ✅ |
| Per-day-of-week limits | ❌ roadmap | 🟡 | ❌ | 🟡 | ✅ | ✅ |
| Pause tracking | ✅ | ❌ | ✅ | ✅ | ❌ | n/a |
| Weekly report / digest | ❌ roadmap (notification) | ✅ | 🟡 | ✅ email | ✅ | ❌ |
| Productivity score | 🚫 non-goal (we measure, not judge) | n/a | ❌ | ✅ | n/a | n/a |
| Local-first, no network | ✅ by architecture | ❌ device-linked | ✅ | ❌ cloud | 🟡 device sync | 🟡 per-device |
| Data export | ✅ JSON + CSV | ✅ Takeout | ✅ JSON + CSV | 🟡 | ✅ | n/a |
| Search / query history | ❌ roadmap | ❌ | ✅ query language | ✅ | 🟡 | n/a |
| Per-URL browser tracking | ❌ roadmap (extension) | n/a | ✅ extension | ✅ extension | n/a | ✅ (blocker) |
| Notification counting | ❌ hard on Linux¹ | ✅ | ❌ | ❌ | ✅ | n/a |
| Multi-device sync | 🚫 non-goal (privacy) | ✅ | 🟡 manual | ✅ | ✅ | ❌ |
| Linux-native (no Electron/web UI required) | ✅ | n/a | 🟡 web dashboard | ❌ | n/a | ✅ |
| KDE Wayland window tracking | ✅ KWin script | n/a | 🟡 awatcher | ❌ | n/a | n/a |

¹ Counting notifications reliably on Linux means replacing or proxying the
session's notification daemon — invasive for a wellbeing tool. Not planned
for v0.x.

## What we ship that the others don't

- **A wellbeing layer over honest tracking** — ActivityWatch tracks but has
  no goals; Digital Wellbeing has goals but is Android-only. Chrona gives
  you the screen-time goal, per-app timers, focus sessions and bedtime
  schedule on top of raw, AFK-aware, rule-re-categorisable history.
- **Query-time categorisation** — edit a rule and your entire history is
  re-categorised instantly (rules run at read time, not write time).
  ActivityWatch needs a query to do this; Digital Wellbeing can't.
- **PWA tracking done at the daemon level** — installed web apps
  (Chrome/Brave/Chromium/Edge "Install app…") appear as `crx_<id>` windows;
  the daemon maps them through their `.desktop` entry, so a YouTube Music
  PWA shows its own name, icon and stats rather than counting as browser
  time. Nobody in this table does PWA resolution on Linux.
- **Desktop-entry identity for every app** — names and icons are resolved
  from `StartupWMClass`/`Exec`/PWA app-ids against freedesktop `.desktop`
  files and the icon theme search path, in the daemon, for both UIs.
- **KDE Plasma 6 Wayland support out of the box** via the bundled KWin
  script — no awatcher workarounds, plus wlr-foreign-toplevel and X11 EWMH
  backends in the same daemon.
- **Scripting surface by design** — the same Unix-socket JSON API the GUI
  uses is documented and stable-ish for your own tooling (`socat` one-liners
  in the README).

## Where we honestly lose today (and what would fix it)

Ordered by what users ask for most:

1. **Enforcement.** Google pauses the app, RescueTime and Cold Turkey block
   it outright; we notify and show a banner. Next step: a KWin-script
   soft-block ("take a break" fullscreen card, dismissable) — Linux gives
   us no honest way to hard-block a determined user, and we won't pretend
   otherwise.
2. **Per-URL granularity.** ActivityWatch and RescueTime read browser URLs
   via extension. Our PWA support covers installed web apps today; a small
   companion extension is the fix for tabs.
3. **Weekly digest.** Every competitor emails or notifies a summary; we
   have the data (week payload already exists) but no Sunday-evening
   notification yet.
4. **Day-of-week limits.** Apple and Cold Turkey let weekends differ from
   weekdays; our limits are flat daily numbers so far.
5. **GNOME Wayland.** Needs a GNOME Shell extension for window events —
   the one big desktop we don't cover.
6. **Focus mode DND wiring** — our focus timer logs sessions but doesn't
   silence notifications; KDE/GNOME both expose D-Bus toggles we can call.

## Roadmap priorities (tracked against the matrix)

1. Enforcement: soft-block / "take a break" screen when a limit is hit
   (KWin script can raise a fullscreen notification window).
2. Focus sessions wiring Do Not Disturb on KDE and GNOME.
3. Weekly report notification (a Sunday-evening digest of your dashboards).
4. Per-day-of-week limit schedules (weekend vs weekday budgets).
5. Browser companion extension for per-URL tracking inside
   Firefox/Chrome/Brave.
6. Bedtime enforcement: grayscale + dim via KDE Night Color / gamma.
7. GNOME Wayland shell extension for window events.
