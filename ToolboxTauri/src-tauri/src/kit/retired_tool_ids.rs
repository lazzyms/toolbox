use std::collections::BTreeMap;

use serde::Deserialize;

use crate::kit::contracts::{JobOutcome, OutcomePointer, OutcomeState};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetiredToolIdTable {
    version: u32,
    retired: BTreeMap<String, String>,
}

fn parse_retired_tool_ids(raw: &str) -> Result<RetiredToolIdTable, String> {
    let table: RetiredToolIdTable = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    for (retired_id, replacement) in &table.retired {
        if retired_id.is_empty() || replacement.is_empty() || retired_id == replacement {
            return Err(format!("{retired_id} must map to another tool id or removed."));
        }
        if table.retired.contains_key(replacement) {
            return Err(format!("{retired_id} maps to {replacement}, which is retired too."));
        }
    }
    Ok(table)
}

fn shipped_table() -> RetiredToolIdTable {
    parse_retired_tool_ids(include_str!("../../../shared/retired-tool-ids.json")).unwrap()
}

fn rust_sources(directory: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .flat_map(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_sources(&path)
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                vec![std::fs::read_to_string(path).unwrap()]
            } else {
                Vec::new()
            }
        })
        .collect()
}

fn declared_pointer_targets() -> Vec<String> {
    let mut targets = Vec::new();
    for source in rust_sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")) {
        for (start, _) in source.match_indices("tool_id: \"") {
            let rest = &source[start + "tool_id: \"".len()..];
            targets.push(rest[..rest.find('"').unwrap()].to_string());
        }
    }
    let samples: Vec<JobOutcome> =
        serde_json::from_str(include_str!("../../../shared/job-outcome-samples.json")).unwrap();
    for outcome in samples {
        let pointer = match outcome.state {
            OutcomeState::Skipped { reason } => reason.pointer,
            OutcomeState::Failed { error, .. } => error.pointer,
            OutcomeState::Succeeded { .. } => None,
        };
        if let Some(OutcomePointer::Tool { tool_id }) = pointer {
            targets.push(tool_id);
        }
    }
    targets
}

fn retired_targets(targets: &[String], table: &RetiredToolIdTable) -> Vec<String> {
    targets.iter().filter(|target| table.retired.contains_key(*target)).cloned().collect()
}

#[test]
fn the_shipped_table_parses_with_a_version() {
    let table = shipped_table();
    assert!(table.retired.is_empty() || table.version > 0, "rows need a migration version above zero");
}

#[test]
fn the_retired_table_rejects_drift_prone_rows() {
    assert!(parse_retired_tool_ids(r#"{"version":1,"retired":{"a-tool":"a-tool"}}"#).is_err());
    assert!(parse_retired_tool_ids(r#"{"version":1,"retired":{"a-tool":"b-tool","b-tool":"removed"}}"#).is_err());
    assert!(parse_retired_tool_ids(r#"{"version":-1,"retired":{}}"#).is_err());
    assert!(parse_retired_tool_ids(r#"{"version":1,"retired":{},"aliases":{}}"#).is_err());
    assert!(parse_retired_tool_ids(r#"{"version":1,"retired":{"a-tool":"removed","b-tool":"pdf-unlock"}}"#).is_ok());
}

#[test]
fn tool_pointer_targets_are_never_retired_ids() {
    let targets = declared_pointer_targets();
    assert!(targets.iter().any(|target| target == "heic-convert"), "the scan finds the declared pointer targets");
    assert_eq!(retired_targets(&targets, &shipped_table()), Vec::<String>::new());

    let retiring_heic = parse_retired_tool_ids(r#"{"version":1,"retired":{"heic-convert":"removed"}}"#).unwrap();
    assert!(retired_targets(&targets, &retiring_heic).contains(&"heic-convert".to_string()));
}
