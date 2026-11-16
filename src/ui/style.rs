use crate::theme::{Theme, radius, space};
use gpui::prelude::*;
use gpui::{Div, Pixels, Size, div, px};

/// Sizes shared by the shell, the overview strip and the tables. Computed once
/// per frame from the viewport; nothing here changes behavior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub compact: bool,
    pub rail_width: f32,
    pub top_bar_height: f32,
    pub overview_height: f32,
    pub footer_height: f32,
    pub page_padding: f32,
    pub section_gap: f32,
    pub table_header_height: f32,
    pub row_height: f32,
    pub cell_padding: f32,
}

impl Layout {
    pub fn for_viewport(viewport: Size<Pixels>) -> Self {
        let compact = viewport.width < px(1080.) || viewport.height < px(700.);
        Self {
            compact,
            rail_width: if compact { 64.0 } else { 76.0 },
            top_bar_height: if compact { 56.0 } else { 64.0 },
            overview_height: if compact { 76.0 } else { 88.0 },
            footer_height: if compact { 28.0 } else { 32.0 },
            page_padding: if compact { space::LG } else { space::XXL },
            section_gap: if compact { space::MD } else { space::XL },
            table_header_height: 40.0,
            row_height: 52.0,
            cell_padding: 20.0,
        }
    }
}

/// The surface every list, detail body, dialog and menu shares.
pub fn card(theme: &Theme) -> Div {
    div()
        .bg(theme.panel)
        .border_1()
        .border_color(theme.border)
        .rounded(px(radius::LG))
        .overflow_hidden()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, size};

    #[test]
    fn layout_is_compact_below_the_thresholds() {
        let small = Layout::for_viewport(size(px(960.), px(600.)));
        assert!(small.compact);
        assert_eq!(small.page_padding, space::LG);
        assert_eq!(small.section_gap, space::MD);

        let narrow = Layout::for_viewport(size(px(1000.), px(820.)));
        assert!(narrow.compact);

        let default = Layout::for_viewport(size(px(1280.), px(820.)));
        assert!(!default.compact);
        assert_eq!(default.page_padding, space::XXL);
        assert_eq!(default.section_gap, space::XL);
    }

    #[test]
    fn layout_heights_match_the_design() {
        let full = Layout::for_viewport(size(px(1280.), px(820.)));
        assert_eq!(full.top_bar_height, 64.0);
        assert_eq!(full.overview_height, 88.0);
        assert_eq!(full.footer_height, 32.0);
        assert_eq!(full.row_height, 52.0);
        assert_eq!(full.table_header_height, 40.0);

        let compact = Layout::for_viewport(size(px(960.), px(600.)));
        assert_eq!(compact.top_bar_height, 56.0);
        assert_eq!(compact.overview_height, 76.0);
        assert_eq!(compact.footer_height, 28.0);
        assert_eq!(compact.row_height, 52.0);
    }
}
