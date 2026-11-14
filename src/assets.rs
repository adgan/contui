use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Assets are embedded rather than read from disk so the binary behaves the same
/// from `cargo run` and from inside the .app bundle.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "fonts/Inter.ttf" => Some(include_bytes!("../assets/fonts/Inter.ttf")),
            "fonts/JetBrainsMono.ttf" => {
                Some(include_bytes!("../assets/fonts/JetBrainsMono.ttf"))
            }
            "icons/play.svg" => Some(include_bytes!("../assets/icons/play.svg")),
            "icons/stop.svg" => Some(include_bytes!("../assets/icons/stop.svg")),
            "icons/restart.svg" => Some(include_bytes!("../assets/icons/restart.svg")),
            "icons/trash.svg" => Some(include_bytes!("../assets/icons/trash.svg")),
            "icons/pull.svg" => Some(include_bytes!("../assets/icons/pull.svg")),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![
            "fonts/Inter.ttf".into(),
            "fonts/JetBrainsMono.ttf".into(),
            "icons/play.svg".into(),
            "icons/stop.svg".into(),
            "icons/restart.svg".into(),
            "icons/trash.svg".into(),
            "icons/pull.svg".into(),
        ])
    }
}
