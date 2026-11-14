use crate::docker::actions::{Proto, RunField};
use crate::docker::model::ResourceKind;
use crate::theme::Theme;
use crate::ui::mutations::{confirm_copy, ConfirmState, RunState};
use crate::ui::root::RootView;
use crate::ui::widgets::text_input;
use gpui::prelude::*;
use gpui::{
    div, px, svg, AnyElement, Context, FocusHandle, KeyDownEvent, MouseButton, SharedString, Window,
};

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


#[derive(Clone)]
pub struct RunFocus {
    pub image: FocusHandle,
    pub name: FocusHandle,
    pub command: FocusHandle,
    pub ports: Vec<(FocusHandle, FocusHandle)>,
    pub env: Vec<(FocusHandle, FocusHandle)>,
    pub mounts: Vec<(FocusHandle, FocusHandle)>,
}

fn tab_focus(cx: &mut Context<RootView>) -> FocusHandle {
    cx.focus_handle().tab_stop(true)
}

impl RunFocus {
    pub fn new(cx: &mut Context<RootView>) -> Self {
        Self {
            image: tab_focus(cx),
            name: tab_focus(cx),
            command: tab_focus(cx),
            ports: Vec::new(),
            env: Vec::new(),
            mounts: Vec::new(),
        }
    }

    pub fn add_port(&mut self, cx: &mut Context<RootView>) {
        let host = tab_focus(cx);
        let container = tab_focus(cx);
        self.ports.push((host, container));
    }

    pub fn remove_port(&mut self, index: usize) {
        if index < self.ports.len() {
            self.ports.remove(index);
        }
    }

    pub fn add_env(&mut self, cx: &mut Context<RootView>) {
        let key = tab_focus(cx);
        let value = tab_focus(cx);
        self.env.push((key, value));
    }

    pub fn remove_env(&mut self, index: usize) {
        if index < self.env.len() {
            self.env.remove(index);
        }
    }

    pub fn add_mount(&mut self, cx: &mut Context<RootView>) {
        let source = tab_focus(cx);
        let target = tab_focus(cx);
        self.mounts.push((source, target));
    }

    pub fn remove_mount(&mut self, index: usize) {
        if index < self.mounts.len() {
            self.mounts.remove(index);
        }
    }

    pub fn handle(&self, field: RunField) -> Option<FocusHandle> {
        match field {
            RunField::Image => Some(self.image.clone()),
            RunField::Name => Some(self.name.clone()),
            RunField::Command => Some(self.command.clone()),
            RunField::PortHost(index) => self.ports.get(index).map(|pair| pair.0.clone()),
            RunField::PortContainer(index) => self.ports.get(index).map(|pair| pair.1.clone()),
            RunField::EnvKey(index) => self.env.get(index).map(|pair| pair.0.clone()),
            RunField::EnvValue(index) => self.env.get(index).map(|pair| pair.1.clone()),
            RunField::MountSource(index) => self.mounts.get(index).map(|pair| pair.0.clone()),
            RunField::MountTarget(index) => self.mounts.get(index).map(|pair| pair.1.clone()),
        }
    }
}

fn field_id(field: RunField) -> SharedString {
    SharedString::from(format!("run-{field:?}"))
}

fn field_error(run: &RunState, field: RunField) -> Option<String> {
    run.field_errors
        .iter()
        .find(|error| error.field == field)
        .map(|error| error.message.clone())
}

fn input_field(
    field: RunField,
    text: &str,
    placeholder: &str,
    focus: &FocusHandle,
    error: Option<String>,
    read_only: bool,
    mono: bool,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<RootView>,
) -> gpui::Stateful<gpui::Div> {
    text_input::field(
        field_id(field),
        text,
        placeholder,
        focus,
        focus.is_focused(window),
        error.is_some(),
        read_only,
        mono,
        theme,
    )
    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
        let key = event.keystroke.key.as_str();
        if key == "tab" {
            if event.keystroke.modifiers.shift {
                window.focus_prev();
            } else {
                window.focus_next();
            }
            return;
        }
        if key == "enter" {
            this.submit_run(window, cx);
            return;
        }
        if let Some(target) = this.run_field_text_mut(field) {
            if text_input::edit_text(target, event, cx) {
                this.clear_run_field_error(field);
                cx.notify();
            }
        }
    }))
}

fn labeled(
    label: &'static str,
    field: RunField,
    text: &str,
    placeholder: &str,
    focus: &FocusHandle,
    error: Option<String>,
    read_only: bool,
    mono: bool,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<RootView>,
) -> gpui::Div {
    let mut column = div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(theme.text_dim).child(label))
        .child(input_field(
            field,
            text,
            placeholder,
            focus,
            error.clone(),
            read_only,
            mono,
            theme,
            window,
            cx,
        ));
    if let Some(message) = error {
        column = column.child(div().text_xs().text_color(theme.error).child(message));
    }
    column
}

