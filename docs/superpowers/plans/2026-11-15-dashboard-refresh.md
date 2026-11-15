# contUI overview-first dashboard refresh — implementation plan

Date: 2026-11-15
Design: `docs/superpowers/specs/2026-11-15-dashboard-refresh-design.md`
Branch: `feat/improve-ui-spacing-and-visuals`

## Goal

Replace the full-bleed text table with the approved overview-first dashboard:
76px rail, 64px top bar with title/subtitle and actions, a three-card global
overview strip, a carded 44px-row table, a hero detail page, and restyled
dialogs. Visual and layout change only; all state, shortcuts, data flow and
mutations keep working.

## Decisions taken while planning

- **`shell.rs` is not split out.** The design allows the builder to choose the
  split. The rail and top bar stay in `root.rs` because they call private
  `RootView` methods (`on_search_key`, `switch_page`, `refresh`). Moving them
  would only add `pub(crate)` churn. Pure presentation helpers still move to
  `ui/style.rs`, `ui/stats.rs` and `ui/widgets/*`.
- **`theme.rs` owns the scale values**, `ui/style.rs` owns `Layout` and the
  `card()` composite. `theme.rs` remains the single source of truth.
- **Compact mode** is `viewport.width < 1080 || viewport.height < 700`.
- **No new behavior.** The only new interaction is clicking an overview card to
  switch page.

## File map

New:

- `src/ui/style.rs` — `Layout`, `card()`, `kicker()`, `meta()` text helpers.
- `src/ui/stats.rs` — `OverviewStats`, subtitle formatting.
- `src/ui/widgets/pill.rs` — state pills.
- `src/ui/widgets/button.rs` — primary/ghost/danger/icon buttons.
- `src/ui/widgets/stat_card.rs` — overview card.
- `assets/icons/{containers,images,volumes,search,refresh,chevron-down,back,terminal,alert,check}.svg`.

Changed:

- `src/theme.rs` — nine color tokens plus spacing/radius/type constants.
- `src/ui/mod.rs`, `src/ui/widgets/mod.rs` — module registration.
- `src/ui/table.rs` — `CellStyle`, widths 132/92/52, remove `accent`, tests.
- `src/ui/root.rs` — shell, overview strip, table card, detail page, chrome.
- `src/ui/detail.rs` — log toolbar rendering (keep log body behavior).
- `src/ui/dialog.rs` — visual restyle only.
- `src/ui/widgets/text_input.rs` — 34px field, radius 10.
- `src/assets.rs` — new icons and asset test.

## Verification commands

Run after every task that changes Rust:

```bash
cargo test --lib
cargo build
```

Final manual pass at 1280×820 and 960×600 per the design's acceptance list.

---

## Task 1 — Theme tokens, scale constants, layout helper

**Files:** `src/theme.rs`, `src/ui/style.rs`, `src/ui/mod.rs`

1. Add to `Theme` and `Theme::dark()`:

```rust
pub rail: Rgba,           // #0e1116
pub panel_raised: Rgba,   // #171c24
pub border_subtle: Rgba,  // #1a2028
pub text_faint: Rgba,     // #5b6472
pub accent_soft: Rgba,    // #16243c
pub running_soft: Rgba,   // #12281a
pub paused_soft: Rgba,    // #2b2412
pub exited_soft: Rgba,    // #1d222a
pub error_soft: Rgba,     // #2d1718
```

2. Add modules in `theme.rs`:

```rust
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 20.0;
    pub const XXL: f32 = 24.0;
    pub const XXXL: f32 = 32.0;
}

pub mod radius {
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 10.0;
    pub const LG: f32 = 14.0;
    pub const PILL: f32 = 999.0;
}

pub mod type_scale {
    pub const KICKER: f32 = 10.0;
    pub const META: f32 = 11.0;
    pub const BODY: f32 = 13.0;
    pub const TITLE: f32 = 18.0;
    pub const HERO: f32 = 22.0;
    pub const DISPLAY: f32 = 28.0;
    pub const MONO: f32 = 12.0;
}
```

3. Create `src/ui/style.rs`:

