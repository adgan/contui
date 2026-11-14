use gpui::{Rgba, rgb};

/// Single source of truth for every color in the app.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub bg: Rgba,
    pub panel: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub text_dim: Rgba,
    pub accent: Rgba,
    pub running: Rgba,
    pub exited: Rgba,
    pub paused: Rgba,
    pub error: Rgba,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg: rgb(0x0b0d10),
            panel: rgb(0x12151a),
            hover: rgb(0x1a1f27),
            selected: rgb(0x1f2937),
            border: rgb(0x232a34),
            text: rgb(0xe6e9ef),
            text_dim: rgb(0x8b93a1),
            accent: rgb(0x2f7bf6),
            running: rgb(0x3fb950),
            exited: rgb(0x6e7681),
            paused: rgb(0xd29922),
            error: rgb(0xf85149),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;

    #[test]
    fn dark_theme_uses_spec_tokens() {
        let t = Theme::dark();
        assert_eq!(t.bg, rgb(0x0b0d10));
        assert_eq!(t.panel, rgb(0x12151a));
        assert_eq!(t.accent, rgb(0x2f7bf6));
        assert_eq!(t.running, rgb(0x3fb950));
        assert_eq!(t.error, rgb(0xf85149));
    }
}
