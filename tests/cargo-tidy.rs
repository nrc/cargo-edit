#[macro_use]
extern crate pretty_assertions;

use std::fs::File;
use std::io::Read;

pub mod utils;
use crate::utils::{clone_out_test, execute_command};

#[test]
fn orders_deps() {
    let (_tmpdir, manifest) = clone_out_test("tests/fixtures/tidy/Cargo.toml.source");

    execute_command(&["tidy"], &manifest);

    let mut f = File::open(&manifest).unwrap();
    let mut tidied = String::new();
    f.read_to_string(&mut tidied).unwrap();

    let deps = get_deps(&tidied);
    let mut sorted = deps.clone();
    sorted.sort();

    assert_eq!(deps, sorted);
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
