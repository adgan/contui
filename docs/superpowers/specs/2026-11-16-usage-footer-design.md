# Host usage moves to the footer — design

Date: 2026-11-16
Status: approved
Supersedes: the `USAGE` card in `2026-11-15-live-usage-design.md`

## Intent

CPU and memory are live, host-wide readings, but the overview strip is about
resources: Containers, Images, Volumes. The fourth `USAGE` card competes with
those links and disappears when a detail page opens. Move the readings into a
footer bar pinned below every screen, so live usage is always visible and the
overview returns to three clickable resource cards.

## Goals

- The overview strip is three cards again: Containers, Images, Volumes.
- A footer bar at the bottom shows host CPU share and total memory used by
  running containers, plus the same coverage states as before.
- The footer appears on the list pages and on detail pages.
- No new Docker calls, timers or per-frame work: the footer reads the same
  `UsageTotals` the card used.

## Non-goals

- Per-container stats in the footer; the container detail band stays.
- History, charts or sparklines.
- A host memory limit, which Docker does not report here, so memory has no
  progress meter.

## Design

```
┌──────────┬──────────────────────────────────────────────────────────────┐
│  rail    │  top bar                                                     │
│          │  overview strip: CONTAINERS · IMAGES · VOLUMES                │
│          │  table                                                       │
│          ├──────────────────────────────────────────────────────────────┤
│          │  ◇ HOST USAGE │ CPU 4.1% │ MEMORY 1.2 GB        3 running      │
└──────────┴──────────────────────────────────────────────────────────────┘
```

- `Layout` gains `footer_height`: 32px, 28px compact.
- The footer uses `theme.panel`, a `border_subtle` top border and the page
  padding. Label `HOST USAGE` sits beside the activity icon; `CPU` and `MEMORY`
  are labelled monospace readings separated by 1px rules; coverage is right
  aligned.
- Labels come from `UsageBarLabels` (renamed from `UsageCardLabels`):
  - no snapshot → both `—`, status `connecting…`;
  - host disconnected → both `—`, status `host disconnected`;
  - no running containers → CPU `0.0%`, memory `—`, status `no running containers`;
  - running but no samples → both `—`, status `waiting for stats…`;
  - partial CPU coverage → `2 of 3 reporting`;
  - full CPU coverage → `3 running`.
- The toast sits above the footer (`footer_height + 12px`) so it does not cover
  the readings.
- Stale/missing samples keep the existing behavior: the value is `—`, never a
  frozen number.

## Release

Version `0.3.1`, a UI-only release on top of `0.3.0`. No data or protocol change.