```rust
use crate::theme::{Theme, radius, space};
use gpui::{Div, Pixels, Size, div, px};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub compact: bool,
    pub rail_width: f32,
    pub top_bar_height: f32,
    pub overview_height: f32,
    pub page_padding: f32,
    pub section_gap: f32,
    pub table_header_height: f32,
    pub row_height: f32,
    pub cell_padding: f32,
}

impl Layout {
    pub fn for_viewport(viewport: Size<Pixels>) -> Self {
        let compact = viewport.width < px(1080.) || viewport.height < px(700.);
        Self {
            compact,
            rail_width: if compact { 64.0 } else { 76.0 },
            top_bar_height: if compact { 56.0 } else { 64.0 },
            overview_height: if compact { 76.0 } else { 88.0 },
            page_padding: if compact { space::LG } else { space::XXL },
            section_gap: if compact { space::MD } else { space::XL },
            table_header_height: 40.0,
            row_height: 44.0,
            cell_padding: 20.0,
        }
    }
}

pub fn card(theme: &Theme) -> Div {
    div()
        .bg(theme.panel)
        .border_1()
        .border_color(theme.border)
        .rounded(px(radius::LG))
        .overflow_hidden()
}
```

4. Register `pub mod style;` in `src/ui/mod.rs`.

**Tests:** extend `theme.rs` test to assert `rail`, `panel_raised`,
`border_subtle`, `text_faint`, `accent_soft`, `running_soft`, `paused_soft`,
`exited_soft`, `error_soft` equal their spec values. Add to `style.rs`:

- `layout_is_compact_below_thresholds` — 960×600 compact, 1280×820 full.
- `layout_heights_match_design` — top bar 64/56, overview 88/76, rows 44.

Commit: `feat: add dashboard theme tokens and layout scale`.

---

## Task 2 — Overview statistics

**Files:** `src/ui/stats.rs`, `src/ui/mod.rs`

```rust
use crate::docker::model::{ContainerState, Snapshot};
use gpui::SharedString;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverviewStats {
    pub containers_total: usize,
    pub containers_running: usize,
    pub containers_stopped: usize,
    pub images_total: usize,
    pub images_in_use: usize,
    pub images_bytes: i64,
    pub images_size_label: SharedString,
    pub volumes_total: usize,
    pub volumes_drivers: Vec<String>,
}

impl OverviewStats {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self;
    pub fn containers_share(&self) -> f32;      // 0.0..=1.0, 0 when total == 0
    pub fn volume_driver_label(&self) -> String; // "local" or "2 drivers" or "—"
    pub fn images_size_caption(&self) -> String; // "5.6 GB total" or "— total"
}
```

Rules:

- `containers_running` counts `ContainerState::Running` only. `stopped = total - running`.
- `images_bytes` sums `ImageRow::size`; label via `format_bytes`.
- Drivers: lowercased distinct? Keep the raw driver strings, sort and dedupe.
- Empty snapshot: all counts 0, size label `"—"`, drivers empty.

`root.rs` gets `pub stats: OverviewStats`, initialised from `Snapshot::empty()`
and set in `apply()` immediately after `self.snapshot = snapshot;`.

**Tests:** empty, mixed (running/paused/exited), image byte total and label,
single-driver vs multi-driver volume labels, container share clamps to 1.0.

Commit: `feat: compute overview statistics from each snapshot`.

---

## Task 3 — Shared pill, button and stat-card widgets

**Files:** `src/ui/widgets/{mod.rs,pill.rs,button.rs,stat_card.rs}`

### pill.rs

```rust
use crate::docker::model::ContainerState;
use crate::theme::Theme;
use gpui::{Div, Rgba, div, px};

pub fn pill(label: impl Into<gpui::SharedString>, fg: Rgba, soft: Rgba) -> Div;
pub fn state_pill(state: ContainerState, theme: &Theme) -> Div;
pub fn state_colors(state: ContainerState, theme: &Theme) -> (Rgba, Rgba);
pub fn state_label(state: ContainerState) -> &'static str;
```

Pill: `h(px(22.))`, `.px(px(8.))`, `.rounded(px(999.))`, 6px dot + 11/500
label, background `soft`, text/dot `fg`. Running green, paused/restarting
amber, exited/created/dead/unknown grey, dead uses error red. (Keep the current
`state_color` mapping: dead = error, exited/created/unknown = exited.)

### button.rs

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant { Primary, Ghost, Danger }

pub struct ButtonSpec<'a> {
    pub label: &'a str,
    pub icon: Option<&'static str>,
    pub variant: ButtonVariant,
    pub enabled: bool,
    pub emphasized: bool,
}

