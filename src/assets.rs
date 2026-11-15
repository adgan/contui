use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Assets are embedded rather than read from disk so the binary behaves the same
/// from `cargo run` and from inside the .app bundle.
pub struct Assets;

const ICONS: &[(&str, &[u8])] = &[
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("icons/stop.svg", include_bytes!("../assets/icons/stop.svg")),
    (
        "icons/restart.svg",
        include_bytes!("../assets/icons/restart.svg"),
    ),
    (
        "icons/trash.svg",
        include_bytes!("../assets/icons/trash.svg"),
    ),
    ("icons/pull.svg", include_bytes!("../assets/icons/pull.svg")),
    ("icons/run.svg", include_bytes!("../assets/icons/run.svg")),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    (
        "icons/close.svg",
        include_bytes!("../assets/icons/close.svg"),
    ),
    (
        "icons/containers.svg",
        include_bytes!("../assets/icons/containers.svg"),
    ),
    (
        "icons/images.svg",
        include_bytes!("../assets/icons/images.svg"),
    ),
    (
        "icons/volumes.svg",
        include_bytes!("../assets/icons/volumes.svg"),
    ),
    (
        "icons/search.svg",
        include_bytes!("../assets/icons/search.svg"),
    ),
    (
        "icons/refresh.svg",
        include_bytes!("../assets/icons/refresh.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/chevron-right.svg",
        include_bytes!("../assets/icons/chevron-right.svg"),
    ),
    (
        "icons/compose.svg",
        include_bytes!("../assets/icons/compose.svg"),
    ),
    ("icons/back.svg", include_bytes!("../assets/icons/back.svg")),
    (
        "icons/terminal.svg",
        include_bytes!("../assets/icons/terminal.svg"),
    ),
    (
        "icons/alert.svg",
        include_bytes!("../assets/icons/alert.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../assets/icons/check.svg"),
    ),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "fonts/Inter.ttf" => Some(include_bytes!("../assets/fonts/Inter.ttf")),
            "fonts/JetBrainsMono.ttf" => Some(include_bytes!("../assets/fonts/JetBrainsMono.ttf")),
            _ => ICONS
                .iter()
                .find(|(icon_path, _)| *icon_path == path)
                .map(|(_, bytes)| *bytes),
        };
        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> =
            vec!["fonts/Inter.ttf".into(), "fonts/JetBrainsMono.ttf".into()];
        paths.extend(ICONS.iter().map(|(path, _)| SharedString::from(*path)));
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_embedded_and_parses_as_svg() {
        for (path, bytes) in ICONS {
            let loaded = Assets.load(path).unwrap().unwrap();
            let text = std::str::from_utf8(&loaded).unwrap();
            assert_eq!(loaded.len(), bytes.len(), "{path} changed size");
            assert!(text.trim_start().starts_with("<svg"), "{path} is not svg");
            assert!(text.contains("</svg>"), "{path} is not closed");
        }
    }

    #[test]
    fn listing_covers_every_embedded_icon() {
        let listed = Assets.list("").unwrap();
        for (path, _) in ICONS {
            assert!(
                listed.iter().any(|entry| entry.as_ref() == *path),
                "{path} missing from list()"
            );
        }
    }
}
