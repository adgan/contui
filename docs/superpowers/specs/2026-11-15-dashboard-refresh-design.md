# contUI — overview-first visual refresh — design

Date: 2026-11-15
Status: awaiting approval
Mockup: `docs/superpowers/specs/2026-11-15-dashboard-refresh-mockup.html`

The mockup is a layout study. Resource names, counts and inspect field values in it
are illustrative, not implementation requirements.

## Intent

contUI works, but it reads like a wall of text. The table is full-bleed, the page
title, counts, search and host chip all share one 56px line, every row is 32px of
13px text, and the detail page is a raw key/value dump. There are no margins, no
surface hierarchy, and state is a colored word rather than a shape.

This refresh makes the app feel deliberate without turning it into a slow,
card-heavy web dashboard. The chosen direction is **overview-first**: a compact
strip of live numbers above every list, a carded table, and a structured detail
page. It is a visual and layout change only. Data flow, shortcuts, mutations and
the native GPUI performance model stay exactly as they are.

Why now: mutations landed, the app is usable every day, and the next thing people
notice is how it looks. "Fast but ugly" is not the product.

## Goals

1. Give every screen real margins, grouping and a visible type hierarchy.
2. Put a three-card overview above each list: containers, images and volumes at a
   glance. The cards are live, clickable and never invented data.
3. Turn the table into a contained card with 44px rows, state pills, a clear
   selected row, and actions that appear on hover.
4. Give detail pages a hero header, a segmented tab control, grouped inspect cards
   and a terminal-style log viewer.
5. Bring dialogs, popovers, the host menu and toasts into the same visual system.
6. Keep the app feeling instant: no new timers, no polling for labels, no work per
   frame that can be done per snapshot.

## Non-goals

- No light mode. Dark only, as today; light mode stays v1.1.
- No new Docker data. No CPU/memory, no volume size, no disk-usage estimate.
- No new pages or navigation model. Rail, three lists, detail page, back.
- No animation or transition framework.
- No custom titlebar, glass, blur or transparent window chrome.
- No functional change to start/stop/restart/remove/run/pull, confirmations or
  error handling.

## Anti-goals

A refresh that technically compiles but is a failure:

- Fake dashboard furniture: sparklines, percentage deltas, "health scores" and
  other numbers the daemon did not give us.
- Cards so large the table is pushed below the fold at the default 820px height.
- Five competing accent colors, gradients or glow.
- A table that is prettier but slower to scan (fewer rows per screen, weaker
  alignment, state reduced to a dot with no label).
- A repaint timer just to keep "updated 2s ago" fresh.
- Light theme. That is a different design.

## Constraints

- GPUI 0.2.2, Rust 2024, one binary crate.
- Bundled Inter and JetBrains Mono; no system font fallback in the design.
- Default window 1280×820, minimum 960×600. Every layout must work at the minimum
  without clipping, horizontal scrolling or overlapping controls.
- Tables stay `uniform_list`; every row must be the same height.
- `theme.rs` stays the single source of truth for color, spacing, radii and type.
- No new crates.

## Approaches considered

**1. Quiet utility.** Keep the current layout and add page margins, better type
and state pills. Safe and cheap, but it still reads text-first and does not answer
the "no real margins / wall of text" complaint with a stronger structure. Rejected
after the user chose the dashboard direction.

**2. Overview-first dashboard (chosen).** A global overview strip above each list,
a carded table, a hero treatment on detail pages, and shared card/button/pill
primitives. It adds visual anchors and numbers without inventing data, and the
table stays the main event. Cost: roughly 100px of vertical space above the table
and a moderate amount of layout work.

**3. Terminal console.** Lean into monospace, hard rules and LED dots. Distinctive,
but doubles down on the text-heavy feeling this change exists to remove. Rejected.

### One decision inside the chosen approach

The overview is **one global strip**, not three different per-page stat rows. It
shows containers, images and volumes on every list page, which gives the app a
consistent dashboard header and avoids inventing a third volume metric (Docker
does not report volume size or usage). Page-specific counts remain in the page
subtitle. This is the point most worth objecting to at approval; the rest follows
from it.

## Design

### Layout grid