pub fn button(
    id: impl Into<gpui::ElementId>,
    spec: ButtonSpec<'_>,
    theme: &Theme,
    cx: &mut gpui::Context<crate::ui::root::RootView>,
    on_click: impl Fn(&mut crate::ui::root::RootView, &mut gpui::Context<crate::ui::root::RootView>) + 'static,
) -> gpui::Stateful<gpui::Div>;

pub fn icon_button(...same shape, label replaced by tooltip-free icon...) -> Stateful<Div>;
```

Sizes: 32px high, radius 10, 12/500, gap 6, icon 14. Primary accent fill with
`theme.bg` text; Ghost border and panel surface; Danger ghost with error text
and `error_soft` hover. Disabled: no cursor, 40% opacity. `emphasized == false`
on row icon buttons sets 35% opacity; hover is applied by the caller's row
`on_hover` state, matching current `mutations.hovered_row`.

### stat_card.rs

```rust
pub struct StatCard<'a> {
    pub id: &'static str,
    pub kicker: &'a str,
    pub icon: &'static str,
    pub value: String,
    pub unit: &'a str,
    pub right: String,
    pub foot: String,
    pub progress: Option<f32>,
    pub active: bool,
}

pub fn stat_card(
    card: StatCard<'_>,
    layout: &crate::ui::style::Layout,
    theme: &Theme,
    cx: &mut gpui::Context<crate::ui::root::RootView>,
    on_click: impl Fn(&mut crate::ui::root::RootView, &mut gpui::Context<crate::ui::root::RootView>) + 'static,
) -> Stateful<Div>;
```

Card: panel surface, 1px border, radius 14, fixed height from `Layout`,
padding 16/10, kicker row with 14px icon, 28px value + 11px unit + right meta,
foot 11/dim, 3px progress pinned to bottom (absolute, left/right/bottom 0).
Active: accent border + 2px accent top rule; hover: hover surface. No gradient.

**Tests:** `pill` has no pure logic beyond `state_label`; assert every state has
a non-empty label and dead maps to error red. `button` has no unit-testable
logic. Widgets must compile in both modules.

Commit: `feat: add pill, button and overview card primitives`.

---

## Task 4 — Icons and asset registry

**Files:** `assets/icons/*.svg`, `src/assets.rs`

Draw 16×16 stroke icons in the existing style (`stroke="currentColor"`,
`stroke-width="1.6"`, `fill="none"`, round caps/joins): containers (stacked
box), images (layers), volumes (database cylinder), search (magnifier), refresh
(circular arrow), chevron-down, back (left arrow), terminal (`>_` box), alert
(triangle), check (tick).

Register each in `Assets::load` exactly like the existing icons. Add a test:

```rust
#[test]
fn every_icon_is_embedded_and_parses_as_svg() {
    for path in ["icons/containers.svg", /* … all icons … */] {
        let bytes = Assets.load(path).unwrap().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.trim_start().starts_with("<svg"));
        assert!(text.contains("</svg>"));
    }
}
```

Commit: `feat: add dashboard icon set`.

---

## Task 5 — Table model: cell styles, widths, row fill fix

**Files:** `src/ui/table.rs`, `src/ui/root.rs`

1. Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellStyle { Text, Mono, Timestamp, Pill }
```

Add `pub styles: Vec<CellStyle>` to `TableSpec`. There must be exactly one style
per column. Update every spec constructor.

2. Change `ActionsColumn::width()` to `132.0 / 92.0 / 52.0`.

3. Remove `accent: Option<Rgba>` from `TableRow` and delete `SortValue` imports
that become unused. Update all row constructors and tests in `table.rs` and
`root.rs`.

4. Specs (design table):

```rust
// Containers
columns: Name, State, Image, Ports, Created
widths:  Flex(2.2), Fixed(104.), Flex(2.), Fixed(132.), Fixed(100.)
styles:  Text, Pill, Mono, Mono, Timestamp

// Images
columns: Repository, ID, Size, Created, In Use
widths:  Flex(3.), Fixed(120.), Fixed(90.), Fixed(100.), Fixed(80.)
styles:  Text, Mono, Mono, Timestamp, Text

// Volumes
columns: Name, Driver, Mountpoint, Created
widths:  Flex(2.), Fixed(100.), Flex(3.), Fixed(100.)
styles:  Text, Text, Mono, Timestamp
```

5. Add `TableSpec::style_count_matches_columns()` test and update the action
width assertions to 132/92/52.

