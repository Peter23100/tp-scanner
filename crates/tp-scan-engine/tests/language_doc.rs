//! `filters/LANGUAGE.md` stays true: every cookbook example (§6) compiles against the real filters,
//! the example filters in `examples/recipes.scan` are the doc's, verbatim, and the fields and
//! conditions the doc quotes are the ones `filters/common.scan` defines.

use std::path::{Path, PathBuf};
use tp_scan_engine::compile::Compiled;
use tp_scan_engine::lang::parse_sources;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}")).replace("\r\n", "\n")
}

/// Every `filters/*.scan` file, as (name, text).
fn filter_sources() -> Vec<(String, String)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root().join("filters"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "scan"))
        .collect();
    paths.sort();
    paths.iter().map(|p| (p.display().to_string(), std::fs::read_to_string(p).unwrap())).collect()
}

fn compiles(extra: &[(&str, &str)]) {
    let owned = filter_sources();
    let mut srcs: Vec<(&str, &str)> = owned.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    srcs.extend_from_slice(extra);
    let p = parse_sources(&srcs).unwrap_or_else(|e| panic!("{e}"));
    Compiled::new(&p).unwrap_or_else(|e| panic!("{e:?}"));
}

/// The cookbook: from `## 6.` up to `## 7.`.
fn cookbook() -> String {
    let doc = read("filters/LANGUAGE.md");
    let a = doc.find("## 6. Cookbook").expect("§6");
    let b = doc.find("## 7.").expect("§7");
    doc[a..b].to_string()
}

fn fenced(text: &str) -> Vec<String> {
    text.split("```").skip(1).step_by(2).map(|b| b.trim_start_matches('\n').trim_end().to_string()).collect()
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_example_filters_are_the_docs_and_compile_with_the_real_ones() {
    let recipes = read("examples/recipes.scan");
    let blocks: Vec<String> = fenced(&cookbook()).into_iter().filter(|b| b.starts_with("trigger") || b.starts_with("list")).collect();
    assert!(blocks.len() >= 6, "{}", blocks.len());
    for b in &blocks {
        assert!(recipes.contains(b.as_str()), "examples/recipes.scan is missing, or differs from, the doc's:\n{b}");
    }
    compiles(&[("examples/recipes.scan", &recipes)]);
}

#[test]
fn the_docs_fields_and_conditions_are_the_ones_common_scan_defines() {
    let common = squash(&read("filters/common.scan"));
    let book = cookbook();
    // Fenced definitions, and the `field ...` cells of the §6.1 table.
    let mut defs: Vec<String> = fenced(&book).into_iter().filter(|b| b.starts_with("condition")).collect();
    for cell in book.split('`').skip(1).step_by(2) {
        if cell.starts_with("field ") {
            defs.push(cell.to_string());
        }
    }
    assert!(defs.len() >= 10, "{}", defs.len());
    for d in &defs {
        assert!(common.contains(&squash(d)), "filters/common.scan doesn't define, as written in the doc:\n{d}");
    }
}

#[test]
fn every_pattern_in_the_table_compiles() {
    let book = cookbook();
    let table = &book[book.find("### 6.6").expect("§6.6")..];
    let mut n = 0;
    for row in table.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| idea")) {
        let cell = row.split('|').nth(2).expect("two columns").trim();
        let expr = cell.trim_matches('`');
        let src = if expr.contains(['<', '>']) { format!("condition doc_pattern = {expr}") } else { format!("field doc_pattern = {expr}") };
        compiles(&[("doc §6.6", &src)]);
        n += 1;
    }
    assert!(n >= 10, "{n}");
}
