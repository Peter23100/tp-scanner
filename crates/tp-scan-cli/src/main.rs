//! `tp-scan`: check filter files, and replay history through them.
//!
//! ```text
//! tp-scan check <filters-dir>
//! tp-scan replay --filters <dir> --bars <file> [options]
//! ```
//!
//! Replay options:
//!
//! | option | meaning |
//! |---|---|
//! | `--bars FILE` | intraday bars. `.csv`: `symbol,start_ny,open,high,low,close,volume` (New York wall time). A Massive minute-aggregate day file (optionally `.gz`) with `--massive`. Repeat for several days, in date order. |
//! | `--seconds` | the CSV bars are 1-second bars (default: 1-minute) |
//! | `--daily FILE` | split-adjusted daily bars, `symbol,date,open,high,low,close,volume`. Each session's reference (prior close, ATR, ADV, prior month, daily EMA) is built only from days before it. |
//! | `--float FILE` | `symbol,float_shares` |
//! | `--jsonl FILE` | write signals and list changes as JSON lines |
//! | `--instrument` | with `--jsonl`, also every evaluation's measurement vector (stage 4) |
//! | `--explain FILTER:SYMBOL` | after the replay, show why FILTER is or isn't true for SYMBOL (repeatable) |
//! | `--quiet` | don't print each signal |
//!
//! Output: one line per signal, the stats, and every non-empty list at the end of the replay.

mod files;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tp_marketdata::source::{ReplaySource, Resolution};
use tp_scan_engine::compile::{Hours, Kind, Side};
use tp_scan_engine::engine::{Engine, EngineError, ListChange, Signal, Sink};
use tp_scan_engine::instrument::JsonlSink;
use tp_scan_engine::lang::load_dir;

const USAGE: &str = "usage:
  tp-scan check <filters-dir>
  tp-scan replay --filters <dir> --bars <file> [--bars <file>...] [--massive] [--seconds]
                 [--daily <file>] [--float <file>] [--jsonl <file>] [--instrument]
                 [--explain FILTER:SYMBOL] [--quiet]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("check") if args.len() == 2 => check(Path::new(&args[1])),
        Some("replay") => parse_replay(&args[1..]).and_then(|o| replay(&o)),
        _ => Err(USAGE.to_string()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn side(s: Side) -> &'static str {
    match s {
        Side::Long => "long",
        Side::Short => "short",
        Side::Neutral => "-",
    }
}

fn hours(h: Hours) -> &'static str {
    match h {
        Hours::Rth => "rth",
        Hours::Pre => "pre",
        Hours::Extended => "extended",
        Hours::Any => "any",
    }
}

fn check(dir: &Path) -> Result<(), String> {
    let p = load_dir(dir).map_err(|e| e.to_string())?;
    let e = Engine::new(&p).map_err(|e| e.to_string())?;
    println!("{:<26} {:<7} {:<5} {:<8} {:>8}  {:<7} show", "filter", "kind", "side", "hours", "cooldown", "screen");
    for f in e.compiled().filters() {
        let kind = if f.kind == Kind::List { "list" } else { "trigger" };
        let cd = if f.kind == Kind::List { "-".to_string() } else { format!("{}m", f.cooldown_ms / 60_000) };
        let show: Vec<String> = p.filters.iter().find(|d| d.name == f.name).map(|d| d.columns.iter().map(|c| c.0.to_string()).collect()).unwrap_or_default();
        println!(
            "{:<26} {:<7} {:<5} {:<8} {:>8}  {:<7} {}",
            f.name,
            kind,
            side(f.side),
            hours(f.hours),
            cd,
            f.screen.as_deref().unwrap_or("-"),
            show.join(", ")
        );
    }
    let triggers = e.compiled().filters().iter().filter(|f| f.kind == Kind::Trigger).count();
    println!(
        "\nok: {} filters ({} triggers, {} lists), {} fields, {} conditions",
        e.compiled().filters().len(),
        triggers,
        e.compiled().filters().len() - triggers,
        p.num_defs.len(),
        p.cond_defs.len()
    );
    Ok(())
}

#[derive(Debug, Default)]
struct ReplayOpts {
    filters: Option<PathBuf>,
    bars: Vec<PathBuf>,
    massive: bool,
    seconds: bool,
    daily: Option<PathBuf>,
    float: Option<PathBuf>,
    jsonl: Option<PathBuf>,
    instrument: bool,
    explain: Vec<(String, String)>,
    quiet: bool,
}

fn parse_replay(args: &[String]) -> Result<ReplayOpts, String> {
    let mut o = ReplayOpts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value\n{USAGE}"));
        match a.as_str() {
            "--filters" => o.filters = Some(val("--filters")?.into()),
            "--bars" => o.bars.push(val("--bars")?.into()),
            "--daily" => o.daily = Some(val("--daily")?.into()),
            "--float" => o.float = Some(val("--float")?.into()),
            "--jsonl" => o.jsonl = Some(val("--jsonl")?.into()),
            "--explain" => {
                let v = val("--explain")?;
                let (f, s) = v.split_once(':').ok_or("--explain takes FILTER:SYMBOL")?;
                o.explain.push((f.to_string(), s.to_string()));
            }
            "--massive" => o.massive = true,
            "--seconds" => o.seconds = true,
            "--instrument" => o.instrument = true,
            "--quiet" => o.quiet = true,
            _ => return Err(format!("unknown option `{a}`\n{USAGE}")),
        }
    }
    if o.filters.is_none() || o.bars.is_empty() {
        return Err(format!("replay needs --filters and --bars\n{USAGE}"));
    }
    if o.instrument && o.jsonl.is_none() {
        return Err("--instrument writes to --jsonl; give one".into());
    }
    if o.massive && o.seconds {
        return Err("Massive minute-aggregate files are minute bars; drop --seconds".into());
    }
    Ok(o)
}

