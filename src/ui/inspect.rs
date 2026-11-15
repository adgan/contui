use gpui::SharedString;
use serde_json::Value;

const VALUE_CLIP: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub struct InspectNode {
    pub key: SharedString,
    pub value: Option<SharedString>,
    pub children: Vec<InspectNode>,
}

/// Turns an inspect payload into a two-level tree the detail page can render.
/// `depth` counts containers descended; `max_depth` stops the descent and
/// summarises what is below.
pub fn build_tree(value: &Value, max_depth: usize) -> Vec<InspectNode> {
    build_nodes(value, max_depth, 0)
}

fn build_nodes(value: &Value, max_depth: usize, depth: usize) -> Vec<InspectNode> {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, child)| build_node(key, child, max_depth, depth + 1))
            .collect(),
        _ => Vec::new(),
    }
}

fn build_node(key: &str, value: &Value, max_depth: usize, depth: usize) -> InspectNode {
    match value {
        Value::Object(map) if !map.is_empty() && depth < max_depth => InspectNode {
            key: key.to_string().into(),
            value: None,
            children: build_nodes(value, max_depth, depth),
        },
        Value::Array(items) if items.iter().any(Value::is_object) && depth < max_depth => {
            InspectNode {
                key: key.to_string().into(),
                value: None,
                children: items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| build_node(&index.to_string(), item, max_depth, depth + 1))
                    .collect(),
            }
        }
        other => InspectNode {
            key: key.to_string().into(),
            value: Some(clip(summarize(other))),
            children: Vec::new(),
        },
    }
}

fn summarize(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            if items.is_empty() {
                "[]".to_string()
            } else {
                items
                    .iter()
                    .map(|item| match item {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                "{}".to_string()
            } else {
                map.iter()
                    .map(|(k, v)| format!("{k}: {}", summarize(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
    }
}

fn clip(text: String) -> SharedString {
    if text.chars().count() <= VALUE_CLIP {
        return text.into();
    }
    let mut out: String = text.chars().take(VALUE_CLIP).collect();
    out.push('…');
    out.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_grouped_tree_from_inspect_json() {
        let json = serde_json::json!({
            "State": {"Running": true, "Paused": false},
            "Config": {"Env": ["A=1", "B=2"]}
        });
        let tree = build_tree(&json, 2);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].key.as_ref(), "State");
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[0].children[0].key.as_ref(), "Running");
        assert_eq!(tree[0].children[0].value.as_ref().unwrap().as_ref(), "true");
        assert_eq!(tree[1].key.as_ref(), "Config");
        assert_eq!(tree[1].children[0].key.as_ref(), "Env");
        assert_eq!(
            tree[1].children[0].value.as_ref().unwrap().as_ref(),
            "A=1, B=2"
        );
    }

    #[test]
    fn truncates_deep_inspect_values() {
        let long = "x".repeat(400);
        let json = serde_json::json!({ "Config": { "Env": [format!("KEY={long}")] } });
        let tree = build_tree(&json, 2);
        let value = tree[0].children[0].value.as_ref().unwrap().to_string();
        assert_eq!(value.chars().count(), 201, "200 chars plus the ellipsis");
        assert!(value.ends_with('…'));
    }

    #[test]
    fn renders_null_and_empty_objects_as_visible_scalars() {
        let json = serde_json::json!({ "A": null, "B": {}, "C": [] });
        let tree = build_tree(&json, 2);
        let by_key = |key: &str| {
            tree.iter()
                .find(|node| node.key.as_ref() == key)
                .and_then(|node| node.value.clone())
                .map(|v| v.to_string())
        };
        assert_eq!(by_key("A"), Some("null".to_string()));
        assert_eq!(by_key("B"), Some("{}".to_string()));
        assert_eq!(by_key("C"), Some("[]".to_string()));
    }
}
