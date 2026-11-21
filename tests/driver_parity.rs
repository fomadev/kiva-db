// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot I of docs/architecture/RC5_ARCHITECTURE.md (section 10): the table
//! of what each client can do (DOCUMENTATION.md 24.10) lists every
//! structured request the server answers, and the driver methods it names
//! exist in the drivers.

const API: &str = include_str!("../src/api.rs");
const MANUAL: &str = include_str!("../DOCUMENTATION.md");
const NODE: &str = include_str!("../drivers/node/index.js");
const PYTHON: &str = include_str!("../drivers/python/src/kivadb/client.py");

/// The operations of `api::handle`: its match arms `"op" =>` and `"a" | "b" =>`.
fn operations() -> Vec<String> {
    // A checkout on Windows may hold CRLF line ends.
    let api = API.replace("\r\n", "\n");
    let start = api.find("pub fn handle(sess: &mut Session, req: &Json)").expect("api::handle");
    let body = &api[start..];
    let end = body.find("\n}\n").expect("end of handle");
    let mut ops = Vec::new();
    for line in body[..end].lines() {
        let t = line.trim();
        // Arms of the outer match only: eight spaces of indentation.
        if !line.starts_with("        \"") || line.starts_with("         ") || !t.contains("=>") {
            continue;
        }
        let head = t.split("=>").next().unwrap();
        for part in head.split('|') {
            let name = part.trim().trim_matches('"');
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                ops.push(name.to_string());
            }
        }
    }
    ops
}

/// The rows of the table, as cells.
fn rows() -> Vec<Vec<String>> {
    let manual = MANUAL.replace("\r\n", "\n");
    let start = manual.find("<!-- parity -->").expect("the parity table");
    let end = manual.find("<!-- /parity -->").expect("its end");
    manual[start..end]
        .lines()
        .filter(|l| l.starts_with('|') && !l.starts_with("| :") && !l.starts_with("| Operation"))
        .map(|l| l.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect())
        .collect()
}

#[test]
fn every_structured_request_has_a_row() {
    let ops = operations();
    assert!(ops.len() >= 14 && ops.contains(&"collections".to_string()), "{ops:?}");
    let table = rows();
    for op in &ops {
        let listed = table.iter().any(|r| r[2].contains(&format!("`{op}`")));
        assert!(listed, "the structured request '{op}' is missing from the table of DOCUMENTATION.md 24.10");
    }
}

#[test]
fn the_driver_methods_of_the_table_exist() {
    let methods = |cell: &str| -> Vec<String> {
        cell.split('`')
            .skip(1)
            .step_by(2)
            .filter_map(|code| code.split('(').next().filter(|_| code.contains('(')))
            .map(String::from)
            .collect()
    };
    let mut checked = 0;
    for row in rows() {
        for m in methods(&row[4]) {
            let found = m == "connect" || NODE.contains(&format!("async {m}(")) || NODE.contains(&format!("  {m}("));
            assert!(found, "Node.js driver has no {m}() (row {})", row[0]);
            checked += 1;
        }
        for m in methods(&row[5]) {
            let found = m == "connect" || PYTHON.contains(&format!("def {m}("));
            assert!(found, "Python driver has no {m}() (row {})", row[0]);
            checked += 1;
        }
    }
    assert!(checked > 25, "{checked}");
}
