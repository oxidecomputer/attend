use crate::model::EditorState;

pub fn format_human(state: &EditorState) -> String {
    let files = state.files.iter().map(|f| f.to_string());
    let terminals = state.terminals.iter().map(|t| format!("{t} $"));
    files.chain(terminals).collect::<Vec<_>>().join("\n")
}

pub fn format_json(state: &EditorState) -> String {
    serde_json::to_string_pretty(state).unwrap_or_default()
}
