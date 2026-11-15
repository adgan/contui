# Table readability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One aligned State column, two-line wrapping for the name column, and a hover tooltip with copy for the full name.

**Architecture:** Column widths stop using content as their flex basis, so header and every row divide leftover space identically. Rows grow from 44px to 52px so the first text column can clamp to two lines inside the same uniform height. A small `CellTooltip` view is attached with GPUI's hoverable tooltip; the row context menu gains "Copy name".

**Tech Stack:** Rust 2024, GPUI 0.2.2, `uniform_list`, bollard (untouched).

**Spec:** `docs/superpowers/specs/2026-11-15-table-readability-design.md`

## Global Constraints

- GPUI `=0.2.2`; no new crates.
- Every list row and group header stays one uniform height — `uniform_list` only.
- Minimum window 960×600; no clipping, overlap or horizontal scrolling at that size.
- `src/theme.rs` remains the only source of colours, spacing and type scale.
- Column widths stay declared as `ColumnWidth::Fixed` / `ColumnWidth::Flex(weight)`; do not hardcode pixel widths per page.
- Tests run with `cargo test --lib`; the app is built with `cargo build`.

## Review Focus

- A 200-character container name with no spaces: it must clamp to two lines with an ellipsis, never widen its column or reach under the State pill.
- Clicking Copy inside the tooltip: it must put the full name on the clipboard and must not open the detail page, dismiss the tooltip before the click lands, or shift the row selection.
- The narrowest supported window (960×600): header and pills must still line up, and the name must still get two readable lines rather than one clipped one.
- Grouped containers: group headers and data rows must stay the same height and the collapse chevrons must stay vertically centred.
- Empty and filtered lists: the empty state and a one-row search result must render exactly as before.

---

### Task 1: True proportional columns

**Files:**
- Modify: `src/ui/table.rs` (add `ColumnLayout` + `column_layout`, tests at the bottom)
- Modify: `src/ui/root/view.rs:2108-2117` (`MapWidth::map_width`)

