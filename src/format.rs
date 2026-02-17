use crate::model::EditorState;

pub fn format_human(state: &EditorState) -> String {
    state
        .files
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_json(state: &EditorState) -> String {
    serde_json::to_string_pretty(state).unwrap_or_default()
}