```
┌──────────┬──────────────────────────────────────────────────────────────┐
│  rail    │  top bar: title + subtitle        Run  search  refresh  host │
│  76px    ├──────────────────────────────────────────────────────────────┤
│          │  24px page padding                                           │
│  ▣       │  ┌────────────┐ ┌────────────┐ ┌────────────┐                │
│  ▤       │  │ CONTAINERS │ │   IMAGES   │ │  VOLUMES   │  overview      │
│  ◧       │  │ 3 running  │ │ 45 · 5.6GB │ │ 1 · local  │  strip (88px)  │
│          │  └────────────┘ └────────────┘ └────────────┘                │
│          │                                                              │
│          │  ┌──────────────────────────────────────────────────────┐    │
│          │  │ NAME           STATE     IMAGE       PORTS   CREATED │    │
│          │  │ sploney-db-1   ● running postgres:18 5432    2m ago  │    │
│          │  │ …                                                    │    │
│          │  └──────────────────────────────────────────────────────┘    │
│          │                                                              │
│  ●       │                                                              │
└──────────┴──────────────────────────────────────────────────────────────┘
```

- Spacing scale: `4, 8, 12, 16, 20, 24, 32`. Page padding 24, section gap 20,
  card padding 16–20, control gap 8.
- Compact mode applies when the window content area is under 700px tall or under
  1080px wide: page padding 16, section gap 12, top bar 56, overview strip 76.
  Nothing is hidden; only sizes shrink.
- Radii: 6 (small controls), 10 (inputs, segmented control), 14 (cards, dialogs),
  999 (pills).
- Type roles:

  | Role | Size | Weight | Color | Used for |
  |---|---|---|---|---|
  | Kicker | 10 | 600 | dim | table headers, card labels, section labels |
  | Meta | 11 | 500 | dim | counts, timestamps, captions |
  | Body | 13 | 400 | text | table cells, dialog copy |
  | Body strong | 13 | 500 | text | names, active labels |
  | Title | 18 | 600 | text | page titles |
  | Hero | 22 | 600 | text | detail resource name |
  | Display | 28 | 600 | text | overview numbers |
  | Mono | 12 | 400 | dim/text | IDs, image tags, ports, inspect values, logs |

- The list screen uses two bands: the 64px top bar (56 compact) and the overview
  strip (88px, 76 compact). The table card gets all remaining height and scrolls
  internally.
- The detail screen drops the overview strip so the inspect/log body gets the
  space instead.

### Shell

**Rail (76px).** Background one step darker than the app, border on the right.
Top: a 32px container-stack mark on an accent-soft tile. Three nav items, each
60×54: 20px icon, 4px gap, 9px uppercase label. Selected item: selected surface,
accent icon and label, 3px accent bar on the rail's left edge. Hover: hover
surface. Bottom: an 8px status dot with a small "Host" label; its color is the
same connected/unknown/error mapping as today.

**Top bar (64px, 56px compact).** Left: page title 18/600, subtitle 11/dim below it. The subtitle
is page-specific: "3 running · 13 stopped", "45 images · 5.6 GB", "1 volume ·
local driver". Right cluster, 8px gaps:

- Primary action, only on Containers (Run) and Images (Pull): 32px high, accent
  fill, 14px icon, 12/500 label, radius 10.
- Search: 260×32 (200 at minimum width), icon, placeholder "Search", and a small
  `⌘F` hint chip aligned right. Query text in 13px.
- Refresh: 32×32 ghost icon.
- Host chip: 32px high, status dot, host name, chevron; bordered surface; opens
  the host menu.

**Overview strip (88px, 76px compact; list pages only).** Three equal cards,
12px gaps, 16px padding, panel surface, 1px border, radius 14. Card height is
authoritative and content never grows it: 12px top padding, 10/13 kicker, 4px
gap, 28/30 value row, 2px gap, 11/14 foot, 12px bottom padding; the 3px
progress bar is pinned flush to the bottom edge and overlays the bottom padding,
so it adds no stack height. Compact
mode uses 76px, 10px padding, a 24px value row and 9/12 kicker and foot. Labels
that still do not fit truncate with an ellipsis. Each card is clickable and
switches to its page. Active card: accent border, accent icon and a short 2px
accent rule at the top; hover: hover surface. No gradient.

- Containers: kicker `CONTAINERS`; value row is the 28px running count with
  "running" beside it and "of N" on the right; a 3px bar at the bottom edge shows
  the running share in green over border color; foot "N stopped".
- Images: kicker `IMAGES`; value row is the 28px count with "images" beside it
  and "N in use" on the right; foot "X total".
- Volumes: kicker `VOLUMES`; value row is the 28px count with "volume" beside it
  and the driver on the right; foot "local driver" or "N drivers".
- When no snapshot has arrived: value `—`, foot "connecting…". When the host is
  disconnected, keep the last values; the host chip owns the error state.
- No percentage deltas, no sparklines, no invented metrics.

### Table card

- One panel card: panel surface, 1px border, radius 14, overflow hidden, fills the
  remaining height.
- Header row 40px, 20px horizontal padding, no background, bottom border subtle.
  Kicker labels; active sort column gets an accent arrow. The action column header
  is empty.
