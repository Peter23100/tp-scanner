//! Dry run of the download universe's mover rule over the scanner's daily cache.
//!
//! cargo run --release -p tp-history --example movers -- [--from 2026-01-01] [--to 2026-12-31]
//!     [--daily <dir>] [--change 5] [--gap 4] [--price 2] [--dollar-volume 20000000] [--list]
//!
//! Reads `<daily>/YYYY-MM-DD.csv` (`symbol,open,high,low,close,volume`, as printed) and
//! `<daily>/splits.json`; default dir `%LOCALAPPDATA%\tp-scanner\daily`. Prints movers per day and
//! a summary. Read-only.

use chrono::NaiveDate;
use std::collections::HashMap;
use std::path::PathBuf;
use tp_history::universe::{movers, DayBar, MoverRules};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn read_day(path: &std::path::Path) -> Option<Vec<DayBar>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 6 {
            continue;
        }
        let n = |i: usize| f[i].parse::<f64>().ok();
        if let (Some(open), Some(high), Some(low), Some(close), Some(volume)) = (n(1), n(2), n(3), n(4), n(5)) {
            out.push(DayBar { symbol: f[0].to_string(), open, high, low, close, volume });
        }
    }
    Some(out)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = arg(&args, "--daily").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA")).join("tp-scanner").join("daily"));
    let from = arg(&args, "--from").map(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").expect("--from")).unwrap_or(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
    let to = arg(&args, "--to").map(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").expect("--to")).unwrap_or(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
    let d = MoverRules::default();
    let num = |n: &str, dv: f64| arg(&args, n).map(|s| s.parse::<f64>().expect(n)).unwrap_or(dv);
    let rules = MoverRules {
        min_change_pct: num("--change", d.min_change_pct),
        min_gap_pct: num("--gap", d.min_gap_pct),
        min_price: num("--price", d.min_price),
        min_dollar_volume: num("--dollar-volume", d.min_dollar_volume),
    };
    let list = args.iter().any(|a| a == "--list");

    // splits.json: [{"execution_date":"YYYY-MM-DD","factor":f,"ticker":"X"}]
    let mut splits: HashMap<(NaiveDate, String), f64> = HashMap::new();
    if let Ok(t) = std::fs::read_to_string(dir.join("splits.json")) {
        if let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(&t) {
            for s in a {
                if let (Some(d), Some(f), Some(sym)) = (s["execution_date"].as_str(), s["factor"].as_f64(), s["ticker"].as_str()) {
                    if let Ok(d) = NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                        splits.insert((d, sym.to_string()), f);
                    }
                }
            }
        }
    }

    let mut days: Vec<(NaiveDate, PathBuf)> = std::fs::read_dir(&dir)
        .expect("daily dir")
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let stem = p.file_stem()?.to_str()?.to_string();
            NaiveDate::parse_from_str(&stem, "%Y-%m-%d").ok().map(|d| (d, p))
        })
        .collect();
    days.sort();

    println!("rules: |change| >= {}% or |gap| >= {}%, close >= ${}, dollar volume >= ${:.0}M", rules.min_change_pct, rules.min_gap_pct, rules.min_price, rules.min_dollar_volume / 1e6);
    let mut prev: Option<HashMap<String, f64>> = None;
    let mut counts: Vec<(NaiveDate, usize, usize)> = Vec::new();
    for (day, path) in days {
        let Some(bars) = read_day(&path) else { continue };
        if bars.is_empty() {
            continue; // market closed
        }
        if day >= from && day <= to {
            if let Some(p) = &prev {
                let u = movers(p, &bars, |s| splits.get(&(day, s.to_string())).copied().unwrap_or(1.0), &rules);
                counts.push((day, u.movers.len(), u.no_reference.len()));
                if list {
                    let top: Vec<String> = u.movers.iter().take(15).map(|m| format!("{} {:+.1}%", m.symbol, m.change_pct)).collect();
                    println!("{day}  {:>4} movers  {}", u.movers.len(), top.join(", "));
                }
            }
        }
        prev = Some(bars.into_iter().map(|b| (b.symbol, b.close)).collect());
    }
    if counts.is_empty() {
        println!("no days in range");
        return;
    }
    let mut n: Vec<usize> = counts.iter().map(|c| c.1).collect();
    n.sort();
    let total: usize = n.iter().sum();
    let (min_d, max_d) = (counts.iter().min_by_key(|c| c.1).unwrap(), counts.iter().max_by_key(|c| c.1).unwrap());
    println!(
        "{} days {}..{}: movers/day median {}  mean {:.0}  min {} ({})  max {} ({})  total symbol-days {}",
        counts.len(),
        counts[0].0,
        counts[counts.len() - 1].0,
        n[n.len() / 2],
        total as f64 / n.len() as f64,
        min_d.1,
        min_d.0,
        max_d.1,
        max_d.0,
        total
    );
    let noref: usize = counts.iter().map(|c| c.2).sum();
    println!("liquid symbols with no previous close (new listings / ticker changes), all days: {noref}");
}
