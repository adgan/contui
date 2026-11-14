use gpui::prelude::*;
use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, px, size};

pub mod docker;
pub mod theme;
pub mod ui;

use ui::root::RootView;

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
            |_, cx| cx.new(RootView::new),
        )
        .unwrap();
        cx.activate(true);
    });
}
