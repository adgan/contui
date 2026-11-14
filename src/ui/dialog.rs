use crate::docker::model::ResourceKind;
use crate::theme::Theme;
use crate::ui::mutations::{confirm_copy, ConfirmState};
use crate::ui::root::RootView;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, MouseButton, SharedString};

pub fn confirm(confirm: &ConfirmState, theme: &Theme, cx: &mut Context<RootView>) -> AnyElement {
    let (title, body) = confirm_copy(confirm.kind, &confirm.title, confirm.tag_count);

    let backdrop = div()
        .absolute()
        .inset_0()
        .bg(gpui::rgba(0x00000099))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event, _window, cx| {
                this.cancel_confirm();
                cx.notify();
            }),
        );

    let mut panel = div()
        .w(px(440.))
        .p_4()
        .occlude()
        .flex()
        .flex_col()
        .gap_3()
        .rounded_lg()
        .bg(theme.panel)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .child(div().text_base().child(title))
        .child(div().text_sm().text_color(theme.text_dim).child(body));

    if confirm.kind != ResourceKind::Volume {
        let (label, hint) = match confirm.kind {
            ResourceKind::Container => (
                "Force remove while running",
                "Sends SIGKILL instead of stopping the container first.",
            ),
            _ => (
                "Force remove even if a container uses it",
                "Running containers keep running, but cannot be restarted.",
            ),
        };
        let checked = confirm.force;
        let box_color = if checked { theme.accent } else { theme.border };
        let check_color = if checked { theme.bg } else { theme.panel };
        panel = panel.child(
            div()
                .id("confirm-force")
                .flex()
                .items_start()
                .gap_2()
                .cursor_pointer()
                .on_click(cx.listener(|this, _event, _window, cx| {
                    if let Some(confirm) = &mut this.mutations.confirm {
                        confirm.force = !confirm.force;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .mt(px(1.))
                        .size(px(14.))
                        .rounded_sm()
                        .border_1()
                        .border_color(box_color)
                        .bg(box_color)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(check_color)
                        .child(if checked { "✓" } else { "" }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().child(label))
                        .child(div().text_xs().text_color(theme.text_dim).child(hint)),
                ),
        );
    }

    if let Some(error) = &confirm.error {
        panel = panel.child(div().text_xs().text_color(theme.error).child(error.clone()));
    }

    let submitting = confirm.submitting;
    let cancel_color = if submitting { theme.text_dim } else { theme.text };
    let mut cancel = div()
        .id("confirm-cancel")
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .text_sm()
        .text_color(cancel_color)
        .child("Cancel");
    if !submitting {
        cancel = cancel
            .cursor_pointer()
            .hover(|s| s.bg(theme.hover))
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.cancel_confirm();
                cx.notify();
            }));
    }

    let remove_label = if submitting { "Removing…" } else { "Remove" };
    let mut remove = div()
        .id("confirm-remove")
        .px_3()
        .py_1()
        .rounded_md()
        .bg(if submitting { theme.border } else { theme.error })
        .text_sm()
        .text_color(theme.bg)
        .child(remove_label);
    if !submitting {
        remove = remove
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.submit_confirm();
                cx.notify();
            }));
    }

    panel = panel.child(
        div()
            .flex()
            .justify_end()
            .gap_2()
            .child(cancel)
            .child(remove),
    );

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(backdrop)
        .child(panel)
        .into_any_element()
}

pub fn menu_separator(theme: &Theme) -> gpui::Div {
    div().h(px(1.)).my_1().bg(theme.border)
}

pub fn menu_label(theme: &Theme, label: SharedString) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(theme.text_dim)
        .child(label)
}

pub fn pull_popover(
    reference: &str,
    error: Option<&str>,
    pulling: bool,
    focus: &gpui::FocusHandle,
    focused: bool,
    theme: &Theme,
    cx: &mut Context<RootView>,
) -> AnyElement {
    use crate::docker::service::PullOrigin;
    use crate::ui::widgets::text_input;

    let can_pull = !reference.trim().is_empty() && !pulling;
    let input = text_input::field(
        "pull-input",
        reference,
        "image[:tag]",
        focus,
        focused,
        error.is_some(),
        pulling,
        true,
        theme,
    )
    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _window, cx| {
        if event.keystroke.key == "enter" {
            let reference = this.mutations.pull_input.clone();
            this.start_pull(&reference, PullOrigin::Header);
            cx.notify();
            return;
        }
        if text_input::edit_text(&mut this.mutations.pull_input, event, cx) {
            this.mutations.pull_error = None;
            cx.notify();
        }
    }));

    let mut submit = div()
        .id("pull-submit")
        .px_3()
        .py_1()
        .rounded_md()
        .text_sm()
        .text_color(if can_pull { theme.bg } else { theme.text_dim })
        .bg(if can_pull { theme.accent } else { theme.border })
        .child(if pulling { "Pulling…" } else { "Pull" });
    if can_pull {
        submit = submit
            .cursor_pointer()
            .hover(|style| style.opacity(0.9))
            .on_click(cx.listener(|this, _event, _window, cx| {
                let reference = this.mutations.pull_input.clone();
                this.start_pull(&reference, PullOrigin::Header);
                cx.notify();
            }));
    }

    let mut panel = div()
        .absolute()
        .top(px(60.))
        .right(px(16.))
        .w(px(360.))
        .p_3()
        .occlude()
        .flex()
        .flex_col()
        .gap_2()
        .rounded_md()
        .bg(theme.panel)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .child(
            div()
                .text_xs()
                .text_color(theme.text_dim)
                .child("Pull an image from the registry"),
        )
        .child(input);

    if let Some(error) = error {
        panel = panel.child(div().text_xs().text_color(theme.error).child(error.to_string()));
    }
    panel = panel.child(div().flex().justify_end().child(submit));

    panel.into_any_element()
}