fn section_header(
    title: &'static str,
    add_label: &'static str,
    disabled: bool,
    theme: &Theme,
    cx: &mut Context<RootView>,
    on_add: impl Fn(&mut RootView, &mut Context<RootView>) + 'static,
) -> gpui::Div {
    let mut add = div()
        .id(SharedString::from(format!("add-{add_label}")))
        .text_xs()
        .child(add_label)
        .text_color(if disabled { theme.text_dim } else { theme.accent });
    if !disabled {
        add = add.cursor_pointer().on_click(cx.listener(
            move |this, _event, _window, cx| {
                on_add(this, cx);
                cx.notify();
            },
        ));
    }
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_xs().text_color(theme.text_dim).child(title))
        .child(add)
}

fn toggle(
    id: String,
    label: &'static str,
    selected: bool,
    disabled: bool,
    theme: &Theme,
    cx: &mut Context<RootView>,
    on_click: impl Fn(&mut RootView) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let mut toggle = div()
        .id(SharedString::from(id))
        .px_2()
        .h(px(28.))
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(if selected { theme.accent } else { theme.border })
        .text_xs()
        .text_color(if selected && !disabled {
            theme.text
        } else {
            theme.text_dim
        })
        .child(label);
    if !disabled {
        toggle = toggle
            .cursor_pointer()
            .on_click(cx.listener(move |this, _event, _window, cx| {
                on_click(this);
                cx.notify();
            }));
    }
    toggle
}

fn remove_button(
    id: String,
    disabled: bool,
    theme: &Theme,
    cx: &mut Context<RootView>,
    on_click: impl Fn(&mut RootView) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let mut button = div()
        .id(SharedString::from(id))
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .child(svg().path("icons/close.svg").size(px(12.)).text_color(theme.text_dim));
    if !disabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(theme.hover))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                on_click(this);
                cx.notify();
            }));
    }
    button
}

