//! Compatibility readers for the application's saved state (including what
//! the former miaotty builds wrote).

use serde_json::{json, Value};
use std::path::Path;

/// Prefer canonical state, then the former native host, then the former
/// eframe host. Legacy files remain intact; subsequent saves use canonical paths.
pub fn load(config: &Path, data: Option<&Path>) -> Option<Value> {
    let canonical = config.join("session.json");
    if canonical.exists() {
        return read(&canonical);
    }
    let native = config.join("native-session.json");
    if native.exists() {
        return read(&native);
    }
    read(&data?.join("session.json"))
}

fn read(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Convert indexed eframe pane/layout references to native pane IDs. Native
/// sessions and recipes pass through unchanged.
pub fn normalize(mut value: Value) -> Option<Value> {
    let tabs = value.get_mut("tabs")?.as_array_mut()?;
    for tab in tabs {
        let indexed = tab.get("active").is_some_and(Value::is_u64);
        if !indexed {
            continue;
        }
        let panes = tab.get_mut("panes")?.as_array_mut()?;
        let ids: Vec<String> = (0..panes.len())
            .map(|i| format!("legacy-pane-{i}"))
            .collect();
        for (pane, id) in panes.iter_mut().zip(&ids) {
            pane["id"] = json!(id);
        }
        let active = tab.get("active")?.as_u64()? as usize;
        tab["active"] = json!(ids.get(active).or_else(|| ids.first())?);
        tab["layout"] = convert_layout(tab.get("layout")?, &ids)?;
    }
    if value.get("active_tab").is_none() {
        value["active_tab"] = value.get("active").cloned().unwrap_or(json!(0));
    }
    if value.get("recent").is_none() {
        value["recent"] = value.get("recent_files").cloned().unwrap_or(json!([]));
    }
    Some(value)
}

fn convert_layout(value: &Value, ids: &[String]) -> Option<Value> {
    if let Some(index) = value.get("Leaf").and_then(Value::as_u64) {
        return Some(json!({"leaf": ids.get(index as usize)?}));
    }
    let split = value.get("Split")?;
    Some(json!({
        "dir": if split.get("down")?.as_bool()? { "down" } else { "right" },
        "ratio": split.get("ratio")?,
        "a": convert_layout(split.get("a")?, ids)?,
        "b": convert_layout(split.get("b")?, ids)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_indexed_splits_focus_and_tab_decorations() {
        let old = json!({"active": 1, "recent_files": ["note.txt"], "tabs": [{
            "title": "work", "prefix": "dev", "mark": "★", "group": "project",
            "panes": [{"cwd": "/tmp"}, {"cwd": "/"}, {"cwd": null}], "active": 2,
            "layout": {"Split": {"down": false, "ratio": 0.3, "a": {"Leaf": 0},
                "b": {"Split": {"down": true, "ratio": 0.7, "a": {"Leaf": 1}, "b": {"Leaf": 2}}}}}
        }]});
        let converted = normalize(old).unwrap();
        let tab = &converted["tabs"][0];
        assert_eq!(converted["active_tab"], 1);
        assert_eq!(converted["recent"], json!(["note.txt"]));
        assert_eq!(tab["active"], "legacy-pane-2");
        assert_eq!(tab["layout"]["a"]["leaf"], "legacy-pane-0");
        assert_eq!(tab["layout"]["b"]["dir"], "down");
        assert_eq!(tab["layout"]["b"]["b"]["leaf"], "legacy-pane-2");
        assert_eq!(tab["panes"][1]["cwd"], "/");
        assert_eq!(tab["group"], "project");
        assert_eq!(tab["mark"], "★");
    }

    #[test]
    fn canonical_wins_and_legacy_state_is_never_deleted() {
        let root = std::env::temp_dir().join(format!("mtty-migration-{}", std::process::id()));
        let config = root.join("config");
        let data = root.join("data");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        for (path, source) in [
            (data.join("session.json"), "eframe"),
            (config.join("native-session.json"), "native"),
        ] {
            std::fs::write(path, json!({"source": source}).to_string()).unwrap();
        }
        assert_eq!(load(&config, Some(&data)).unwrap()["source"], "native");
        std::fs::write(
            config.join("session.json"),
            json!({"source": "canonical"}).to_string(),
        )
        .unwrap();
        assert_eq!(load(&config, Some(&data)).unwrap()["source"], "canonical");
        assert!(config.join("native-session.json").exists());
        assert!(data.join("session.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_session_survives_normalization_and_invalid_indices_are_rejected() {
        let native = json!({"active_tab": 0, "recent": [], "tabs": [{"active": "a", "panes": [{"id":"a"}], "layout": {"leaf":"a"}}]});
        assert_eq!(normalize(native.clone()), Some(native));
        let bad = json!({"tabs": [{"active":0, "panes":[{}], "layout":{"Leaf":4}}]});
        assert!(normalize(bad).is_none());
    }
}
