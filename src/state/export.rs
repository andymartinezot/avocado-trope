//! `avocado-trope inspect --json` and `--markdown`: a session as data, for scripts and
//! for pasting into a PR. Built from the same fold as the graph.
//!
//! These contain what the session contains: prompts, file paths, tool
//! summaries and failure text. Read before sharing.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::fact::AgentKind;
use crate::state::info::SessionInfo;
use crate::state::session::{AgentInfo, SessionModel, ToolState};

fn label(a: &AgentInfo) -> &str {
    a.agent_type.as_deref().unwrap_or(a.kind.default_label())
}

fn elapsed_secs(model: &SessionModel) -> Option<i64> {
    let start = model.agent("main")?.first_ts?;
    let end = model.agents.values().filter_map(|a| a.last_ts).max()?;
    Some((end - start).num_seconds().max(0))
}

/// Every file changed in the session → created by it.
fn files(model: &SessionModel) -> BTreeMap<String, bool> {
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    for a in model.agents.values() {
        for (path, created) in a.files.iter() {
            *out.entry(path.clone()).or_default() |= *created;
        }
    }
    out
}

/// Failed calls in spawn order: (agent label, tool, summary, error).
fn failures(model: &SessionModel) -> Vec<(&str, &str, &str, &str)> {
    model
        .spawn_order()
        .filter_map(|id| model.agent(id))
        .flat_map(|a| {
            a.tool_calls()
                .filter(|t| t.state == ToolState::Err)
                .map(move |t| {
                    (
                        label(a),
                        t.name.as_str(),
                        t.summary.as_deref().unwrap_or(""),
                        t.error.as_deref().unwrap_or(""),
                    )
                })
        })
        .collect()
}

/// The session as one JSON object.
pub fn json(model: &SessionModel, info: &SessionInfo) -> Value {
    let agents: Vec<Value> = model
        .spawn_order()
        .filter_map(|id| Some((id, model.agent(id)?)))
        .map(|(id, a)| {
            json!({
                "id": id,
                "kind": match a.kind {
                    AgentKind::Main => "main",
                    AgentKind::Subagent => "subagent",
                    AgentKind::Group => "group",
                },
                "label": label(a),
                "description": a.description,
                "model": a.model,
                "status": a.status_word(),
                "tools": a.tool_calls().len(),
                "failed": a.tool_calls().filter(|t| t.state == ToolState::Err).count(),
                "output_tokens": a.output_tokens,
                "context_tokens": a.context_tokens,
                "context_window": crate::settings::effective_window(a.context_window, a.context_peak),
                "files": a.files.keys().collect::<Vec<_>>(),
            })
        })
        .collect();
    let failures: Vec<Value> = failures(model)
        .into_iter()
        .map(|(agent, tool, summary, error)| {
            json!({"agent": agent, "tool": tool, "summary": summary, "error": error})
        })
        .collect();
    let files: Vec<Value> = files(model)
        .into_iter()
        .map(|(path, created)| json!({"path": path, "created": created}))
        .collect();
    json!({
        "session": model.session_id,
        "title": info.title,
        "elapsed_secs": elapsed_secs(model),
        "tool_calls": model.tool_count(),
        "failed_calls": failures.len(),
        "output_tokens": model.agents.values().map(|a| a.output_tokens).sum::<u64>(),
        "prompts": model.prompts.iter().map(|p| p.excerpt.as_str()).collect::<Vec<_>>(),
        "agents": agents,
        "files": files,
        "failures": failures,
    })
}

/// Pipes and newlines would break out of a table cell.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// The session as a Markdown summary.
pub fn markdown(model: &SessionModel, info: &SessionInfo) -> String {
    let mut out = String::new();
    let title = info.title.as_deref().unwrap_or("Untitled session");
    out.push_str(&format!("# {}\n\n", title.replace(['\n', '\r'], " ")));
    let failed = failures(model);
    let files = files(model);
    let mut facts = vec![
        format!("{} agents", model.agent_count()),
        format!("{} tool calls", model.tool_count()),
        format!("{} failed", failed.len()),
        format!("{} files changed", files.len()),
    ];
    if let Some(s) = elapsed_secs(model) {
        facts.insert(0, format!("{}m {:02}s", s / 60, s % 60));
    }
    out.push_str(&format!("{}\n\n", facts.join(" · ")));

    out.push_str("## Agents\n\n| Agent | Model | Status | Tools | Failed | Output tokens |\n");
    out.push_str("|---|---|---|---:|---:|---:|\n");
    for a in model.spawn_order().filter_map(|id| model.agent(id)) {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            cell(label(a)),
            a.model
                .as_deref()
                .map(crate::theme::short_model)
                .unwrap_or_default(),
            a.status_word(),
            a.tool_calls().len(),
            a.tool_calls().filter(|t| t.state == ToolState::Err).count(),
            a.output_tokens,
        ));
    }
    if !files.is_empty() {
        out.push_str("\n## Files changed\n\n");
        for (path, created) in &files {
            let new = if *created { " (new)" } else { "" };
            // A backtick in a path would end the code span early.
            out.push_str(&format!("- `{}`{new}\n", path.replace('`', "'")));
        }
    }
    if !failed.is_empty() {
        out.push_str("\n## Failures\n\n| Agent | Tool | Call | Error |\n|---|---|---|---|\n");
        for (agent, tool, summary, error) in failed {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                cell(agent),
                cell(tool),
                cell(summary),
                cell(error)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_cell_cannot_break_out() {
        assert_eq!(cell("a|b\nc"), "a\\|b c");
    }

    #[test]
    fn an_empty_session_exports() {
        let model = SessionModel::new("s".into());
        let info = SessionInfo::default();
        assert_eq!(json(&model, &info)["session"], "s");
        assert!(markdown(&model, &info).starts_with("# Untitled session"));
    }
}
