use crate::docker::endpoint::HostStatus;
use crate::ui::root::Page;

/// What the empty area under the table says. `None` means there is nothing to
/// explain (a connected host with rows and no query).
pub fn message_for(
    page: Page,
    status: &HostStatus,
    query: &str,
) -> Option<(String, String)> {
    if !query.is_empty() {
        return Some((
            format!("No matches for \"{query}\""),
            "Try a different name, image or id.".to_string(),
        ));
    }

    match status {
        HostStatus::Connected | HostStatus::Unknown => Some((
            page.empty_title().to_string(),
            "Containers, images and volumes show up here as soon as the engine has them."
                .to_string(),
        )),
        HostStatus::Unreachable(reason) => Some((
            "Can't reach the Docker engine".to_string(),
            format!("{reason}. Is the engine running? Retry with ⌘R."),
        )),
        HostStatus::AuthFailed(reason) => Some((
            "Permission denied by the Docker engine".to_string(),
            format!("{reason}. Check that your user may access the socket."),
        )),
        HostStatus::Unsupported(reason) => Some((
            "This host is not supported yet".to_string(),
            reason.clone(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreachable_host_names_the_socket() {
        let (title, body) = message_for(
            Page::Containers,
            &HostStatus::Unreachable("no such file: /Users/me/.rd/docker.sock".into()),
            "",
        )
        .unwrap();
        assert!(title.contains("Can't reach"));
        assert!(body.contains("Is the engine running?"));
    }

    #[test]
    fn auth_failed_is_distinct_from_unreachable() {
        let (title, _) = message_for(
            Page::Containers,
            &HostStatus::AuthFailed("permission denied".into()),
            "",
        )
        .unwrap();
        assert!(title.to_lowercase().contains("permission"));
    }

    #[test]
    fn empty_query_with_no_rows_is_not_the_same_as_no_search_hits() {
        let (title, _) = message_for(Page::Containers, &HostStatus::Connected, "").unwrap();
        assert_eq!(title, "No containers yet");

        let (title, _) = message_for(Page::Containers, &HostStatus::Connected, "zzz").unwrap();
        assert_eq!(title, "No matches for \"zzz\"");
    }

    #[test]
    fn unsupported_host_explains_itself() {
        let (title, body) = message_for(
            Page::Containers,
            &HostStatus::Unsupported("SSH hosts are listed but not connected".into()),
            "",
        )
        .unwrap();
        assert!(title.contains("not supported") || body.contains("not connected"));
    }
}
