//! Smoke tests against a corpus of real SHOWPLAN XML documents captured from
//! a live SQL Server 2025 instance running AdventureWorks2022.
//!
//! Every `.xml` file under `tests/data/aw/` must parse without error and yield
//! at least one [`PlanNode`].

use std::fs;
use std::path::PathBuf;

use sqldev_showplan::{parse_all, PlanNode};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("aw")
}

fn count_nodes(node: &PlanNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

#[test]
fn every_aw_plan_parses() {
    let dir = corpus_dir();
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "xml"))
        .collect();
    files.sort();

    assert!(
        !files.is_empty(),
        "no fixtures found under {}",
        dir.display()
    );

    let mut failures: Vec<String> = Vec::new();
    let mut total_nodes = 0usize;

    for path in &files {
        let xml = fs::read_to_string(path).expect("read fixture");
        match parse_all(&xml) {
            Ok(plans) => {
                if plans.is_empty() {
                    failures.push(format!("{}: parsed but yielded zero plans", path.display()));
                    continue;
                }
                for plan in &plans {
                    let nodes = count_nodes(&plan.root);
                    assert!(nodes > 0, "{}: plan has no nodes", path.display());
                    total_nodes += nodes;
                }
            }
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} fixtures failed to parse:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
    assert!(total_nodes > 0);
}

#[test]
fn missing_index_fixture_surfaces_suggestion() {
    let path = corpus_dir().join("07_subquery_exists.xml");
    if !path.exists() {
        return;
    }
    let xml = fs::read_to_string(&path).unwrap();
    let plans = parse_all(&xml).expect("parse");
    let plan = &plans[0];
    assert!(
        !plan.missing_indexes.is_empty(),
        "expected MissingIndex suggestion in {}",
        path.display()
    );
}

#[test]
fn multi_statement_batch_yields_multiple_plans() {
    let path = corpus_dir().join("14_multi_statement_batch.xml");
    if !path.exists() {
        return;
    }
    let xml = fs::read_to_string(&path).unwrap();
    let plans = parse_all(&xml).expect("parse");
    assert!(
        plans.len() >= 2,
        "expected ≥2 plans for multi-statement batch, got {}",
        plans.len()
    );
}

#[test]
fn actual_plan_has_runtime_stats() {
    let path = corpus_dir().join("15_actual_plan_simple.xml");
    if !path.exists() {
        return;
    }
    let xml = fs::read_to_string(&path).unwrap();
    let plans = parse_all(&xml).expect("parse");
    let plan = &plans[0];

    fn has_runtime(node: &PlanNode) -> bool {
        node.runtime_stats.is_some() || node.children.iter().any(has_runtime)
    }
    assert!(
        has_runtime(&plan.root),
        "actual plan should have RunTimeCountersPerThread on at least one node"
    );
}

#[test]
fn scan_or_seek_captures_object() {
    let path = corpus_dir().join("01_simple_select.xml");
    if !path.exists() {
        return;
    }
    let xml = fs::read_to_string(&path).unwrap();
    let plans = parse_all(&xml).expect("parse");
    let plan = &plans[0];

    fn find_object(node: &PlanNode) -> bool {
        node.object.is_some() || node.children.iter().any(find_object)
    }
    assert!(
        find_object(&plan.root),
        "expected at least one node with an Object reference"
    );
}
