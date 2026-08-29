# How Chrona compares

Chrona is deliberately not a clone of any single product: it takes the
*wellbeing layer* of Google Digital Wellbeing (goals, focus, bedtime) and
merges it with the *local-first tracking honesty* of ActivityWatch, on a
native Rust + Slint stack. This page tracks where we stand against the
tools people actually compare us to, and where we intentionally do not
compete.

Status legend: ✅ shipped · 🟡 partial · ❌ not yet (roadmap) · 🚫 non-goal.

## Feature matrix

| | Chrona | Google Digital Wellbeing | ActivityWatch | RescueTime | Apple Screen Time |
|---|---|---|---|---|---|
| Per-app screen time | ✅ | ✅ | ✅ | ✅ | ✅ |
| Category breakdown | ✅ rules, query-time | ✅ fixed | ✅ tags, query-time | ✅ fixed | ✅ fixed |
| AFK/idle subtraction | ✅ + away time shown | n/a (phone) | ✅ shown | ✅ | n/a |
| Day timeline visual | ✅ app strip + hourly bars | ✅ strip | ✅ vertical timeline | ✅ | 🟡 bars |
| Unlocks counter | ✅ | ✅ | ❌ | ❌ | ✅ pickups |
| **Overall daily screen-time goal** | ✅ | ✅ | ❌ | ❌ | 🟡 (per-app only) |
| Per-app / per-category daily limits | ✅ | ✅ | ❌ | ❌ | ✅ |
| Limit-reached notification | ✅ desktop + dashboard banner | ✅ | ❌ | ✅ | ✅ |
| Over-limit enforcement (blocking) | ❌ roadmap | ✅ pause app | ❌ | ✅ (premium) | ✅ |
| Focus sessions | 🟡 timer + log (DND roadmap) | ✅ pause apps | ❌ | ✅ | ✅ |
| Bedtime / wind-down | 🟡 schedule (enforce roadmap) | ✅ grayscale + DND | ❌ | ❌ | ✅ Downtime |
| Per-day-of-week limits | ❌ roadmap | 🟡 | ❌ | 🟡 | ✅ |
| Pause tracking | ✅ | ❌ | ✅ | ✅ | ❌ |
| Local-first, no network | ✅ by architecture | ❌ device-linked | ✅ | ❌ cloud | 🟡 device sync |
| Data export | ✅ JSON + CSV | ✅ Takeout | ✅ JSON + CSV | 🟡 | ✅ |
| Search / query history | ❌ roadmap | ❌ | ✅ query language | ✅ | 🟡 |
| Per-URL browser tracking | ❌ roadmap (extension) | n/a | ✅ extension | ✅ extension | n/a |
| Notification counting | ❌ hard on Linux¹ | ✅ | ❌ | ❌ | ✅ |
| Multi-device sync | 🚫 non-goal (privacy) | ✅ | 🟡 manual | ✅ | ✅ |
| Linux-native (no Electron/web UI required) | ✅ | n/a | 🟡 web dashboard | ❌ | n/a |
| KDE Wayland window tracking | ✅ KWin script | n/a | 🟡 awatcher | ❌ | n/a |

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
- **KDE Plasma 6 Wayland support out of the box** via the bundled KWin
  script — no awatcher workarounds, plus wlr-foreign-toplevel and X11 EWMH
  backends in the same daemon.
- **PWA-aware tracking** — installed progressive web apps are separate
  windows with their own titles, tracked as first-class apps with their own
  icons in the dashboard.
- **Scripting surface by design** — the same Unix-socket JSON API the GUI
  uses is documented and stable-ish for your own tooling (`socat` one-liners
  in the README).

## Roadmap priorities (tracked against the matrix)

1. Enforcement: soft-block / "take a break" screen when a limit is hit
   (KWin script can raise a fullscreen notification window).
2. Browser companion extension for per-URL tracking inside
   Firefox/Chrome/Brave.
3. Per-day-of-week limit schedules (weekend vs weekday budgets).
4. Bedtime enforcement: grayscale + dim via KDE Night Color / gamma.
5. Focus sessions wiring Do Not Disturb on KDE and GNOME.
6. Weekly report notification (a Sunday-evening digest of your dashboards).
