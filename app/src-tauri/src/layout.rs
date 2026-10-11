//! `filters/display.json`: which screens exist, and which panels and charts each holds.
//!
//! ```json
//! {"screens": [{"name": "top", "title": "Flush / reversal", "rows": [{"height": 1, "columns": [
//!     {"width": 1, "items": [{"filters": ["M_Open_Flush_Up"], "link": "purple"}]},
//!     {"width": 2, "items": [{"chart": "purple"}]}
//! ]}]}]}
//! ```
//!
//! A screen is rows (weighted by `height`) of columns (weighted by `width`) of stacked items.
//! An item is a **panel** (`filters`: one or more filters, shown together with a `Filter` column
//! when there's more than one) or an embedded **chart** (`chart`: its link colour). A panel's `link`
//! colour sends a clicked symbol to every chart of that colour, so several panels can drive one
//! chart and one panel can drive several. `height` weights an item within its column (default 1).
//!
//! Checked against the loaded filters: an unknown name refuses the layout (a typo would otherwise
//! show an empty panel forever), and filters that appear in no panel are reported.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tp_scan_engine::compile::{Compiled, Kind, Program, Side};

pub const LINK_COLORS: [&str; 8] = ["red", "blue", "green", "yellow", "orange", "purple", "pink", "cyan"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Display {
    pub screens: Vec<Screen>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Screen {
    pub name: String,
    #[serde(default)]
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    #[serde(default = "one")]
    pub height: f64,
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    #[serde(default = "one")]
    pub width: f64,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub filters: Vec<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub chart: Option<String>,
    #[serde(default = "one")]
    pub height: f64,
}

fn one() -> f64 {
    1.0
}

/// What the screens need to know about a filter.
#[derive(Debug, Clone, Serialize)]
pub struct FilterMeta {
    pub name: String,
    pub kind: &'static str,
    pub side: &'static str,
    pub hours: String,
    pub cooldown_s: i64,
    pub columns: Vec<String>,
    pub ranked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Layout {
    pub display: Display,
    pub filters: Vec<FilterMeta>,
    /// Loaded filters that no panel shows.
    pub not_displayed: Vec<String>,
}

pub fn side_str(s: Side) -> &'static str {
    match s {
        Side::Long => "long",
        Side::Short => "short",
        Side::Neutral => "neutral",
    }
}

pub fn filter_meta(p: &Program, c: &Compiled) -> Vec<FilterMeta> {
    c.filters()
        .iter()
        .map(|f| {
            let def = p.filters.iter().find(|d| d.name == f.name);
            FilterMeta {
                name: f.name.to_string(),
                kind: if f.kind == Kind::List { "list" } else { "trigger" },
                side: side_str(f.side),
                hours: format!("{:?}", f.hours).to_lowercase(),
                cooldown_s: f.cooldown_ms / 1000,
                columns: def.map(|d| d.columns.iter().map(|c| c.0.to_string()).collect()).unwrap_or_default(),
                ranked: def.is_some_and(|d| d.rank.is_some()),
            }
        })
        .collect()
}

/// Parse and check a layout against the loaded filters.
pub fn check(text: &str, filters: Vec<FilterMeta>) -> Result<Layout, String> {
    let display: Display = serde_json::from_str(text).map_err(|e| format!("display.json: {e}"))?;
    let known: HashSet<&str> = filters.iter().map(|f| f.name.as_str()).collect();
    let mut shown = HashSet::new();
    let mut names = HashSet::new();
    for s in &display.screens {
        if s.name.is_empty() || !s.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("display.json: screen name `{}` must be letters, digits, - or _", s.name));
        }
        if !names.insert(s.name.as_str()) {
            return Err(format!("display.json: screen `{}` appears twice", s.name));
        }
        for col in s.rows.iter().flat_map(|r| &r.columns) {
            for it in &col.items {
                match (&it.chart, it.filters.is_empty()) {
                    (Some(_), false) => return Err(format!("display.json ({}): an item is a chart or a panel, not both", s.name)),
                    (None, true) => return Err(format!("display.json ({}): an item needs `filters` or `chart`", s.name)),
                    _ => {}
                }
                for c in it.chart.iter().chain(it.link.iter()) {
                    if !LINK_COLORS.contains(&c.as_str()) {
                        return Err(format!("display.json ({}): link colour `{c}` is not one of {}", s.name, LINK_COLORS.join(", ")));
                    }
                }
                for f in &it.filters {
                    if !known.contains(f.as_str()) {
                        return Err(format!("display.json ({}): no filter named `{f}`", s.name));
                    }
                    shown.insert(f.clone());
                }
            }
        }
    }
    let not_displayed = filters.iter().filter(|f| !shown.contains(&f.name)).map(|f| f.name.clone()).collect();
    Ok(Layout { display, filters, not_displayed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str) -> FilterMeta {
        FilterMeta { name: name.into(), kind: "trigger", side: "long", hours: "rth".into(), cooldown_s: 60, columns: vec![], ranked: false }
    }

    #[test]
    fn a_good_layout_reports_what_it_leaves_out() {
        let t = r#"{"screens":[{"name":"top","rows":[{"columns":[{"items":[{"filters":["A"],"link":"red"},{"chart":"red"}]}]}]}]}"#;
        let l = check(t, vec![meta("A"), meta("B")]).unwrap();
        assert_eq!(l.not_displayed, vec!["B".to_string()]);
        assert_eq!(l.display.screens[0].rows[0].columns[0].width, 1.0);
    }

    #[test]
    fn mistakes_are_refused_by_name() {
        let cases = [
            (r#"{"screens":[{"name":"top","rows":[{"columns":[{"items":[{"filters":["Typo"]}]}]}]}]}"#, "Typo"),
            (r#"{"screens":[{"name":"top","rows":[{"columns":[{"items":[{"chart":"mauve"}]}]}]}]}"#, "mauve"),
            (r#"{"screens":[{"name":"top","rows":[{"columns":[{"items":[{}]}]}]}]}"#, "needs"),
            (r#"{"screens":[{"name":"a b","rows":[]}]}"#, "a b"),
            (r#"{"screens":[{"name":"x","rows":[]},{"name":"x","rows":[]}]}"#, "twice"),
        ];
        for (t, word) in cases {
            let e = check(t, vec![meta("A")]).unwrap_err();
            assert!(e.contains(word), "{e}");
        }
    }

    #[test]
    fn the_shipped_layout_shows_every_filter() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../filters");
        let p = tp_scan_engine::lang::load_dir(&dir).unwrap();
        let c = Compiled::new(&p).unwrap();
        let text = std::fs::read_to_string(dir.join("display.json")).unwrap();
        let l = check(&text, filter_meta(&p, &c)).unwrap();
        assert!(l.not_displayed.is_empty(), "not on any screen: {:?}", l.not_displayed);
    }
}
