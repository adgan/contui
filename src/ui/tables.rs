use crate::docker::model::{ContainerState, Snapshot};
use crate::ui::table::{
    ActionsColumn, CellStyle, ColumnWidth, SortValue, TableRow, TableSpec, TableState,
};
use crate::ui::widgets::pill::state_label;

/// Builds the three list tables from a snapshot, carrying the user's query,
/// sort and selection from `previous` onto the fresh rows.
pub fn tables_from(snapshot: &Snapshot, previous: &[TableState; 3]) -> [TableState; 3] {
    let mut next = [
        containers_table(snapshot),
        images_table(snapshot),
        volumes_table(snapshot),
    ];
    for (fresh, old) in next.iter_mut().zip(previous.iter()) {
        fresh.adopt_state(old);
    }
    next
}

pub fn containers_table(snapshot: &Snapshot) -> TableState {
    let spec = TableSpec {
        id: "containers-table",
        columns: vec!["Name", "State", "Image", "Ports", "Created"],
        widths: vec![
            ColumnWidth::Flex(2.2),
            ColumnWidth::Fixed(104.0),
            ColumnWidth::Flex(2.0),
            ColumnWidth::Fixed(132.0),
            ColumnWidth::Fixed(100.0),
        ],
        styles: vec![
            CellStyle::Text,
            CellStyle::Pill,
            CellStyle::Mono,
            CellStyle::Mono,
            CellStyle::Timestamp,
        ],
        actions: Some(ActionsColumn::Container),
    };

    let mut sorted: Vec<&crate::docker::model::ContainerRow> = snapshot.containers.iter().collect();
    sorted.sort_by(|a, b| {
        let a_running = a.state == ContainerState::Running;
        let b_running = b.state == ContainerState::Running;
        b_running
            .cmp(&a_running)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let rows = sorted
        .into_iter()
        .map(|c| TableRow {
            id: c.id.clone(),
            cells: vec![
                c.name.clone(),
                state_label(c.state).into(),
                c.image.clone(),
                c.ports.clone(),
                c.created_label.clone(),
            ],
            sort_values: vec![
                SortValue::Text(c.name.to_string()),
                SortValue::Text(state_label(c.state).to_string()),
                SortValue::Text(c.image.to_string()),
                SortValue::Text(c.ports.to_string()),
                SortValue::Time(c.created),
            ],
        })
        .collect();

    let mut table = TableState::new(spec);
    table.set_rows(rows);
    table
}

pub fn images_table(snapshot: &Snapshot) -> TableState {
    let spec = TableSpec {
        id: "images-table",
        columns: vec!["Repository", "ID", "Size", "Created", "In Use"],
        widths: vec![
            ColumnWidth::Flex(3.0),
            ColumnWidth::Fixed(120.0),
            ColumnWidth::Fixed(90.0),
            ColumnWidth::Fixed(100.0),
            ColumnWidth::Fixed(80.0),
        ],
        styles: vec![
            CellStyle::Text,
            CellStyle::Mono,
            CellStyle::Mono,
            CellStyle::Timestamp,
            CellStyle::Text,
        ],
        actions: Some(ActionsColumn::Image),
    };

    let mut sorted: Vec<&crate::docker::model::ImageRow> = snapshot.images.iter().collect();
    sorted.sort_by(|a, b| b.created.cmp(&a.created));

    let rows = sorted
        .into_iter()
        .map(|image| TableRow {
            id: image.id.clone(),
            cells: vec![
                image.repository_tag.clone(),
                image.short_id.clone(),
                image.size_label.clone(),
                image.created_label.clone(),
                if image.in_use { "yes" } else { "—" }.into(),
            ],
            sort_values: vec![
                SortValue::Text(image.repository_tag.to_string()),
                SortValue::Text(image.short_id.to_string()),
                SortValue::Number(image.size),
                SortValue::Time(image.created),
                SortValue::Text(if image.in_use { "yes" } else { "no" }.to_string()),
            ],
        })
        .collect();

    let mut table = TableState::new(spec);
    table.set_rows(rows);
    table
}

pub fn volumes_table(snapshot: &Snapshot) -> TableState {
    let spec = TableSpec {
        id: "volumes-table",
        columns: vec!["Name", "Driver", "Mountpoint", "Created"],
        widths: vec![
            ColumnWidth::Flex(2.0),
            ColumnWidth::Fixed(100.0),
            ColumnWidth::Flex(3.0),
            ColumnWidth::Fixed(100.0),
        ],
        styles: vec![
            CellStyle::Text,
            CellStyle::Text,
            CellStyle::Mono,
            CellStyle::Timestamp,
        ],
        actions: Some(ActionsColumn::Volume),
    };

    let mut sorted: Vec<&crate::docker::model::VolumeRow> = snapshot.volumes.iter().collect();
    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let rows = sorted
        .into_iter()
        .map(|volume| TableRow {
            id: volume.name.to_string(),
            cells: vec![
                volume.name.clone(),
                volume.driver.clone(),
                volume.mountpoint.clone(),
                volume.created_label.clone(),
            ],
            sort_values: vec![
                SortValue::Text(volume.name.to_string()),
                SortValue::Text(volume.driver.to_string()),
                SortValue::Text(volume.mountpoint.to_string()),
                SortValue::Text(volume.created_label.to_string()),
            ],
        })
        .collect();

    let mut table = TableState::new(spec);
    table.set_rows(rows);
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::{ContainerRow, ImageRow, Snapshot, VolumeRow};
    use chrono::{DateTime, Utc};
    use gpui::SharedString;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn container(name: &str, state: ContainerState) -> ContainerRow {
        ContainerRow {
            id: format!("{name}-id"),
            name: SharedString::from(name.to_string()),
            state,
            status: SharedString::from("Up 2 minutes"),
            image: SharedString::from("postgres:18"),
            ports: SharedString::from("5432→5432"),
            created: now(),
            created_label: SharedString::from("2m ago"),
            compose_project: None,
        }
    }

    fn image(tag: &str, in_use: bool) -> ImageRow {
        ImageRow {
            id: format!("{tag}-id"),
            short_id: SharedString::from("abcdef123456"),
            repository_tag: SharedString::from(tag.to_string()),
            size: 456_654_848,
            size_label: SharedString::from("436 MB"),
            created: now(),
            created_label: SharedString::from("2m ago"),
            in_use,
            tag_count: 1,
        }
    }

    fn volume(name: &str) -> VolumeRow {
        VolumeRow {
            name: SharedString::from(name.to_string()),
            driver: SharedString::from("local"),
            mountpoint: SharedString::from("/var/lib/docker/volumes/x/_data"),
            created_label: SharedString::from("2m ago"),
        }
    }

    #[test]
    fn each_table_declares_its_action_column() {
        let snapshot = Snapshot::empty();
        assert_eq!(
            containers_table(&snapshot).spec.actions,
            Some(ActionsColumn::Container)
        );
        assert_eq!(
            images_table(&snapshot).spec.actions,
            Some(ActionsColumn::Image)
        );
        assert_eq!(
            volumes_table(&snapshot).spec.actions,
            Some(ActionsColumn::Volume)
        );
        assert_eq!(ActionsColumn::Container.width(), 132.0);
        assert_eq!(ActionsColumn::Image.width(), 92.0);
        assert_eq!(ActionsColumn::Volume.width(), 52.0);
    }

    #[test]
    fn containers_table_puts_running_first_then_name() {
        let snapshot = Snapshot {
            containers: vec![
                container("aaa", ContainerState::Exited),
                container("zzz", ContainerState::Running),
                container("bbb", ContainerState::Running),
            ],
            ..Snapshot::empty()
        };
        let table = containers_table(&snapshot);
        let names: Vec<&str> = table
            .visible()
            .iter()
            .map(|&ix| table.rows[ix].cells[0].as_ref())
            .collect();
        assert_eq!(names, vec!["bbb", "zzz", "aaa"]);
    }

    #[test]
    fn images_table_marks_in_use_from_container_list() {
        let snapshot = Snapshot {
            images: vec![image("postgres:18", true), image("alpine:3", false)],
            ..Snapshot::empty()
        };
        let table = images_table(&snapshot);
        let used = table
            .rows
            .iter()
            .find(|r| r.cells[0].as_ref() == "postgres:18")
            .unwrap();
        let unused = table
            .rows
            .iter()
            .find(|r| r.cells[0].as_ref() == "alpine:3")
            .unwrap();
        assert_eq!(used.cells[4].as_ref(), "yes");
        assert_eq!(unused.cells[4].as_ref(), "—");
    }

    #[test]
    fn volumes_table_has_expected_columns() {
        let table = volumes_table(&Snapshot {
            volumes: vec![volume("sploney_db-data")],
            ..Snapshot::empty()
        });
        assert_eq!(
            table.spec.columns,
            vec!["Name", "Driver", "Mountpoint", "Created"]
        );
        assert_eq!(table.rows.len(), 1);
    }

    #[test]
    fn rebuilding_tables_keeps_query_sort_and_selection() {
        let snapshot = Snapshot {
            containers: vec![
                container("alpha", ContainerState::Running),
                container("beta", ContainerState::Exited),
            ],
            ..Snapshot::empty()
        };
        let mut previous = [
            containers_table(&snapshot),
            images_table(&snapshot),
            volumes_table(&snapshot),
        ];
        previous[0].set_query("alpha");
        previous[0].click_header(0);
        previous[0].selected = Some(0);

        let next = tables_from(&snapshot, &previous);
        assert_eq!(next[0].query, "alpha");
        assert_eq!(next[0].sort, Some((0, true)));
        assert!(next[0].visible().len() <= 1);
    }

    #[test]
    fn empty_snapshot_produces_empty_tables_with_headers() {
        let table = containers_table(&Snapshot::empty());
        assert!(table.visible().is_empty());
        assert_eq!(
            table.spec.columns,
            vec!["Name", "State", "Image", "Ports", "Created"]
        );
    }
}
