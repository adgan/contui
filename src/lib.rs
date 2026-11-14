use gpui::prelude::*;
use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, px, size,
};

use crate::actions::{
    Back, FocusSearch, Quit, Refresh, ShowContainers, ShowImages, ShowVolumes,
};

pub mod actions;
pub mod assets;
pub mod docker;
pub mod theme;
pub mod ui;

use ui::root::RootView;

pub fn run() {
    Application::new()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            {
                use gpui::AssetSource as _;
                let fonts: Vec<std::borrow::Cow<'static, [u8]>> = [
                    "fonts/Inter.ttf",
                    "fonts/JetBrainsMono.ttf",
                ]
                .into_iter()
                .filter_map(|path| assets::Assets.load(path).ok().flatten())
                .collect();
                if !fonts.is_empty() {
                    let _ = cx.text_system().add_fonts(fonts);
                }
            }
        cx.bind_keys([
            KeyBinding::new("cmd-1", ShowContainers, None),
            KeyBinding::new("cmd-2", ShowImages, None),
            KeyBinding::new("cmd-3", ShowVolumes, None),
            KeyBinding::new("cmd-r", Refresh, None),
            KeyBinding::new("cmd-f", FocusSearch, None),
            KeyBinding::new("escape", Back, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.set_menus(vec![
            Menu {
                name: "contUI".into(),
                items: vec![
                    MenuItem::action("Quit contUI", Quit),
                ],
            },
            Menu {
                name: "View".into(),
            items: vec![
                MenuItem::action("Containers", ShowContainers),
                MenuItem::action("Images", ShowImages),
                MenuItem::action("Volumes", ShowVolumes),
                MenuItem::separator(),
                MenuItem::action("Refresh", Refresh),
                MenuItem::action("Search", FocusSearch),
            ],
        },
        ]);

        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(960.), px(600.))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("contUI".into()),
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
