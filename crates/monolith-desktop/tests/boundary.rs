//! The crate boundary of the desktop client
//! (`docs/adr/0011-ui-security-boundary.md`): the UI crates reach no
//! Monolith crate but each other, through any path of the dependency
//! graph, and the desktop reaches the core only through the runtime.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
}

/// The packages of the lock file with the names of what they depend on
/// (dev-dependencies included).
fn lock() -> BTreeMap<String, Vec<String>> {
    let text = std::fs::read_to_string(workspace().join("Cargo.lock")).unwrap();
    let mut packages = BTreeMap::new();
    for block in text.split("[[package]]").skip(1) {
        let mut name = None;
        let mut dependencies = Vec::new();
        let mut in_dependencies = false;
        for line in block.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("name = ") {
                name = Some(value.trim_matches('"').to_owned());
            } else if line.starts_with("dependencies = [") {
                in_dependencies = true;
            } else if in_dependencies && line == "]" {
                in_dependencies = false;
            } else if in_dependencies {
                // "name" or "name version" when two versions exist.
                let entry = line.trim_matches(|c| c == '"' || c == ',');
                dependencies.push(entry.split(' ').next().unwrap().to_owned());
            }
        }
        packages
            .entry(name.unwrap())
            .or_insert_with(Vec::new)
            .extend(dependencies);
    }
    packages
}

/// Every package `root` reaches.
fn closure(packages: &BTreeMap<String, Vec<String>>, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(name) = pending.pop() {
        if seen.insert(name.clone()) {
            pending.extend(packages.get(&name).into_iter().flatten().cloned());
        }
    }
    seen
}

#[test]
fn the_ui_crates_reach_no_monolith_crate_but_each_other() {
    let packages = lock();
    let allowed = [
        "monolith-ui",
        "monolith-ui-kit",
        "monolith-ui-model",
        "monolith-ui-snapshot",
    ];
    for root in ["monolith-ui", "monolith-ui-kit", "monolith-ui-model"] {
        assert!(
            packages.contains_key(root),
            "{root} is not in the lock file"
        );
        let reached: Vec<String> = closure(&packages, root)
            .into_iter()
            .filter(|name| name.starts_with("monolith") && !allowed.contains(&name.as_str()))
            .collect();
        assert!(reached.is_empty(), "{root} reaches {reached:?}");
    }
}

#[test]
fn the_ui_model_depends_on_no_toolkit() {
    let packages = lock();
    let reached = closure(&packages, "monolith-ui-model");
    for toolkit in ["egui", "eframe", "winit", "epaint"] {
        assert!(!reached.contains(toolkit), "the UI model reaches {toolkit}");
    }
}

#[test]
fn the_desktop_reaches_the_core_only_through_the_runtime() {
    let packages = lock();
    let direct: BTreeSet<&str> = packages["monolith-desktop"]
        .iter()
        .map(String::as_str)
        .filter(|name| name.starts_with("monolith"))
        .collect();
    // monolith-tor is a dev-dependency, for the mock network of the tests,
    // as the manifest is checked for below; monolith-ui-snapshot serves the
    // screenshot feature of development builds only.
    let expected: BTreeSet<&str> = [
        "monolith-runtime",
        "monolith-tor",
        "monolith-ui",
        "monolith-ui-kit",
        "monolith-ui-model",
        "monolith-ui-snapshot",
    ]
    .into_iter()
    .collect();
    assert_eq!(direct, expected);
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let dependencies = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    for name in [
        "monolith-core",
        "monolith-identity",
        "monolith-protocol",
        "monolith-session",
        "monolith-storage",
        "monolith-tor",
    ] {
        assert!(
            !dependencies
                .lines()
                .any(|line| line.trim_start().starts_with(name)),
            "the desktop depends on {name}"
        );
    }
}
