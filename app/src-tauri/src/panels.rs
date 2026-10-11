//! The panel windows: which filters each shows, its link colour, whether it drives its charts on
//! each new fire (follow), its sound, and the group (a Primus screen: left, right, top, fourth) it
//! opens with. Seeded once from `filters/display.json`, then owned by the user (right-click menu,
//! + panel) and saved in the data dir as `panels.json`.
//!
//! `tile` is where a window goes the first time it opens: x, y, w, h as fractions of its group's
//! monitor, from the Primus grid. After that the window sits wherever the user left it.

use crate::layout::{Display, LINK_COLORS};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelDef {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub filters: Vec<String>,
    pub link: String,
    #[serde(default = "yes")]
    pub follow: bool,
    #[serde(default)]
    pub sound: bool,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub tile: Option<[f64; 4]>,
}

/// A chart that opens with a group (the charts that sat inside the Primus screens).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChartSeed {
    pub label: String,
    pub color: String,
    pub group: String,
    #[serde(default)]
    pub tile: Option<[f64; 4]>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Panels {
    pub panels: Vec<PanelDef>,
    #[serde(default)]
    pub charts: Vec<ChartSeed>,
}

fn yes() -> bool {
    true
}

impl Panels {
    /// Groups in the order they first appear (the launcher's screen buttons).
    pub fn groups(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let all = self.panels.iter().map(|p| &p.group).chain(self.charts.iter().map(|c| &c.group));
        for g in all {
            if !g.is_empty() && !out.contains(g) {
                out.push(g.clone());
            }
        }
        out
    }

    pub fn get(&self, id: &str) -> Option<&PanelDef> {
        self.panels.iter().find(|p| p.id == id)
    }

    /// A fresh id that no panel uses.
    pub fn new_id(&self) -> String {
        (1..).map(|n| format!("p{n}")).find(|id| self.get(id).is_none()).expect("a free id")
    }
}

/// Check an edit from a window: a known id, a real colour, a sane id. Filter names are not checked
/// here: a panel naming a filter that a later edit removed shows it as missing rather than being
/// refused.
pub fn validate(p: &PanelDef) -> Result<(), String> {
    if !LINK_COLORS.contains(&p.link.as_str()) {
        return Err(format!("no link colour `{}`", p.link));
    }
    if p.id.is_empty() || !p.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("panel id `{}` must be letters, digits, - or _", p.id));
    }
    Ok(())
}

/// The Primus screens as panel windows: every grid cell becomes a window tiled where the cell sat.
pub fn seed(d: &Display) -> Panels {
    let mut out = Panels::default();
    for s in &d.screens {
        let (mut n, mut charts) = (0, 0);
        let rows_h: f64 = s.rows.iter().map(|r| r.height).sum::<f64>().max(f64::EPSILON);
        let mut y = 0.0;
        for row in &s.rows {
            let h = row.height / rows_h;
            let cols_w: f64 = row.columns.iter().map(|c| c.width).sum::<f64>().max(f64::EPSILON);
            let mut x = 0.0;
            for col in &row.columns {
                let w = col.width / cols_w;
                let items_h: f64 = col.items.iter().map(|i| i.height).sum::<f64>().max(f64::EPSILON);
                let mut iy = y;
                for it in &col.items {
                    let ih = h * it.height / items_h;
                    let tile = Some([x, iy, w, ih]);
                    if let Some(color) = &it.chart {
                        charts += 1;
                        out.charts.push(ChartSeed { label: format!("chart-{}-{charts}", s.name), color: color.clone(), group: s.name.clone(), tile });
                    } else {
                        n += 1;
                        out.panels.push(PanelDef {
                            id: format!("{}-{n}", s.name),
                            title: it.title.clone(),
                            filters: it.filters.clone(),
                            link: it.link.clone().unwrap_or_else(|| "red".into()),
                            follow: true,
                            sound: false,
                            group: s.name.clone(),
                            tile,
                        });
                    }
                    iy += ih;
                }
                x += w;
            }
            y += h;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_becomes_tiled_windows() {
        let t = r#"{"screens":[{"name":"top","rows":[
            {"height":3,"columns":[{"width":1,"items":[{"filters":["A"],"link":"blue"},{"filters":["B"],"height":3}]},{"width":3,"items":[{"chart":"blue"}]}]},
            {"height":1,"columns":[{"items":[{"filters":["C","D"],"title":"CD"}]}]}
        ]}]}"#;
        let d: Display = serde_json::from_str(t).unwrap();
        let p = seed(&d);
        assert_eq!(p.panels.len(), 3);
        assert_eq!(p.panels[0].id, "top-1");
        assert_eq!(p.panels[0].link, "blue");
        assert_eq!(p.panels[0].tile, Some([0.0, 0.0, 0.25, 0.1875]));
        assert_eq!(p.panels[1].tile, Some([0.0, 0.1875, 0.25, 0.5625]));
        assert_eq!(p.panels[2].tile, Some([0.0, 0.75, 1.0, 0.25]));
        assert_eq!(p.panels[2].title.as_deref(), Some("CD"));
        assert_eq!(p.charts, vec![ChartSeed { label: "chart-top-1".into(), color: "blue".into(), group: "top".into(), tile: Some([0.25, 0.0, 0.75, 0.75]) }]);
        assert_eq!(p.groups(), vec!["top".to_string()]);
        assert_eq!(p.new_id(), "p1");
    }

    #[test]
    fn the_shipped_layout_seeds_every_screen() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../filters");
        let d: Display = serde_json::from_str(&std::fs::read_to_string(dir.join("display.json")).unwrap()).unwrap();
        let p = seed(&d);
        assert_eq!(p.groups(), vec!["left", "right", "top", "fourth"]);
        for x in &p.panels {
            validate(x).unwrap();
            let [a, b, w, h] = x.tile.unwrap();
            assert!(a >= 0.0 && b >= 0.0 && a + w <= 1.0 + 1e-9 && b + h <= 1.0 + 1e-9, "{x:?}");
        }
    }

    #[test]
    fn bad_edits_are_refused() {
        let mut p = PanelDef { id: "p1".into(), title: None, filters: vec![], link: "mauve".into(), follow: true, sound: false, group: String::new(), tile: None };
        assert!(validate(&p).unwrap_err().contains("mauve"));
        p.link = "red".into();
        p.id = "a b".into();
        assert!(validate(&p).is_err());
    }
}
