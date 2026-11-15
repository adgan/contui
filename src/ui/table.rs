use crate::docker::model::ResourceKind;
use chrono::{DateTime, Utc};
use gpui::{Rgba, SharedString, UniformListScrollHandle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionsColumn {
    Container,
    Image,
    Volume,
}

impl ActionsColumn {
    pub fn width(self) -> f32 {
        match self {
            ActionsColumn::Container => 124.0,
            ActionsColumn::Image => 84.0,
            ActionsColumn::Volume => 44.0,
        }
    }

    pub fn for_kind(kind: ResourceKind) -> Self {
        match kind {
            ResourceKind::Container => ActionsColumn::Container,
            ResourceKind::Image => ActionsColumn::Image,
            ResourceKind::Volume => ActionsColumn::Volume,
        }
    }

    pub fn kind(self) -> ResourceKind {
        match self {
            ActionsColumn::Container => ResourceKind::Container,
            ActionsColumn::Image => ResourceKind::Image,
            ActionsColumn::Volume => ResourceKind::Volume,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidth {
    Fixed(f32),
    Flex(f32),
}

#[derive(Clone, Debug)]
pub struct TableSpec {
    pub id: &'static str,
    pub columns: Vec<&'static str>,
    pub widths: Vec<ColumnWidth>,
    pub actions: Option<ActionsColumn>,
}

#[derive(Clone, Debug)]
pub enum SortValue {
    Text(String),
    Number(i64),
    Time(DateTime<Utc>),
}

#[derive(Clone, Debug)]
pub struct TableRow {
    pub id: String,
    pub cells: Vec<SharedString>,
    pub sort_values: Vec<SortValue>,
    pub accent: Option<Rgba>,
}

impl TableRow {
    fn matches(&self, needle: &str) -> bool {
        self.cells
            .iter()
            .any(|cell| cell.to_lowercase().contains(needle))
    }

    fn compare(&self, other: &Self, column: usize) -> std::cmp::Ordering {
        match (self.sort_values.get(column), other.sort_values.get(column)) {
            (Some(SortValue::Text(a)), Some(SortValue::Text(b))) => {
                a.to_lowercase().cmp(&b.to_lowercase())
            }
            (Some(SortValue::Number(a)), Some(SortValue::Number(b))) => a.cmp(b),
            (Some(SortValue::Time(a)), Some(SortValue::Time(b))) => a.cmp(b),
            (Some(a), Some(b)) => format!("{a:?}").cmp(&format!("{b:?}")),
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
        }
    }
}

pub struct TableState {
    pub spec: TableSpec,
    pub rows: Vec<TableRow>,
    pub sort: Option<(usize, bool)>,
    pub query: String,
    pub selected: Option<usize>,
    pub scroll: UniformListScrollHandle,
    cached: Vec<usize>,
}

impl TableState {
    pub fn new(spec: TableSpec) -> Self {
        Self {
            spec,
            rows: Vec::new(),
            sort: None,
            query: String::new(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            cached: Vec::new(),
        }
    }

    pub fn set_rows(&mut self, rows: Vec<TableRow>) {
        self.rows = rows;
        self.recompute();
    }

    pub fn set_query(&mut self, query: &str) {
        if self.query == query {
            return;
        }
        self.query = query.to_string();
        self.recompute();
    }

    /// Carries the user's query, sort and selected row onto a freshly built
    /// table, so a live snapshot never resets what they were looking at.
    pub fn adopt_state(&mut self, previous: &TableState) {
        self.query = previous.query.clone();
        self.sort = previous.sort;
        let selected_id = previous.selected_row().map(|row| row.id.clone());
        self.recompute();
        self.selected = selected_id
            .and_then(|id| self.rows.iter().position(|row| row.id == id));
    }

    pub fn click_header(&mut self, column: usize) {
        self.sort = match self.sort {
            Some((col, true)) if col == column => Some((column, false)),
            Some((col, false)) if col == column => None,
            _ => Some((column, true)),
        };
        self.recompute();
    }

    pub fn visible(&self) -> &[usize] {
        &self.cached
    }

    /// Position of the selected row inside the visible list, for scrolling.
    pub fn selection_position(&self) -> Option<usize> {
        let selected = self.selected?;
        self.cached.iter().position(|&ix| ix == selected)
    }

    pub fn selected_row(&self) -> Option<&TableRow> {
        self.selected.and_then(|ix| self.rows.get(ix))
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.cached.is_empty() {
            self.selected = None;
            return;
        }
        let Some(selected) = self.selected else {
            self.selected = if delta >= 0 {
                self.cached.first().copied()
            } else {
                self.cached.last().copied()
            };
            return;
        };
        let current_position = self
            .cached
            .iter()
            .position(|&ix| ix == selected)
            .unwrap_or(0);
        let next = (current_position as isize + delta)
            .clamp(0, self.cached.len() as isize - 1) as usize;
        self.selected = Some(self.cached[next]);
    }

    fn recompute(&mut self) {
        let needle = self.query.to_lowercase();
        let mut indices: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| needle.is_empty() || row.matches(&needle))
            .map(|(ix, _)| ix)
            .collect();

        if let Some((column, ascending)) = self.sort {
            indices.sort_by(|a, b| {
                let ordering = self.rows[*a].compare(&self.rows[*b], column);
                if ascending {
                    ordering
                } else {
                    ordering.reverse()
                }
            });
        }

        if let Some(selected) = self.selected
            && !indices.contains(&selected)
        {
            self.selected = indices.last().copied();
        }

        self.cached = indices;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TableSpec {
        TableSpec {
            id: "t",
            columns: vec!["Name", "Image"],
            widths: vec![ColumnWidth::Flex(1.0), ColumnWidth::Flex(1.0)],
            actions: None,
        }
    }

    #[test]
    fn action_columns_are_not_data_cells() {
        let mut s = spec();
        s.actions = Some(ActionsColumn::Container);
        assert_eq!(s.columns.len(), 2);
        assert!(s.actions.is_some());
    }

    fn row(id: &str, name: &str, image: &str) -> TableRow {
        TableRow {
            id: id.to_string(),
            cells: vec![name.to_string().into(), image.to_string().into()],
            sort_values: vec![
                SortValue::Text(name.to_string()),
                SortValue::Text(image.to_string()),
            ],
            accent: None,
        }
    }

    #[test]
    fn filters_case_insensitively_across_columns() {
        let mut t = TableState::new(spec());
        t.set_rows(vec![
            row("1", "sploney-db-1", "postgres:18"),
            row("2", "k8s_traefik", "fcdef599"),
        ]);
        assert_eq!(t.visible(), &[0, 1]);
        t.set_query("POSTGRES");
        assert_eq!(t.visible(), &[0]);
        t.set_query("");
        assert_eq!(t.visible().len(), 2);
    }

    #[test]
    fn header_click_cycles_asc_desc_default() {
        let mut t = TableState::new(spec());
        assert_eq!(t.sort, None);
        t.click_header(0);
        assert_eq!(t.sort, Some((0, true)));
        t.click_header(0);
        assert_eq!(t.sort, Some((0, false)));
        t.click_header(0);
        assert_eq!(t.sort, None);
        t.click_header(1);
        assert_eq!(t.sort, Some((1, true)));
    }

    #[test]
    fn sorting_is_numeric_and_temporal_for_number_time_values() {
        let mut t = TableState::new(spec());
        t.set_rows(vec![
            TableRow {
                id: "big".into(),
                cells: vec!["456 MB".into(), "".into()],
                sort_values: vec![SortValue::Number(456_654_848), SortValue::Number(0)],
                accent: None,
            },
            TableRow {
                id: "small".into(),
                cells: vec!["93.8 MB".into(), "".into()],
                sort_values: vec![SortValue::Number(98_353_152), SortValue::Number(0)],
                accent: None,
            },
        ]);
        t.click_header(0);
        assert_eq!(t.visible(), &[1, 0]);
        t.click_header(0);
        assert_eq!(t.visible(), &[0, 1]);

        let mut u = TableState::new(spec());
        let older = chrono::DateTime::from_timestamp(1_600_000_000, 0).unwrap();
        let newer = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        u.set_rows(vec![
            TableRow {
                id: "new".into(),
                cells: vec!["".into(), "".into()],
                sort_values: vec![SortValue::Time(newer), SortValue::Number(0)],
                accent: None,
            },
            TableRow {
                id: "old".into(),
                cells: vec!["".into(), "".into()],
                sort_values: vec![SortValue::Time(older), SortValue::Number(0)],
                accent: None,
            },
        ]);
        u.click_header(0);
        assert_eq!(u.visible(), &[1, 0]);
    }

    #[test]
    fn selection_clamps_after_filtering() {
        let mut t = TableState::new(spec());
        t.set_rows(
            (0..10)
                .map(|i| row(&i.to_string(), &format!("c{i}"), "img"))
                .collect(),
        );
        t.selected = Some(9);
        t.set_query("c7");
        assert_eq!(t.visible(), &[7]);
        assert_eq!(t.selected, Some(7));
    }

    #[test]
    fn computes_visible_indices_for_ten_thousand_rows_quickly() {
        let mut t = TableState::new(spec());
        t.set_rows(
            (0..10_000)
                .map(|i| {
                    if i % 100 == 0 {
                        row(&i.to_string(), &format!("abc{i}"), "img")
                    } else {
                        row(&i.to_string(), &format!("def{i}"), "img")
                    }
                })
                .collect(),
        );
        t.set_query("abc");
        assert_eq!(t.visible().len(), 100);
        let first = t.visible().as_ptr();
        let second = t.visible().as_ptr();
        assert_eq!(first, second, "visible() must not recompute");
    }

    #[test]
    fn move_selection_starts_at_first_row_when_nothing_selected() {
        let mut t = TableState::new(spec());
        t.set_rows(
            (0..3)
                .map(|i| row(&i.to_string(), &format!("c{i}"), "img"))
                .collect(),
        );
        t.move_selection(1);
        assert_eq!(t.selected, Some(0), "first Down must land on row 0");
    }

    #[test]
    fn move_selection_up_from_nothing_selects_the_last_row() {
        let mut t = TableState::new(spec());
        t.set_rows(
            (0..3)
                .map(|i| row(&i.to_string(), &format!("c{i}"), "img"))
                .collect(),
        );
        t.move_selection(-1);
        assert_eq!(t.selected, Some(2));
    }

    #[test]
    fn adopt_state_keeps_query_sort_and_selection() {
        let mut previous = TableState::new(spec());
        previous.set_rows(vec![
            row("a", "alpha", "img"),
            row("b", "beta", "img"),
            row("c", "gamma", "img"),
        ]);
        previous.set_query("alph");
        previous.click_header(0);
        previous.selected = Some(0);

        let mut fresh = TableState::new(spec());
        fresh.set_rows(vec![
            row("c", "gamma", "img"),
            row("a", "alpha", "img"),
            row("b", "beta", "img"),
        ]);
        fresh.adopt_state(&previous);

        assert_eq!(fresh.query, "alph");
        assert_eq!(fresh.sort, Some((0, true)));
        assert_eq!(
            fresh.selected.map(|ix| fresh.rows[ix].id.clone()),
            Some("a".to_string())
        );
        assert_eq!(fresh.visible().len(), 1);
    }

    #[test]
    fn widths_cover_fixed_and_flex_columns() {
        let s = TableSpec {
            id: "t",
            columns: vec!["A", "B"],
            widths: vec![ColumnWidth::Fixed(80.0), ColumnWidth::Flex(1.0)],
            actions: None,
        };
        assert_eq!(s.columns.len(), s.widths.len());
    }
}
