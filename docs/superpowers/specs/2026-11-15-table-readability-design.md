# contUI — table readability: one State column, wrapped names, copy on hover — design

Date: 2026-11-15
Status: awaiting approval

## Intent

In the containers list, long container names run underneath the state pill, and the
pill sits at a different x on every row. Two separate causes:

1. Flexible columns grow from their content, so each row's Name cell is as wide as
   its own text and pushes State to the right (`MapWidth` in `src/ui/root/view.rs`).
2. Rows are a fixed 44px `uniform_list`, so text can only clip. Kubernetes names are
   unreadable and there is no way to recover one in full.

The fix should make the list scannable again: State is one true column, a long name
uses a second line instead of running into the next column, and anything still cut
off is available on hover and can be copied.

## Goals

1. Every column lines up with the header and with every other row, at every window
   width down to 960px.
2. The first column (container name, image repository, volume name) wraps to two
   lines and ellipsizes only if two lines are still not enough.
3. Rows stay one uniform height, so `uniform_list` and its fast scrolling stay.
4. Hovering a name shows the full value in a tooltip with a Copy button.
   Right-clicking a row also offers "Copy name".
5. Sorting, filtering, grouping, selection, hover actions and row clicks behave
   exactly as today.

## Non-goals

- No variable-height list, no per-row measured heights.
- No wrapping in mono or timestamp cells; they keep single-line ellipsis.
- No new columns, no column resizing, no horizontal scrolling.
- No light mode, no new crates.

## Anti-goals

- Rows tall enough that noticeably fewer containers fit on screen (52px is the
  ceiling; not 64).
- Tooltips that flash while scrolling or while moving along a row.
- A Copy control that swallows the row click or opens the detail page.
- Fixing alignment by hardcoding pixel widths that break at narrow widths.

## Constraints

- GPUI 0.2.2, Rust 2024, `uniform_list`, minimum window 960×600.
- Group headers and data rows share `Layout::row_height`; both get the new height.
- `theme.rs` stays the only source of colours and type scale.
- Column widths stay proportional (`ColumnWidth::Flex(weight)`), not hand-computed.

## Approaches considered

**A. Keep 44px single-line rows, add hover + copy only.** Cheapest, zero layout
risk, but it does not wrap anything and leaves the main complaint — long names —
unanswered. Rejected.

**B. Two-line first column, 52px uniform rows, zero-basis flex (chosen).** Names
gain a second line, every column gets a real proportional width, and the existing
list element is untouched. Costs ~8px of extra row height and one new tooltip view.

**C. Variable-height rows via GPUI's `list`.** Every cell can wrap fully, but live
snapshot refreshes re-measure rows, scroll position shifts under events, and the
group-header/selection code assumes uniform rows. Too much risk for a readability
fix. Rejected.

## Design

**Column alignment.** `MapWidth::map_width` stops using content as the flex basis:
flex columns become `flex_basis(px(0.))`, `min_w(px(0.))`, and `flex_grow` set to
the declared weight (the `weight` currently ignored). Header and row cells use the
same function, so they divide the same leftover space identically. Fixed columns
(State 104, Ports 132, Created 100, actions) are unchanged. State stays centred in
its cell, so pills become a straight column.

**Row height.** `Layout::row_height` goes 44 → 52 in both compact and full layouts.
Two 18px lines plus 8px above and below. `SECTION_GAP` and the group headers keep
their construction; they simply inherit the taller row.

**Wrapping.** A new pure helper decides per cell: the first column with
`CellStyle::Text` wraps, everything else truncates. Wrapped cells use
`.line_clamp(2).text_ellipsis()` and the existing 13px/18px text style, so the
second line ends with an ellipsis when two lines are not enough. `truncate()` must
not be combined with the clamp (it forces nowrap).

**Hover + copy.** New `src/ui/widgets/tooltip.rs`: a `CellTooltip` view holding the
full text and theme, rendered as a small raised panel (11–13px text, wraps up to
~360px, and a Copy button). It is attached to the first-column cell with GPUI's
`hoverable_tooltip`, which waits 500ms before showing, stays open while the pointer
is inside it, and lets the button receive the click. Copy writes
`ClipboardItem::new_string(text)`; the button label flips to "Copied" until the
tooltip closes. The row's context menu gains a "Copy name" item at the top, doing
the same clipboard write, so copying does not depend on hover.

**What stays exactly as it is.** Text styles per column, pill colours, row
selection bar, hover action cluster, group headers, `TableSpec` weights, sorting and
filtering.

## Testing

- Unit: `cell_wraps(column, style)` is true only for the first text column;
  `MapWidth` resolves `Flex(weight)` to grow weight `weight` with zero basis and
  `Fixed(w)` to `w` with no grow; `Layout::row_height` is 52 for compact and full.
- Existing table, layout, pill and mutation tests must keep passing.
- Manual: run at 1280×820 and 960×600. Check State pills share one x across rows
  and the header; a k8s name wraps to two lines and ellipsizes at 2; hovering shows
  the full name after the delay and Copy puts it on the clipboard; clicking the row
  still opens the detail page; group headers and rows are the same height.

## Left to the builder

- Exact tooltip padding and max width, within the theme's spacing scale.
- Whether "Copied" resets on pointer re-entry or only when the tooltip closes.
- Whether the context-menu copy item is worth keeping if the tooltip feels enough.