- Rows 44px, 20px horizontal padding, 1px bottom rule in border-subtle. Name cell
  13/500; mono cells 12/dim; created 11/dim. Hover fills, pointer cursor.
- Selected row: selected surface plus a 3px accent bar pinned to the left edge;
  row text brightens. Hover must not wash out a selected row.
- State pill: 22px high, 8px horizontal padding, pill radius, 6px dot + 11/500
  label. Running green on green-soft; paused/restarting amber on amber-soft;
  exited/created/dead/unknown grey on grey-soft; error red on red-soft.
- Row actions: 28×28 ghost icon buttons, 14px icons, gap 2. At rest they sit at
  35% opacity; row hover or selection brings them to 100%. Remove turns red-soft on
  hover. The image row also keeps its Run button.
- Empty and error states live inside the card body: 40px icon tile, 15/600 title,
  12/dim body (max 360px wide). They add no action buttons: the top-bar Run/Pull
  action and the error banner's existing Retry are the only actions, exactly as
  today. No blank white space.
- Compose grouping: containers carrying `com.docker.compose.project` are bucketed
  under a 44px group header (chevron, stacked-tile icon, project name, visible
  container count, running dot/count). Groups sort by project name and standalone
  containers follow under an `UNGROUPED` section header. Every section header
  starts with a 12px blank band, which separates consecutive groups without
  breaking the 44px uniform row height. Clicking a project or `UNGROUPED`
  header collapses its rows; `↑`/`↓` selection skips collapsed rows. A 32px
  `Group` toggle in the top bar (Containers only, shown when the host has
  Compose containers) turns grouping off and on. The toggle is on by default.
  Filtering and column sorting still apply inside groups.
- Pull strip and error banner: 36px, 24px horizontal padding, accent-soft or
  error-soft background, bottom border, 12px text; progress bar 120×4.
- Column widths (final tuning left to the builder):

  | Page | Columns |
  |---|---|
  | Containers | Name flex 2.2 · State 104 · Image flex 2 · Ports 132 · Created 100 · Actions 132 |
  | Images | Repository flex 3 · ID 120 · Size 90 · Created 100 · In Use 80 · Actions 92 |
  | Volumes | Name flex 2 · Driver 100 · Mountpoint flex 3 · Created 100 · Actions 52 |

### Detail page

- Top bar 60px: back icon button 28×28 plus the list name at 14/600. Right: the
  host chip, so the current Docker host is still visible away from the list.
- Hero card (20px padding, panel surface, border, radius 14): 44px icon tile,
  resource name 22/600, and a meta line with the state pill, mono short id, image
  (for containers), and created label. On the right, the resource actions as
  labeled 32px buttons (Start, Stop, Restart) with 14px icons, and a red-ghost
  Remove. Disabled actions stay visible but dim, exactly as today. A pending
  action label sits in amber.
- Tabs: segmented control below the hero, on the app background, 3px padding,
  radius 10; each tab 30px high, 12/500; selected gets panel-raised surface and
  text color; unselected is dim. No underline.
- Body card fills the rest: panel surface, border, radius 14. Inspect cards and
  their rows never shrink inside the scroll container; long keys and values
  truncate instead of spilling sideways. A live refresh updates the tree in
  place and keeps the current scroll position instead of remounting the body.
  - **Inspect:** 20px padding, 12px section gaps. Rendering stays mechanical:
    reuse `build_tree(&json, 2)` unchanged. Top-level nodes with scalar values
    become one "Overview" card, a two-column key/value grid in `build_tree`
    order. Every top-level group becomes its own card: 11/600 uppercase header,
    32px rows with a 200px dim key column and a mono value column, subtle row
    rules. Values nested below a group stay one summarized mono row via the
    existing `summarize()`, clipped at 200 chars; no recursion, dotted keys,
    nested cards or per-resource curation. The mockup's Overview shows six
    top-level scalars (`Id`, `Image`, `Created`, `Platform`, `Path`, `Args`), all
    produced by `build_tree`; it is not a curated field list. A nested value such
    as `Config.Cmd` stays inside its `Config` card.
  - **Logs:** 44px toolbar with line count, stderr count and a Reload ghost
    button; 16px terminal padding, mono 12/20, timestamps 10/faint, stderr amber.
    Horizontal scroll for long lines. Keep the static 500-line tail.
  - **Loading:** centered 12px dim label. **Error:** 40px icon tile, message,
    32px Retry button. **Missing:** same as today, but in the card language.

### Dialogs, popovers, menus

- Shared panel: panel surface, 1px border, radius 14, large shadow.
- Confirm (460px, 20px padding): danger icon tile 32px, title 16/600, body 13/dim,
  force choice as a 12px-padded card row, footer with ghost Cancel and red Remove.