**Interfaces:**
- Consumes: `ColumnWidth` from `src/ui/table.rs`.
- Produces: `pub struct ColumnLayout { pub fixed: Option<f32>, pub grow: f32 }` and `pub fn column_layout(width: ColumnWidth) -> ColumnLayout` in `src/ui/table.rs`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/ui/table.rs`:

```rust
#[test]
fn column_layout_fixes_fixed_columns_and_weights_flex_columns() {
    assert_eq!(
        column_layout(ColumnWidth::Fixed(104.0)),
        ColumnLayout {
            fixed: Some(104.0),
            grow: 0.0
        }
    );
    let flex = column_layout(ColumnWidth::Flex(2.2));
    assert_eq!(flex.fixed, None);
    assert_eq!(flex.grow, 2.2);
    assert_eq!(column_layout(ColumnWidth::Flex(1.0)).grow, 1.0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib column_layout_fixes_fixed_columns_and_weights_flex_columns`
Expected: FAIL with "cannot find function `column_layout`".

- [ ] **Step 3: Implement `ColumnLayout` and `column_layout`**

Place next to `ColumnWidth` in `src/ui/table.rs`:

```rust
/// How a declared width becomes flex geometry: fixed pixels, or a grow weight
/// used with a zero basis so every row divides the leftover space the same way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnLayout {
    pub fixed: Option<f32>,
    pub grow: f32,
}

pub fn column_layout(width: ColumnWidth) -> ColumnLayout {
    match width {
        ColumnWidth::Fixed(value) => ColumnLayout {
            fixed: Some(value),
            grow: 0.0,
        },
        ColumnWidth::Flex(weight) => ColumnLayout {
            fixed: None,
            grow: weight,
        },
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib column_layout_fixes_fixed_columns_and_weights_flex_columns`
Expected: PASS.

- [ ] **Step 5: Use the mapping in `MapWidth::map_width`**

Replace the body in `src/ui/root/view.rs` so flex columns have no content basis:

```rust
fn map_width(self, width: ColumnWidth) -> Self {
    let layout = crate::ui::table::column_layout(width);
    match layout.fixed {
        Some(value) => self.w(px(value)).flex_shrink_0(),
        None => {
            let mut styled = self.min_w(px(0.)).flex_basis(px(0.));
            styled.style().flex_grow = Some(layout.grow);
            styled
        }
    }
}
```

`Styled::style()` is public; it is how a non-round grow weight is set, because
`Styled::flex_grow()` only accepts the implicit weight 1.

- [ ] **Step 6: Verify the crate**

Run: `cargo test --lib && cargo build`
Expected: all tests pass, build succeeds.

- [ ] **Step 7: Commit**

```bash
git add src/ui/table.rs src/ui/root/view.rs
git commit -m "fix(ui): give table columns a zero flex basis so they line up"
```

---

### Task 2: 52px uniform rows

**Files:**
- Modify: `src/ui/style.rs:31` and `src/ui/style.rs:68-80`
- Modify: `src/ui/root.rs:920-921` and `src/ui/root/view.rs:2146-2148` (comments only)

**Interfaces:**
- Produces: `Layout::row_height == 52.0` for both compact and full layouts; every consumer (`render_table`, `group_header`, `ungrouped_header`) already reads it.

- [ ] **Step 1: Update the failing assertions**

In `src/ui/style.rs` tests:

```rust
assert_eq!(full.row_height, 52.0);
assert_eq!(compact.row_height, 52.0);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib layout_heights_match_the_design`
Expected: FAIL, left `44.0`, right `52.0`.

- [ ] **Step 3: Set the row height**

In `Layout::for_viewport`, change `row_height: 44.0` to `row_height: 52.0`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib layout_heights_match_the_design`
Expected: PASS.

- [ ] **Step 5: Update the two comments that name 44px**

`src/ui/root.rs` `ContainerSlot` doc: "Group headers are 52px like data rows".
`src/ui/root/view.rs` `SECTION_GAP` doc: "It is part of the 52px header".

- [ ] **Step 6: Verify and commit**

Run: `cargo test --lib && cargo build`
Expected: PASS and build succeeds.

```bash
git add src/ui/style.rs src/ui/root.rs src/ui/root/view.rs
git commit -m "feat(ui): raise table rows to 52px for two-line names"
```

---

### Task 3: First column wraps to two lines

**Files:**
- Modify: `src/ui/table.rs` (add `cell_wraps`, test at the bottom)
- Modify: `src/ui/root/view.rs:1963-1991` (cell text construction)

**Interfaces:**
- Consumes: `CellStyle`, `MapWidth::map_width` from Task 1.
- Produces: `pub fn cell_wraps(column: usize, style: CellStyle) -> bool` in `src/ui/table.rs`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/ui/table.rs`:

```rust
#[test]
fn only_the_first_text_column_wraps() {
    assert!(cell_wraps(0, CellStyle::Text));
    assert!(!cell_wraps(0, CellStyle::Mono));
    assert!(!cell_wraps(0, CellStyle::Timestamp));
    assert!(!cell_wraps(0, CellStyle::Pill));
    assert!(!cell_wraps(1, CellStyle::Text));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib only_the_first_text_column_wraps`
Expected: FAIL with "cannot find function `cell_wraps`".

- [ ] **Step 3: Implement `cell_wraps`**

Add next to `CellStyle` in `src/ui/table.rs`:

```rust
/// The first text column — container name, image repository, volume name — is
/// the only cell allowed to wrap. Everything else stays a single ellipsized
/// line so row heights stay uniform.
pub fn cell_wraps(column: usize, style: CellStyle) -> bool {
    column == 0 && style == CellStyle::Text
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib only_the_first_text_column_wraps`
Expected: PASS.

- [ ] **Step 5: Clamp the first column instead of truncating it**

In `src/ui/root/view.rs`, import `cell_wraps` and replace the start of the text-cell
block (currently `let mut text = div().map_width(width).truncate().child(cell.clone());`)
with:

```rust
let mut text = div()
    .id(SharedString::from(format!("cell-{row_id}-{column}")))
    .map_width(width);
if cell_wraps(column, style) {
    text = text.line_clamp(2).text_ellipsis();
} else {
    text = text.truncate();
}
text = text.child(cell.clone());
```

Do not combine `truncate()` with `line_clamp()`: `truncate()` sets `nowrap`, which
disables wrapping. `line_clamp(2)` plus `text_ellipsis()` gives an ellipsis on the
end of the second line. The rest of the `match style` block is unchanged.

- [ ] **Step 6: Verify**

Run: `cargo test --lib && cargo build`
Expected: PASS, build succeeds.

- [ ] **Step 7: Manual check at both widths**

Run the app (`cargo run`) at 1280×820 and at the 960×600 minimum. Expected: k8s
names use two lines, ellipsize when still too long, never cross into the State
column, and State pills form one straight column under the STATE header.

- [ ] **Step 8: Commit**

```bash
git add src/ui/table.rs src/ui/root/view.rs
git commit -m "feat(ui): wrap the first table column to two lines"
```

---

### Task 4: Hover tooltip with copy

**Files:**
- Create: `src/ui/widgets/tooltip.rs`
- Modify: `src/ui/widgets/mod.rs`
- Modify: `src/ui/root/view.rs` (attach to the first-column cell from Task 3)

**Interfaces:**
- Consumes: `Theme` from `src/theme.rs`.
- Produces: `pub struct CellTooltip` with `pub fn new(text: SharedString, theme: Theme) -> Self`, implementing `Render`.

- [ ] **Step 1: Create the tooltip view**

`src/ui/widgets/tooltip.rs`:

```rust
use crate::theme::Theme;
use gpui::prelude::*;
use gpui::{ClipboardItem, Context, FontWeight, Render, SharedString, Window, div, px};

/// The full first-column value. GPUI shows it after the tooltip delay and keeps
/// it open while the pointer is inside, so Copy is reachable.
pub struct CellTooltip {
    text: SharedString,
    theme: Theme,
    copied: bool,
}

impl CellTooltip {
    pub fn new(text: SharedString, theme: Theme) -> Self {
        Self {
            text,
            theme,
            copied: false,
        }
    }
}

impl Render for CellTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let hover_bg = theme.panel;
        div()
            .max_w(px(360.))
            .p(px(10.))
            .rounded(px(10.))
            .bg(theme.panel_raised)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(theme.text)
                    .child(self.text.clone()),
            )
            .child(
                div().flex().items_center().child(
                    div()
                        .id("tooltip-copy")
                        .h(px(24.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .occlude()
                        .cursor_pointer()
                        .bg(if self.copied {
                            theme.running_soft
                        } else {
                            theme.hover
                        })
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if self.copied { theme.running } else { theme.text })
                        .hover(move |style| style.bg(hover_bg))
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                this.text.to_string(),
                            ));
                            this.copied = true;
                            cx.notify();
                        }))
                        .child(if self.copied { "Copied" } else { "Copy" }),
                ),
            )
    }
}
```

- [ ] **Step 2: Register the module**

In `src/ui/widgets/mod.rs` add `pub mod tooltip;`.

- [ ] **Step 3: Attach it to the first-column cell**

In `src/ui/root/view.rs`, import `CellTooltip`, then in the wrapping branch from
Task 3, before `line_clamp`:

```rust
if cell_wraps(column, style) {
    let full = cell.to_string();
    let tip_theme = theme.clone();
    text = text.hoverable_tooltip(move |_window, cx| -> gpui::AnyView {
        cx.new(|_cx| CellTooltip::new(full.clone().into(), tip_theme.clone()))
            .into()
    });
    text = text.line_clamp(2).text_ellipsis();
}
```

`hoverable_tooltip` keeps the tooltip alive while the pointer is inside it, which
is what makes the Copy button clickable; plain `tooltip` does not.

- [ ] **Step 4: Verify the crate**

Run: `cargo build && cargo test --lib`
Expected: build succeeds, all tests pass.

- [ ] **Step 5: Manual check**

Run the app. Hover a container name: after ~0.5s the full name appears in a panel
with a Copy button. Press Copy: label reads "Copied", the clipboard holds the full
name (paste into a terminal to confirm), and the detail page does not open. Move
from the cell into the panel: it must stay visible.

If the Copy click does not register inside the tooltip, stop and report it rather
than working around it: the agreed fallback is a copy icon in the row action
cluster, which is a different design.

- [ ] **Step 6: Commit**

```bash
git add src/ui/widgets/tooltip.rs src/ui/widgets/mod.rs src/ui/root/view.rs
git commit -m "feat(ui): show the full name in a hover tooltip with copy"
```

---

### Task 5: Copy name in the row menu

**Files:**
- Modify: `src/ui/root/view.rs:173-182` (`render_row_menu`)

**Interfaces:**
- Consumes: `row_menu_item`, the `title` already read from `menu.id`, and `${kind}`.

- [ ] **Step 1: Add the menu item**

Immediately after the existing "Open" item in `render_row_menu`:

```rust
let copy_title = title.clone();
panel = panel.child(row_menu_item(
    "Copy name",
    theme.text_dim,
    &theme,
    cx,
    move |_this, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_title.clone()));
    },
));
```

- [ ] **Step 2: Verify the crate**

Run: `cargo build && cargo test --lib`
Expected: build succeeds, all tests pass.

- [ ] **Step 3: Manual check**

Right-click any container, image and volume row. "Copy name" appears above the
separator, closes the menu, and pastes the full name shown in the first column.

- [ ] **Step 4: Commit**

```bash
git add src/ui/root/view.rs
git commit -m "feat(ui): add Copy name to the row context menu"
```

---

## Self-review notes

- Spec coverage: alignment (Task 1), two-line wrapping (Tasks 2, 3), tooltip + copy
  (Task 4), context-menu copy (Task 5), unchanged sorting/filtering/grouping (no
  task touches them).
- No new crates; the tooltip reuses GPUI's clipboard and tooltip support.
- Names used across tasks are consistent: `column_layout`, `ColumnLayout`,
  `cell_wraps`, `CellTooltip::new`.