pub fn run_dialog(
    run: &RunState,
    focus: &RunFocus,
    pulling: bool,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> AnyElement {
    let submitting = run.submitting;
    let read_only = submitting;

    let backdrop = div()
        .absolute()
        .inset_0()
        .bg(gpui::rgba(0x000000aa))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event, _window, cx| {
                this.cancel_run();
                cx.notify();
            }),
        );

    let mut body = div()
        .id("run-body")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p_4()
        .flex()
        .flex_col()
        .gap_4();

    body = body
        .child(labeled(
            "Image *",
            RunField::Image,
            &run.form.image,
            "postgres:18",
            &focus.image,
            field_error(run, RunField::Image),
            read_only,
            true,
            theme,
            window,
            cx,
        ))
        .child(labeled(
            "Name",
            RunField::Name,
            &run.form.name,
            "optional",
            &focus.name,
            field_error(run, RunField::Name),
            read_only,
            false,
            theme,
            window,
            cx,
        ))
        .child(labeled(
            "Command",
            RunField::Command,
            &run.form.command,
            "run the image default",
            &focus.command,
            field_error(run, RunField::Command),
            read_only,
            true,
            theme,
            window,
            cx,
        ));

    body = body.child(section_header(
        "Ports",
        "+ Add port",
        submitting,
        theme,
        cx,
        |this, cx| this.add_run_port(cx),
    ));
    for index in 0..run.form.ports.len() {
        let Some((host_focus, container_focus)) = focus.ports.get(index).cloned() else {
            continue;
        };
        let port = &run.form.ports[index];
        let mut row = div()
            .flex()
            .items_start()
            .gap_2()
            .child(div().w(px(92.)).child(input_field(
                RunField::PortHost(index),
                &port.host,
                "host",
                &host_focus,
                field_error(run, RunField::PortHost(index)),
                read_only,
                false,
                theme,
                window,
                cx,
            )))
            .child(div().w(px(96.)).child(input_field(
                RunField::PortContainer(index),
                &port.container,
                "container",
                &container_focus,
                field_error(run, RunField::PortContainer(index)),
                read_only,
                false,
                theme,
                window,
                cx,
            )))
            .child(toggle(
                format!("run-port-{index}-tcp"),
                "tcp",
                port.proto == Proto::Tcp,
                submitting,
                theme,
                cx,
                move |this| this.set_run_port_proto(index, Proto::Tcp),
            ))
            .child(toggle(
                format!("run-port-{index}-udp"),
                "udp",
                port.proto == Proto::Udp,
                submitting,
                theme,
                cx,
                move |this| this.set_run_port_proto(index, Proto::Udp),
            ))
            .child(div().flex_1())
            .child(remove_button(
                format!("run-remove-port-{index}"),
                submitting,
                theme,
                cx,
                move |this| this.remove_run_port(index),
            ));
        if let Some(message) = field_error(run, RunField::PortHost(index))
            .or_else(|| field_error(run, RunField::PortContainer(index)))
        {
            row = row.child(div().text_xs().text_color(theme.error).child(message));
        }
        body = body.child(row);
    }

    body = body.child(section_header(
        "Environment",
        "+ Add variable",
        submitting,
        theme,
        cx,
        |this, cx| this.add_run_env(cx),
    ));
    for index in 0..run.form.env.len() {
        let Some((key_focus, value_focus)) = focus.env.get(index).cloned() else {
            continue;
        };
        let variable = &run.form.env[index];
        let mut row = div()
            .flex()
            .items_start()
            .gap_2()
            .child(div().flex_1().min_w(px(0.)).child(input_field(
                RunField::EnvKey(index),
                &variable.key,
                "KEY",
                &key_focus,
                field_error(run, RunField::EnvKey(index)),
                read_only,
                true,
                theme,
                window,
                cx,
            )))
            .child(div().flex_1().min_w(px(0.)).child(input_field(
                RunField::EnvValue(index),
                &variable.value,
                "value",
                &value_focus,
                field_error(run, RunField::EnvValue(index)),
                read_only,
                true,
                theme,
                window,
                cx,
            )))
            .child(remove_button(
                format!("run-remove-env-{index}"),
                submitting,
                theme,
                cx,
                move |this| this.remove_run_env(index),
            ));
        if let Some(message) = field_error(run, RunField::EnvKey(index)) {
            row = row.child(div().text_xs().text_color(theme.error).child(message));
        }
        body = body.child(row);
    }

    body = body.child(section_header(
        "Volumes",
        "+ Add volume",
        submitting,
        theme,
        cx,
        |this, cx| this.add_run_mount(cx),
    ));
    for index in 0..run.form.mounts.len() {
        let Some((source_focus, target_focus)) = focus.mounts.get(index).cloned() else {
            continue;
        };
        let mount = &run.form.mounts[index];
        let mut row = div()
            .flex()
            .items_start()
            .gap_2()
            .child(div().w(px(180.)).child(input_field(
                RunField::MountSource(index),
                &mount.source,
                "volume or host path",
                &source_focus,
                field_error(run, RunField::MountSource(index)),
                read_only,
                true,
                theme,
                window,
                cx,
            )))
            .child(div().flex_1().min_w(px(0.)).child(input_field(
                RunField::MountTarget(index),
                &mount.target,
                "/path/in/container",
                &target_focus,
                field_error(run, RunField::MountTarget(index)),
                read_only,
                true,
                theme,
                window,
                cx,
            )))
            .child(toggle(
                format!("run-mount-{index}-ro"),
                "ro",
                mount.read_only,
                submitting,
                theme,
                cx,
                move |this| this.toggle_run_mount_read_only(index),
            ))
            .child(remove_button(
                format!("run-remove-mount-{index}"),
                submitting,
                theme,
                cx,
                move |this| this.remove_run_mount(index),
            ));
        if let Some(message) = field_error(run, RunField::MountSource(index))
            .or_else(|| field_error(run, RunField::MountTarget(index)))
        {
            row = row.child(div().text_xs().text_color(theme.error).child(message));
        }
        body = body.child(row);
    }

    let mut footer = div()
        .px_4()
        .py_3()
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap_3();

    if let Some(error) = &run.daemon_error {
        let mut block = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(theme.error).child(error.clone()));
        if run.missing_image {
            let can_pull = !pulling;
            let mut pull = div()
                .id("run-pull-it")
                .text_xs()
                .text_color(if can_pull { theme.accent } else { theme.text_dim })
                .child(if pulling { "Pulling…" } else { "Pull it" });
            if can_pull {
                pull = pull.cursor_pointer().on_click(cx.listener(
                    |this, _event, _window, cx| {
                        this.pull_run_image();
                        cx.notify();
                    },
                ));
            }
            block = block.child(pull);
        }
        footer = footer.child(block);
    } else {
        footer = footer.child(div().flex_1());
    }

    let mut cancel = div()
        .id("run-cancel")
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .text_sm()
        .text_color(if submitting { theme.text_dim } else { theme.text })
        .child("Cancel");
    if !submitting {
        cancel = cancel
            .cursor_pointer()
            .hover(|style| style.bg(theme.hover))
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.cancel_run();
                cx.notify();
            }));
    }

    let can_run = !run.form.image.trim().is_empty() && !submitting;
    let mut run_button = div()
        .id("run-submit")
        .px_3()
        .py_1()
        .rounded_md()
        .bg(if can_run { theme.accent } else { theme.border })
        .text_sm()
        .text_color(if can_run { theme.bg } else { theme.text_dim })
        .child(if submitting { "Running…" } else { "Run" });
    if can_run {
        run_button = run_button
            .cursor_pointer()
            .hover(|style| style.opacity(0.9))
            .on_click(cx.listener(|this, _event, window, cx| {
                this.submit_run(window, cx);
                cx.notify();
            }));
    }
    footer = footer.child(cancel).child(run_button);

    let mut close = div()
        .id("run-close")
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .child(svg().path("icons/close.svg").size(px(12.)).text_color(theme.text_dim));
    if !submitting {
        close = close
            .cursor_pointer()
            .hover(|style| style.bg(theme.hover))
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.cancel_run();
                cx.notify();
            }));
    }

    let header = div()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_base().child("Run a container"))
        .child(close);

    let panel = div()
        .w(px(640.))
        .max_h(px(560.))
        .occlude()
        .flex()
        .flex_col()
        .rounded_lg()
        .bg(theme.panel)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .child(header)
        .child(body)
        .child(footer);

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