6. In `render_table`'s row loop add `.w_full()` to the row `div` **before**
   `.h(px(ROW_HEIGHT))`. This is the real fix for shrink-wrapped rows: the
   uniform_list child must fill the full list width so columns and selected
   background align with the header.

Commit: `fix: fill table rows and add per-column cell styles`.

---

## Task 6 — Shell: rail, top bar, overview strip, page grid

**Files:** `src/ui/root.rs`, `src/ui/style.rs`

1. `render()` computes `let layout = Layout::for_viewport(window.viewport_size());`
   and passes `&layout` into `render_rail`, `render_header`, `render_table`,
   `render_detail`, `render_pull_strip`, `render_toast`.

2. **Rail** (width from layout, bg `theme.rail`): 32px accent-soft mark at top
   with the containers icon; three items 60×54, 20px icon + 9px uppercase
   label; selected = `selected` surface, accent icon/label, 3px accent bar on
   the rail's left edge; hover = hover surface. Bottom: 8px status dot + 9px
   "HOST" faint label.

3. **Top bar** (height from layout, padding page padding, bottom border):
   - Left: title 18/600 with 11/dim subtitle below (`self.subtitle()`), 2px gap.
   - Right: primary action (Run on Containers, Pull on Images), search
     260×32 (200 when compact), refresh 32×32 icon button, host chip 32px with
     dot + name + chevron. Pull remains a ghost button in the top bar; the
     pull popover anchor stays below it.
   - Search keeps `track_focus`, `on_click`, `on_key_down`, and the `⌘F` hint.
     Placeholder "Search", query body 13px.

4. **Overview strip** (list pages only, height from layout): three equal
   `stat_card`s with 12px gaps, page padding on both sides, section gap below.
   Data and captions:
   - Containers: value running, unit "running", right `of {total}`, foot
     `{stopped} stopped`, progress `containers_share`.
   - Images: value total, unit "images", right `{in_use} in use`, foot
     `{size} total`, no progress.
   - Volumes: value total, unit "volume(s)", right driver label without
     "driver", foot `{driver} driver` or `{n} drivers`, no progress.
   - Clicking calls `switch_page(page)` and closes the detail.
   - No snapshot yet: value "—", foot "connecting…" (use
     `snapshot.fetched_at.is_none()`).

5. **List content** becomes:

```
column: [top bar]
        [pull strip]
        [error banner]
        [page padding column:
           overview strip
           table card (flex_1, min_h 0)
        ]
```

The page-padding column is `flex_1 min_h(0) flex_col gap(section_gap)
px(page_padding) pb(page_padding)`. Table card is a `style::card()` with
`flex_1 min_h(0) flex_col`.

**Tests:** existing `tables_from` and page tests keep passing. Add
`subtitle_reports_running_and_stopped` and
`overview_cards_exist_for_every_list_page` only if they can be pure (no GPUI
window). Otherwise rely on compile and manual pass.

Commit: `feat: rebuild the app shell around an overview strip`.

---

## Task 7 — Table card rendering

**Files:** `src/ui/root.rs`

Inside `style::card()`:

- Header 40px, padding 20px, kicker 10/600 uppercase dim, bottom border
  `border_subtle`. Active sort shows a `↑`/`↓` accent arrow. Empty action
  header keeps the action width.
- Rows 44px, padding 20px, bottom rule `border_subtle`, `.w_full()`, pointer
  cursor, hover fill. Selected: `selected` surface + 3px accent bar pinned to
  the left edge (absolute) and text brightens; hover must not override the
  selected fill.
- Cell styles:
  - `Text`: 13/400 `text`; first column 13/500.
  - `Mono`: 12/400, font family "JetBrains Mono", dim for IDs/ports/mountpoints.
  - `Timestamp`: 11/400 dim.
  - `Pill`: `state_pill`; pending container action renders the pending label in
    a paused-colored pill.
- Action cluster: 28×28 icon buttons, 14px icons, gap 2; at rest 35% opacity,
  100% on row hover or selection; Remove red on hover; Image keeps Run.
- Empty/error state inside the card: centered 40px icon tile, 15/600 title,
  12/dim body max 360px. No new buttons.
- Error banner and pull strip: 36px, 24px padding, soft backgrounds
  (`error_soft` / `accent_soft`), bottom border, 12px text, 120×4 progress bar.
- Toast: 440px bottom center, panel/error-soft, icon + 12px text, radius 14,
  shadow, close button.