/// Prints signals, forwards everything to an optional JSONL sink, counts list changes.
struct Console<W: Write> {
    jsonl: Option<JsonlSink<W>>,
    quiet: bool,
    signals: u64,
    errors: Vec<String>,
}

impl<W: Write> Sink for Console<W> {
    fn signal(&mut self, s: &Signal) {
        self.signals += 1;
        if !self.quiet {
            let cols: Vec<String> =
                s.columns.iter().map(|(l, v)| format!("{l}={}", v.map_or("?".to_string(), |x| format!("{x:.2}")))).collect();
            println!(
                "{}  {:<5} {:<24} {:<6} {:>10}  {}",
                s.at.ny().format("%Y-%m-%d %H:%M:%S"),
                side(s.side),
                s.filter,
                s.symbol,
                s.last.map_or("?".to_string(), |x| format!("{x:.2}")),
                cols.join(" ")
            );
        }
        if let Some(j) = self.jsonl.as_mut() {
            j.signal(s);
        }
    }
    fn evaluation(&mut self, e: &tp_scan_engine::engine::Evaluation<'_>) {
        if let Some(j) = self.jsonl.as_mut() {
            j.evaluation(e);
        }
    }
    fn wants_evaluations(&self) -> bool {
        self.jsonl.as_ref().is_some_and(|j| j.wants_evaluations())
    }
    fn list_change(&mut self, c: &ListChange<'_>) {
        if let Some(j) = self.jsonl.as_mut() {
            j.list_change(c);
        }
    }
    fn error(&mut self, symbol: &str, e: &EngineError) {
        if self.errors.len() < 20 {
            self.errors.push(format!("{symbol}: {e}"));
        }
        if let Some(j) = self.jsonl.as_mut() {
            j.error(symbol, e);
        }
    }
}

fn open_bars(path: &Path, massive: bool, seconds: bool) -> Result<ReplaySource, String> {
    let ctx = |e: String| format!("{}: {e}", path.display());
    let file = File::open(path).map_err(|e| ctx(e.to_string()))?;
    let gz = path.extension().is_some_and(|x| x == "gz");
    let reader: Box<dyn std::io::BufRead> = if gz {
        Box::new(std::io::BufReader::new(flate2::read::GzDecoder::new(file)))
    } else {
        Box::new(std::io::BufReader::new(file))
    };
    if massive {
        tp_marketdata::flatfile::read_minute_aggs(reader).map_err(|e| ctx(e.to_string()))
    } else {
        let mut text = String::new();
        let mut reader = reader;
        reader.read_to_string(&mut text).map_err(|e| ctx(e.to_string()))?;
        let res = if seconds { Resolution::Second } else { Resolution::Minute };
        ReplaySource::from_csv(&text, res).map_err(|e| ctx(e.to_string()))
    }
}

fn replay(o: &ReplayOpts) -> Result<(), String> {
    let program = load_dir(o.filters.as_ref().expect("checked")).map_err(|e| e.to_string())?;
    let mut engine = Engine::new(&program).map_err(|e| e.to_string())?;

    if let Some(p) = &o.daily {
        let daily = files::read_daily(p)?;
        eprintln!("daily history: {} symbols", daily.len());
        engine.set_daily_provider(files::provider(daily));
    }
    if let Some(p) = &o.float {
        let floats = files::read_float(p)?;
        eprintln!("float: {} symbols", floats.len());
        for (sym, shares) in floats {
            engine.set_reference(&sym, tp_scan_engine::state::Reference { float_shares: Some(shares) });
        }
    }

    let jsonl = match &o.jsonl {
        Some(p) => Some(JsonlSink::new(BufWriter::new(File::create(p).map_err(|e| format!("{}: {e}", p.display()))?), o.instrument)),
        None => None,
    };
    let mut sink = Console { jsonl, quiet: o.quiet, signals: 0, errors: vec![] };
    let started = std::time::Instant::now();
    for path in &o.bars {
        let mut src = open_bars(path, o.massive, o.seconds)?;
        eprintln!("replaying {}", path.display());
        engine.run(&mut src, &mut sink);
    }
    let elapsed = started.elapsed();

    let st = engine.stats().clone();
    println!(
        "\n{} events, {} evaluations ({} unknown, {} out of hours), {} signals, {} list changes, {} errors  [{:.2}s]",
        st.events, st.evaluations, st.unknown, st.out_of_hours, st.signals, st.list_changes, st.errors, elapsed.as_secs_f64()
    );
    for e in &sink.errors {
        println!("  error: {e}");
    }

    for f in engine.compiled().filters().iter().filter(|f| f.kind == Kind::List) {
        let rows = engine.list(&f.name).expect("a list");
        if rows.is_empty() {
            continue;
        }
        println!("\n{} ({})", f.name, rows.len());
        for r in rows.iter().take(25) {
            let cols: Vec<String> =
                r.columns.iter().map(|(l, v)| format!("{l}={}", v.map_or("?".to_string(), |x| format!("{x:.2}")))).collect();
            println!("  {:<6} {:>10}  since {}  {}", r.symbol, r.last.map_or("?".into(), |x| format!("{x:.2}")), r.since.ny().format("%H:%M"), cols.join(" "));
        }
    }

    for (f, s) in &o.explain {
        match engine.explain(f, s) {
            Some(ex) => println!("\n{ex}"),
            None => println!("\nexplain: no filter `{f}` or no data for `{s}`"),
        }
    }

    if let Some(j) = sink.jsonl.take() {
        let lines = j.lines();
        j.finish().map_err(|e| format!("writing jsonl: {e}"))?;
        eprintln!("wrote {lines} json lines");
    }
    Ok(())
}
