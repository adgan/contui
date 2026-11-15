use gpui::{Rgba, rgb};

/// Single source of truth for every color and for the spacing, radius and type
/// scales the layout uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub bg: Rgba,
    pub panel: Rgba,
    pub panel_raised: Rgba,
    pub rail: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub border: Rgba,
    pub border_subtle: Rgba,
    pub text: Rgba,
    pub text_dim: Rgba,
    pub text_faint: Rgba,
    pub accent: Rgba,
    pub accent_soft: Rgba,
    pub running: Rgba,
    pub running_soft: Rgba,
    pub exited: Rgba,
    pub exited_soft: Rgba,
    pub paused: Rgba,
    pub paused_soft: Rgba,
    pub error: Rgba,
    pub error_soft: Rgba,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg: rgb(0x0b0d10),
            panel: rgb(0x12151a),
            panel_raised: rgb(0x171c24),
            rail: rgb(0x0e1116),
            hover: rgb(0x1a1f27),
            selected: rgb(0x1f2937),
            border: rgb(0x232a34),
            border_subtle: rgb(0x1a2028),
            text: rgb(0xe6e9ef),
            text_dim: rgb(0x8b93a1),
            text_faint: rgb(0x5b6472),
            accent: rgb(0x2f7bf6),
            accent_soft: rgb(0x16243c),
            running: rgb(0x3fb950),
            running_soft: rgb(0x12281a),
            exited: rgb(0x6e7681),
            exited_soft: rgb(0x1d222a),
            paused: rgb(0xd29922),
            paused_soft: rgb(0x2b2412),
            error: rgb(0xf85149),
            error_soft: rgb(0x2d1718),
        }
    }
}

/// Spacing scale in pixels: 4, 8, 12, 16, 20, 24, 32.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 20.0;
    pub const XXL: f32 = 24.0;
    pub const XXXL: f32 = 32.0;
}

/// Corner radii in pixels.
pub mod radius {
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 10.0;
    pub const LG: f32 = 14.0;
    pub const PILL: f32 = 999.0;
}

/// Type sizes in pixels. Weights and colors are chosen per role in the UI.
pub mod type_scale {
    pub const KICKER: f32 = 10.0;
    pub const META: f32 = 11.0;
    pub const BODY: f32 = 13.0;
    pub const TITLE: f32 = 18.0;
    pub const HERO: f32 = 22.0;
    pub const DISPLAY: f32 = 28.0;
    pub const MONO: f32 = 12.0;
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

    #[test]
    fn dashboard_tokens_match_the_design() {
        let t = Theme::dark();
        assert_eq!(t.rail, rgb(0x0e1116));
        assert_eq!(t.panel_raised, rgb(0x171c24));
        assert_eq!(t.border_subtle, rgb(0x1a2028));
        assert_eq!(t.text_faint, rgb(0x5b6472));
        assert_eq!(t.accent_soft, rgb(0x16243c));
        assert_eq!(t.running_soft, rgb(0x12281a));
        assert_eq!(t.paused_soft, rgb(0x2b2412));
        assert_eq!(t.exited_soft, rgb(0x1d222a));
        assert_eq!(t.error_soft, rgb(0x2d1718));
    }

    #[test]
    fn scale_values_are_the_agreed_steps() {
        assert_eq!(space::XS, 4.0);
        assert_eq!(space::XXXL, 32.0);
        assert_eq!(radius::SM, 6.0);
        assert_eq!(radius::LG, 14.0);
        assert_eq!(type_scale::DISPLAY, 28.0);
        assert_eq!(type_scale::BODY, 13.0);
    }
}