- Run (680px, max height 600): header 20px, body 20px with 20px section gaps,
  section labels 10/600 uppercase, 34px inputs, add actions as accent text links,
  footer 16/20 with ghost Cancel and accent Run.
- Pull popover (360px, 16px padding) anchored below the top bar; host menu (380px,
  8px padding) with 52px rows, status dot, name, mono endpoint, active check.
- Toast (440px, bottom center): icon, 12px text, 12px padding, border and shadow;
  error variant uses red-soft.

### Theme and primitives

Keep the existing core palette. Add:

| Token | Value | Use |
|---|---|---|
| `rail` | `#0e1116` | rail surface |
| `panel_raised` | `#171c24` | raised controls, tab selected |
| `border_subtle` | `#1a2028` | row rules, dividers |
| `text_faint` | `#5b6472` | tertiary labels |
| `accent_soft` | `#16243c` | selected nav wash, primary hover |
| `running_soft` | `#12281a` | running pill |
| `paused_soft` | `#2b2412` | paused pill |
| `exited_soft` | `#1d222a` | inactive pill |
| `error_soft` | `#2d1718` | error pill, banner |

`theme.rs` also gains named spacing, radius and type constants so layout code does
not scatter magic numbers.

New shared widgets:

- `ui/style.rs` — spacing/radius/type constants and `card()` helper.
- `ui/widgets/pill.rs` — `state_pill(label, color, soft_background)`.
- `ui/widgets/button.rs` — primary, ghost, danger and icon buttons, with disabled
  and hover states.
- `ui/widgets/stat_card.rs` — the overview card (kicker, value, caption, optional
  progress bar, click handler).
- `ui/stats.rs` — `OverviewStats::from_snapshot(&Snapshot)` and its tests.
- `ui/shell.rs` — rail, top bar and overview strip; keeps `root.rs` from growing
  further. `root.rs` keeps state and action wiring.

New icons: containers, images, volumes, search, refresh, chevron-down, back,
terminal, alert, check. Same 16×16 stroke style as the existing set, tinted through
`text_color`.

### Overview data

Computed once per snapshot into `RootView`:

- Containers: `total = len`, `running = count(state == Running)`,
  `stopped = total - running`.
- Images: `total = len`, `in_use = count(in_use)`,
  `bytes = sum(size)`, plus a preformatted size label.
- Volumes: `total = len`, `drivers = sorted distinct driver names`.
- No new requests. No timer. The subtitle and cards update only when the snapshot
  does, which is exactly how counts work today.

### Behavior and state

- Rail, ⌘1/⌘2/⌘3, arrows, Enter, Esc, ⌘F, ⌘R, ⌘N and ⌘⇧P keep their current
  behavior.
- Clicking an overview card switches page; that is the only new interaction.
- Search stays in the top bar, per page. Clearing and empty states stay the same.
- Selection, sorting, row actions, right-click menu, confirmations and toasts keep
  their current behavior; only their surfaces move.
- No repaint timer, no polling, no new network calls.

### Performance

- Overview stats and their formatted labels are computed once when a snapshot is
  applied, not per frame.
- Table rendering stays virtualized with a constant 44px row.
- No animation, no image assets beyond small SVGs, no new dependencies.
- At 960×600 in compact mode the page must show at least seven full 44px rows
  below the overview strip, including when the pull strip or error banner is
  visible. The table scrolls for the rest; the row height does not shrink.

### Testing and acceptance

Unit tests:

- `OverviewStats` on empty, mixed, paused and multi-driver snapshots.
- Image byte totals and preformatted labels.
- Theme token tests updated for the new tokens.
- Every existing library test keeps passing.

Manual acceptance:

1. At 1280×820 and 960×600, no header, card or table clips; the table scrolls and
   the page keeps 24px margins (16px in compact mode).
2. All three overview cards show correct live numbers and navigate on click.
3. State pills, hover actions, selected accent bar and empty/error states render
   correctly for every container state.
4. Inspect groups render as cards; logs render in the terminal card; loading,
   error and missing states all look intentional.
5. Run, confirm, pull, host menu and toast match the new surfaces.
6. All keyboard flows and mutations behave exactly as before.
7. `cargo test --lib` and `cargo build` pass; the live Docker test still passes
   when a daemon is available.

## What is left to the builder

- Exact SVG drawings for the new icons.
- Exact microcopy for card captions ("stopped" vs "not running").
- The final column-width tuning against real Rancher data.
- Whether the rail labels are 9px uppercase or 10px sentence case.
- Where the new modules are split (the list above is a recommendation).
- Fine color tuning inside the stated palette.
