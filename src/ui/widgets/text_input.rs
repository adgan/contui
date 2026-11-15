use crate::theme::Theme;
use gpui::prelude::*;
use gpui::{div, px, App, FocusHandle, KeyDownEvent, Stateful};

/// The pure editing rule behind every single-line field. The caret is always at
/// the end; paste is the only multi-character insert and stays on one line.
pub fn apply_edit(
    text: &mut String,
    key: &str,
    platform: bool,
    control: bool,
    pasted: Option<String>,
) -> bool {
    if platform && key == "v" {
        let Some(pasted) = pasted else {
            return false;
        };
        let cleaned: String = pasted
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        if cleaned.is_empty() {
            return false;
        }
        text.push_str(&cleaned);
        return true;
    }
    if platform || control {
        return false;
    }
    match key {
        "backspace" => text.pop().is_some(),
        "space" => {
            text.push(' ');
            true
        }
        "tab" | "enter" | "escape" => false,
        key if key.chars().count() == 1 => {
            text.push_str(key);
            true
        }
        _ => false,
    }
}

pub fn edit_text(text: &mut String, event: &KeyDownEvent, cx: &mut App) -> bool {
    let platform = event.keystroke.modifiers.platform;
    let control = event.keystroke.modifiers.control;
    let pasted = if platform && event.keystroke.key == "v" {
        cx.read_from_clipboard().and_then(|item| item.text())
    } else {
        None
    };
    apply_edit(text, &event.keystroke.key, platform, control, pasted)
}

/// Styling flags for one field, grouped so callers never pass a run of bare
/// booleans whose order is easy to get wrong.
#[derive(Clone, Copy, Debug, Default)]
pub struct FieldStyle {
    pub focused: bool,
    pub error: bool,
    pub read_only: bool,
    pub mono: bool,
}

/// Visuals for one input row. The caller owns the key handler, because only it
/// knows which field of the form this input edits.
pub fn field(
    id: impl Into<gpui::ElementId>,
    text: &str,
    placeholder: &str,
    focus: &FocusHandle,
    style: FieldStyle,
    theme: &Theme,
) -> Stateful<gpui::Div> {
    let FieldStyle {
        focused,
        error,
        read_only,
        mono,
    } = style;
    let border = if error {
        theme.error
    } else if focused {
        theme.accent
    } else {
        theme.border
    };
    let color = if text.is_empty() || read_only {
        theme.text_dim
    } else {
        theme.text
    };
    let shown = if text.is_empty() { placeholder } else { text };
    let focus_for_click = focus.clone();

    let mut element = div()
        .id(id)
        .h(px(28.))
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .rounded_md()
        .bg(theme.bg)
        .border_1()
        .border_color(border)
        .text_sm()
        .text_color(color)
        .track_focus(focus)
        .cursor_text()
        .on_click(move |_, window, _| window.focus(&focus_for_click))
        .child(div().truncate().child(shown.to_string()));

    if mono {
        element = element.font_family("JetBrains Mono");
    }
    if focused && !read_only {
        element = element.child(div().w(px(1.)).h(px(14.)).bg(theme.accent));
    }
    element
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_text_at_the_end() {
        let mut s = String::from("ngin");
        assert!(apply_edit(&mut s, "x", false, false, None));
        assert_eq!(s, "nginx");
        assert!(apply_edit(&mut s, "space", false, false, None));
        assert_eq!(s, "nginx ");
        assert!(apply_edit(&mut s, "backspace", false, false, None));
        assert_eq!(s, "nginx");
    }

    #[test]
    fn ignores_navigation_and_shortcuts_but_pastes() {
        let mut s = String::new();
        assert!(!apply_edit(&mut s, "tab", false, false, None));
        assert!(!apply_edit(&mut s, "enter", false, false, None));
        assert!(!apply_edit(&mut s, "left", false, false, None));
        assert!(!apply_edit(&mut s, "a", true, false, None)); // cmd-a
        assert!(!apply_edit(&mut s, "a", false, true, None)); // ctrl-a
        assert!(apply_edit(&mut s, "v", true, false, Some("a\nb".into())));
        assert_eq!(
            s, "a b",
            "a pasted newline becomes a space in a single-line field"
        );
    }

    #[test]
    fn empty_text_ignores_backspace() {
        let mut s = String::new();
        assert!(!apply_edit(&mut s, "backspace", false, false, None));
        assert_eq!(s, "");
    }
}