Commit: `feat: card the table with pills and hover actions`.

---

## Task 8 — Detail page: hero, segmented tabs, inspect cards, logs

**Files:** `src/ui/root.rs`, `src/ui/detail.rs`

1. Top bar 60px: back icon button 28×28 + list name 14/600; right host chip
   (same markup as list, extracted to a small helper). No overview strip.
2. Hero card (`style::card()`, 20px padding, section margin below): 44px icon
   tile, name 22/600, meta line = state pill + mono short id + image (containers)
   + created label. Right: labeled action buttons Start/Stop/Restart plus red
   ghost Remove, all 32px with 14px icons; disabled stay visible and dim.
   Pending label in amber under the actions.
3. Tabs: segmented control on app background, 3px padding, radius 10, 30px
   tabs, selected `panel_raised` + text, unselected dim. No underline.
4. Body card `style::card()` fills remaining height.
   - Inspect: split `build_tree(&json, 2)` nodes into top-level scalars
     (`value.is_some()`) and groups. Scalars -> one "Overview" card as a
     two-column 32px key/value grid in `build_tree` order (200px dim key, mono
     value). Each group -> its own card with an 11/600 uppercase header and
     32px rows (200px dim key, mono value, subtle row rules). Nested values stay
     one summarized cell via the existing `summarize()`/`VALUE_CLIP = 200`.
     No recursion, dotted keys or curation.
   - Logs: 44px toolbar with line count, stderr count, Reload ghost button;
     16px terminal padding, mono 12/20, 10px faint timestamps, stderr amber,
     horizontal scroll. Keep the 500-line tail.
   - Loading: centered 12px dim label. Error: 40px tile, message, 32px Retry.
     Missing: same card language.
5. Delete the old page-level action buttons from the detail top bar; keep the
   same handlers.

Commit: `feat: give detail pages a hero and carded inspect body`.

---

## Task 9 — Dialogs, popovers, menus, text inputs

**Files:** `src/ui/dialog.rs`, `src/ui/widgets/text_input.rs`, `src/ui/root.rs`

- Confirm: 460px, radius 14, 20px padding, 32px danger icon tile, 16/600 title,
  13/dim body, force row as a 12px card, footer ghost Cancel + red Remove.
- Run: 680px wide, max height 600, header/body 20px, 20px section gaps, 10/600
  uppercase section labels, 34px inputs, accent text links for add, footer
  with ghost Cancel + accent Run.
- Pull popover: 360px, 16px padding, anchored below the top bar (top 64/56).
- Host menu: 380px, 8px padding, 52px rows, status dot, name, mono endpoint,
  active check.
- Row context menu: match panel radius/border/shadow and 32px rows.
- Text input: height 34, radius 10, 13px text, accent focus border, 12px mono
  when `mono`.
- Keep every handler, disabled rule and error message unchanged.

Commit: `feat: restyle dialogs and menus to match the dashboard`.

---

## Task 10 — Verification and polish

1. `cargo fmt` (if the repo uses it), `cargo test --lib`, `cargo build`.
2. Restart the app. Manual pass at 1280×820:
   - margins, overview numbers, card navigation
   - selected row bar + hover actions align with header
   - every container state pill
   - detail inspect groups and logs
   - run/confirm/pull/host menu/toast
   - ⌘1/2/3, arrows, Enter, Esc, ⌘F, ⌘R, ⌘N, ⌘⇧P, right-click menu
3. Resize to 960×600 and repeat. Confirm at least seven full rows are visible
   with the pull strip or error banner shown; no horizontal clipping.
4. Fix any clipping or misalignment found. Do not add timers or fake data.

Commit: `polish: verify dashboard at default and minimum window sizes`.

---

## Acceptance checklist

- [ ] No full-bleed table; 24px page margins (16 compact).
- [ ] Overview strip shows live, correct, non-invented numbers on every list.
- [ ] Cards navigate; active card styled with accent border + top rule.
- [ ] Table card: 40px header, 44px rows, pills, aligned columns, selected bar.
- [ ] Row rows are full width (`.w_full()`), fixing the screenshot bug.
- [ ] Detail hero, segmented tabs, mechanical inspect cards, terminal logs.
- [ ] Dialogs/menus/toasts match the system.
- [ ] All existing library tests pass and new stats/theme/layout tests cover
      the added logic.
- [ ] No new crates, timers, animations, polling or invented data.
