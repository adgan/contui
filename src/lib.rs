use gpui::prelude::*;
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div, px, size,
};

pub mod docker;
pub mod theme;

use theme::Theme;

pub fn run() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Dockyard".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| RootPlaceholder { theme: Theme::dark() }),
        )
        .unwrap();
        cx.activate(true);
    });
}

struct RootPlaceholder {
    theme: Theme,
}

impl Render for RootPlaceholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .justify_center()
            .items_center()
            .bg(self.theme.bg)
            .text_color(self.theme.text)
            .text_xl()
            .child("Dockyard")
    }
}
