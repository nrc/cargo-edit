#[macro_use]
extern crate pretty_assertions;

use std::fs::File;
use std::io::Read;

pub mod utils;
use crate::utils::{clone_out_test, clone_out_workspace_test, execute_command};

#[test]
fn orders_deps() {
    let (_tmpdir, manifest) = clone_out_test("tests/fixtures/tidy/Cargo.toml.source");

    execute_command(&["tidy"], &manifest);

    let tidied = read_manifest(&manifest);
    let deps = get_deps(&tidied);
    let mut sorted = deps.clone();
    sorted.sort();

    assert_eq!(deps, sorted);
}

#[test]
fn orders_deps_all() {
    let (_tmpdir, root_manifest, workspace_manifests) = clone_out_workspace_test();

    execute_command(&["tidy", "--all"], &root_manifest);

    for manifest in workspace_manifests {
        let tidied = read_manifest(&manifest);
        let deps = get_deps(&tidied);
        let mut sorted = deps.clone();
        sorted.sort();

        assert_eq!(deps, sorted);
    }
}

fn read_manifest(manifest_path: &str) -> String {
    let mut f = File::open(&manifest_path).unwrap();
    let mut manifest = String::new();
    f.read_to_string(&mut manifest).unwrap();

    manifest
}

fn get_deps(manifest: &str) -> Vec<String> {
    let start = manifest.find("[dependencies]").unwrap();
    let lines = manifest[start..].lines();

    lines
        .skip(1)
        .take_while(|l| !(l.starts_with('[') || l.is_empty()))
        .map(|s| s.to_owned())
        .collect()
}
