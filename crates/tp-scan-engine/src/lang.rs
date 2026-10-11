//! The filter language: text in, [`Program`] out.
//!
//! Designed against the exported Primus filters (`spec/primus/`), which are its conformance set:
//! the language is done when it expresses all of them. It keeps their shape (boolean
//! combinations of numeric comparisons, named fields, `if`) and drops the noise (venue and
//! extended-hours flags on every call).
//!
//! ```text
//! # A comment.
//! field ATR_POS = (last - low(5m)) / atr * 100
//! condition liquid = day_volume >= 250_000 and last >= 2
//!
//! trigger M_Spike_Up {
//!     side long
//!     screen left
//!     hours rth          # rth (default) | pre | extended | any
//!     cooldown 1m        # Primus MaxTriggersCount
//!     show ATR_POS
//!     when liquid
//!      and new_high(15m)
//! }
//!
//! list Movers_Up_Alist {
//!     side long
//!     rank RNK_UP        # members sorted by this, highest first
//!     show RNK_UP, PM_RangePos
//!     when ...
//! }
//! ```
//!
//! ## Expressions
//!
//! Precedence, loosest first: `or`, `and`, `not`, comparison (`< <= > >=`), `+ -`, `* /`, unary
//! `-`. There is no `=`: equality on a computed price is a trap (one Contra filter is dead from
//! exactly that). Keywords may be written `and`/`AND`. Numbers take `_` separators and a bare
//! leading `.`; `9:30` is a time of day in minutes (570), for comparing with `time`.
//!
//! **Windows are rolling.** `high(5m)` is the trailing five minutes, `high(5m, 1)` the five before
//! those (Primus `P1`), `high(30m, clock)` the clock period in progress, which has to be asked for
//! by name because of the boundary trap. See [`crate::state::Win`].
//!
//! Every builtin is listed in [`BARE`] and [`CALLS`]; a name that is neither is a user field or
//! condition, defined anywhere in the program (order doesn't matter).

use crate::compile::{FilterDef, Hours, Kind, Program, Side};
use crate::expr::{if_, CmpOp, Cond, Num};
use crate::measure::{CandleInd, Event, Measure, Venue};
use crate::state::{Align, Win};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Builtin measures and conditions written as a bare name.
pub const BARE: &[&str] = &[
    "last", "day_volume", "day_volume_ext", "day_high", "day_low", "open", "prev_close", "prev_high", "prev_low",
    "prev_month_high", "prev_month_low", "atr", "adv", "vwap", "vwap_ext", "pm_high", "pm_low", "pm_volume",
    "time", "float", "ind_bid", "ind_ask", "clear_price", "near_price", "halted",
];

/// Builtin functions, with their argument shapes.
pub const CALLS: &[(&str, &str)] = &[
    ("high", "high(5m) | high(5m, 1) | high(30m, clock) | high(30m, clock, 1)"),
    ("low", "low(<window>)"),
    ("volume", "volume(<window>)"),
    ("open", "open(<window>): the window's first price; bare `open` is today's 09:30 open"),
    ("close", "close(<window>): the window's last price"),
    ("ema", "ema(5m, 20)"),
    ("sma", "sma(5m, 20)"),
    ("rsi", "rsi(5m, 14)"),
    ("bb_upper", "bb_upper(5m, 20, 2)"),
    ("bb_lower", "bb_lower(5m, 20, 2)"),
    ("daily_ema", "daily_ema(20)"),
    ("vwap_upper", "vwap_upper(2)"),
    ("vwap_lower", "vwap_lower(2)"),
    ("imb_buy", "imb_buy(nyse | nasdaq | any)"),
    ("imb_sell", "imb_sell(nyse | nasdaq | any)"),
    ("new_high", "new_high(15m)"),
    ("new_low", "new_low(15m)"),
    ("if", "if(condition, number, number)"),
    ("max", "max(a, b)"),
    ("min", "min(a, b)"),
    ("abs", "abs(a)"),
];

const ITEM_WORDS: &[&str] = &["field", "condition", "trigger", "list"];
const SETTING_WORDS: &[&str] = &["side", "hours", "cooldown", "screen", "show", "rank", "when"];
const LOGIC_WORDS: &[&str] = &["and", "or", "not"];

/// A position in a source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangError {
    pub file: Arc<str>,
    pub pos: Pos,
    pub msg: String,
}

impl fmt::Display for LangError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}: {}", self.file, self.pos.line, self.pos.col, self.msg)
    }
}

impl std::error::Error for LangError {}

// ---------------------------------------------------------------------------------------------
// Lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    /// Milliseconds.
    Dur(i64),
    /// Minutes since midnight.
    Clock(f64),
    Sym(&'static str),
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Ident(s) => write!(f, "`{s}`"),
            Tok::Num(x) => write!(f, "`{x}`"),
            Tok::Dur(ms) => write!(f, "duration {ms}ms"),
            Tok::Clock(m) => write!(f, "time {}:{:02}", (*m as i64) / 60, (*m as i64) % 60),
            Tok::Sym(s) => write!(f, "`{s}`"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

struct Lexed {
    toks: Vec<(Tok, Pos)>,
}

fn lex(file: &Arc<str>, src: &str) -> Result<Lexed, LangError> {
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut col) = (0usize, 1usize, 1usize);
    let mut toks = vec![];
    let err = |pos: Pos, msg: String| LangError { file: file.clone(), pos, msg };
    while i < chars.len() {
        let c = chars[i];
        let pos = Pos { line, col };
        let adv = |i: &mut usize, col: &mut usize, n: usize| {
            *i += n;
            *col += n;
        };
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
        } else if c.is_whitespace() {
            adv(&mut i, &mut col, 1);
        } else if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            col += i - s;
            toks.push((Tok::Ident(chars[s..i].iter().collect()), pos));
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let s = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[s..i].iter().filter(|&&ch| ch != '_').collect();
            // 9:30 -> a time of day.
            if chars.get(i) == Some(&':') && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                let h: u32 = text.parse().map_err(|_| err(pos, format!("bad hour in time `{text}:`")))?;
                let ms = i + 1;
                let mut me = ms;
                while me < chars.len() && chars[me].is_ascii_digit() {
                    me += 1;
                }
                let mt: String = chars[ms..me].iter().collect();
                let m: u32 = mt.parse().map_err(|_| err(pos, "bad minutes in time".into()))?;
                if h > 23 || m > 59 || mt.len() != 2 {
                    return Err(err(pos, format!("`{h}:{mt}` is not a time of day (use H:MM, 24-hour)")));
                }
                col += me - s;
                i = me;
                toks.push((Tok::Clock((h * 60 + m) as f64), pos));
                continue;
            }
            let x: f64 = text.parse().map_err(|_| err(pos, format!("`{text}` is not a number")))?;
            // 5m, 30s, 8h -> a duration, unless the letter starts a longer word.
            let unit = chars.get(i).copied();
            let after = chars.get(i + 1).copied();
            let is_word = |ch: Option<char>| ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
            if let Some(u @ ('s' | 'm' | 'h')) = unit {
                if !is_word(after) {
                    let per = match u {
                        's' => 1_000.0,
                        'm' => 60_000.0,
                        _ => 3_600_000.0,
                    };
                    let ms = x * per;
                    if ms.fract() != 0.0 || ms <= 0.0 {
                        return Err(err(pos, format!("`{text}{u}` is not a whole positive number of milliseconds")));
                    }
                    i += 1;
                    col += i - s;
                    toks.push((Tok::Dur(ms as i64), pos));
                    continue;
                }
            }
            if is_word(unit) {
                return Err(err(pos, format!("`{}` : a number runs into a word", chars[s..=i].iter().collect::<String>())));
            }
            col += i - s;
            toks.push((Tok::Num(x), pos));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let sym: &'static str = match two.as_str() {
                ">=" => ">=",
                "<=" => "<=",
                "==" | "!=" | "<>" => {
                    return Err(err(pos, format!("`{two}`: there is no equality test (float equality never holds reliably); use <= and >=")))
                }
                _ => match c {
                    '>' => ">",
                    '<' => "<",
                    '+' => "+",
                    '-' => "-",
                    '*' => "*",
                    '/' => "/",
                    '(' => "(",
                    ')' => ")",
                    ',' => ",",
                    '{' => "{",
                    '}' => "}",
                    '=' => "=",
                    _ => return Err(err(pos, format!("unexpected character `{c}`"))),
                },
            };
            adv(&mut i, &mut col, sym.len());
            toks.push((Tok::Sym(sym), pos));
        }
    }
    toks.push((Tok::Eof, Pos { line, col }));
    Ok(Lexed { toks })
}

// ---------------------------------------------------------------------------------------------
// Untyped syntax tree

#[derive(Clone, Debug)]
enum E {
    Num(f64),
    Clock(f64),
    Dur(i64),
    Name(String),
    Call(String, Vec<Node>),
    Neg(Box<Node>),
    Bin(char, Box<Node>, Box<Node>),
    Cmp(CmpOp, Box<Node>, Box<Node>),
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
}

#[derive(Clone, Debug)]
struct Node {
    e: E,
    pos: Pos,
}

enum Item {
    Field { name: String, pos: Pos, body: Node },
    Condition { name: String, pos: Pos, body: Node },
    Filter(FilterItem),
}

struct FilterItem {
    name: String,
    pos: Pos,
    kind: Kind,
    side: Side,
    hours: Hours,
    cooldown_ms: Option<(i64, Pos)>,
    screen: Option<String>,
    show: Vec<(String, Pos)>,
    rank: Option<Node>,
    when: Node,
}

struct Parser<'a> {
    file: &'a Arc<str>,
    toks: Vec<(Tok, Pos)>,
    i: usize,
}

fn is_kw(t: &Tok, w: &str) -> bool {
    matches!(t, Tok::Ident(s) if s == w || (LOGIC_WORDS.contains(&w) && s.eq_ignore_ascii_case(w)))
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].0
    }
    fn pos(&self) -> Pos {
        self.toks[self.i].1
    }
    fn next(&mut self) -> (Tok, Pos) {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn err<T>(&self, pos: Pos, msg: impl Into<String>) -> Result<T, LangError> {
        Err(LangError { file: self.file.clone(), pos, msg: msg.into() })
    }
    fn expect_sym(&mut self, s: &str) -> Result<Pos, LangError> {
        let (t, p) = self.next();
        if t == Tok::Sym(match s {
            "(" => "(",
            ")" => ")",
            "{" => "{",
            "}" => "}",
            "=" => "=",
            "," => ",",
            _ => unreachable!("expect_sym called with {s}"),
        }) {
            Ok(p)
        } else {
            self.err(p, format!("expected `{s}`, found {t}"))
        }
    }
    fn ident(&mut self, what: &str) -> Result<(String, Pos), LangError> {
        match self.next() {
            (Tok::Ident(s), p) => Ok((s, p)),
            (t, p) => self.err(p, format!("expected {what}, found {t}")),
        }
    }

    fn items(&mut self) -> Result<Vec<Item>, LangError> {
        let mut out = vec![];
        while *self.peek() != Tok::Eof {
            let (w, p) = self.ident("`field`, `condition`, `trigger` or `list`")?;
            match w.as_str() {
                "field" | "condition" => {
                    let (name, pos) = self.ident("a name")?;
                    self.expect_sym("=")?;
                    let body = self.expr()?;
                    out.push(if w == "field" { Item::Field { name, pos, body } } else { Item::Condition { name, pos, body } });
                }
                "trigger" | "list" => {
                    let kind = if w == "trigger" { Kind::Trigger } else { Kind::List };
                    out.push(Item::Filter(self.filter(kind)?));
                }
                _ => return self.err(p, format!("expected `field`, `condition`, `trigger` or `list`, found `{w}`")),
            }
        }
        Ok(out)
    }

    fn filter(&mut self, kind: Kind) -> Result<FilterItem, LangError> {
        let (name, pos) = self.ident("a filter name")?;
        self.expect_sym("{")?;
        let mut f = FilterItem {
            name,
            pos,
            kind,
            side: Side::Neutral,
            hours: Hours::Rth,
            cooldown_ms: None,
            screen: None,
            show: vec![],
            rank: None,
            when: Node { e: E::Num(0.0), pos },
        };
        let mut seen: Vec<String> = vec![];
        loop {
            let (w, p) = self.ident("a setting (side, hours, cooldown, screen, show, rank) or `when`")?;
            if seen.contains(&w) {
                return self.err(p, format!("`{w}` is set twice"));
            }
            seen.push(w.clone());
            match w.as_str() {
                "side" => {
                    let (v, vp) = self.ident("long, short or neutral")?;
                    f.side = match v.as_str() {
                        "long" => Side::Long,
                        "short" => Side::Short,
                        "neutral" => Side::Neutral,
                        _ => return self.err(vp, format!("side is long, short or neutral, not `{v}`")),
                    };
                }
                "hours" => {
                    let (v, vp) = self.ident("rth, pre, extended or any")?;
                    f.hours = match v.as_str() {
                        "rth" => Hours::Rth,
                        "pre" => Hours::Pre,
                        "extended" => Hours::Extended,
                        "any" => Hours::Any,
                        _ => return self.err(vp, format!("hours is rth, pre, extended or any, not `{v}`")),
                    };
                }
                "cooldown" => match self.next() {
                    (Tok::Dur(ms), dp) => f.cooldown_ms = Some((ms, dp)),
                    (t, dp) => return self.err(dp, format!("cooldown takes a duration like 5m, found {t}")),
                },
                "screen" => f.screen = Some(self.ident("a screen name")?.0),
                "show" => {
                    loop {
                        f.show.push(self.ident("a field name")?);
                        if *self.peek() == Tok::Sym(",") {
                            self.next();
                        } else {
                            break;
                        }
                    }
                }
                "rank" => f.rank = Some(self.expr()?),
                "when" => {
                    f.when = self.expr()?;
                    if let Tok::Ident(w) = self.peek() {
                        if ITEM_WORDS.contains(&w.as_str()) {
                            return self.err(self.pos(), format!("`{w}` starts a new item; is the filter's closing `}}` missing?"));
                        }
                    }
                    self.expect_sym("}")?;
                    return Ok(f);
                }
                _ => return self.err(p, format!("unknown setting `{w}` (side, hours, cooldown, screen, show, rank, when)")),
            }
        }
    }

    fn expr(&mut self) -> Result<Node, LangError> {
        self.or()
    }

    fn or(&mut self) -> Result<Node, LangError> {
        let pos = self.pos();
        let mut parts = vec![self.and()?];
        while is_kw(self.peek(), "or") {
            self.next();
            parts.push(self.and()?);
        }
        Ok(if parts.len() == 1 { parts.pop().expect("one") } else { Node { e: E::Or(parts), pos } })
    }

    fn and(&mut self) -> Result<Node, LangError> {
        let pos = self.pos();
        let mut parts = vec![self.not()?];
        while is_kw(self.peek(), "and") {
            self.next();
            parts.push(self.not()?);
        }
        Ok(if parts.len() == 1 { parts.pop().expect("one") } else { Node { e: E::And(parts), pos } })
    }

    fn not(&mut self) -> Result<Node, LangError> {
        if is_kw(self.peek(), "not") {
            let (_, pos) = self.next();
            let inner = self.not()?;
            return Ok(Node { e: E::Not(Box::new(inner)), pos });
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Node, LangError> {
        let a = self.sum()?;
        let op = match self.peek() {
            Tok::Sym(">=") => CmpOp::Ge,
            Tok::Sym("<=") => CmpOp::Le,
            Tok::Sym(">") => CmpOp::Gt,
            Tok::Sym("<") => CmpOp::Lt,
            Tok::Sym("=") => return self.err(self.pos(), "there is no `=` test (float equality never holds reliably); use <= and >="),
            _ => return Ok(a),
        };
        let (_, pos) = self.next();
        let b = self.sum()?;
        if matches!(self.peek(), Tok::Sym(">=" | "<=" | ">" | "<")) {
            return self.err(self.pos(), "comparisons don't chain: write `a <= b and b <= c`");
        }
        Ok(Node { e: E::Cmp(op, Box::new(a), Box::new(b)), pos })
    }

    fn sum(&mut self) -> Result<Node, LangError> {
        let mut a = self.prod()?;
        while let Tok::Sym(s @ ("+" | "-")) = *self.peek() {
            let (_, pos) = self.next();
            let b = self.prod()?;
            a = Node { e: E::Bin(s.chars().next().expect("one char"), Box::new(a), Box::new(b)), pos };
        }
        Ok(a)
    }

    fn prod(&mut self) -> Result<Node, LangError> {
        let mut a = self.unary()?;
        while let Tok::Sym(s @ ("*" | "/")) = *self.peek() {
            let (_, pos) = self.next();
            let b = self.unary()?;
            a = Node { e: E::Bin(s.chars().next().expect("one char"), Box::new(a), Box::new(b)), pos };
        }
        Ok(a)
    }

    fn unary(&mut self) -> Result<Node, LangError> {
        if *self.peek() == Tok::Sym("-") {
            let (_, pos) = self.next();
            let inner = self.unary()?;
            return Ok(Node { e: E::Neg(Box::new(inner)), pos });
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, LangError> {
        let (t, pos) = self.next();
        let e = match t {
            Tok::Num(x) => E::Num(x),
            Tok::Clock(m) => E::Clock(m),
            Tok::Dur(ms) => E::Dur(ms),
            Tok::Sym("(") => {
                let inner = self.expr()?;
                self.expect_sym(")")?;
                return Ok(inner);
            }
            Tok::Ident(name) => {
                if ITEM_WORDS.contains(&name.as_str()) || (SETTING_WORDS.contains(&name.as_str()) && name != "when") {
                    return self.err(pos, format!("`{name}` starts a new item or setting; is a closing `}}` or an operand missing before it?"));
                }
                if LOGIC_WORDS.iter().any(|w| name.eq_ignore_ascii_case(w)) || name == "when" {
                    return self.err(pos, format!("expected a value, found `{name}`"));
                }
                if *self.peek() == Tok::Sym("(") {
                    self.next();
                    let mut args = vec![];
                    if *self.peek() != Tok::Sym(")") {
                        loop {
                            args.push(self.expr()?);
                            if *self.peek() == Tok::Sym(",") {
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect_sym(")")?;
                    E::Call(name, args)
                } else {
                    E::Name(name)
                }
            }
            t => return self.err(pos, format!("expected a value, found {t}")),
        };
        Ok(Node { e, pos })
    }
}

// ---------------------------------------------------------------------------------------------
// Typing: untyped tree -> Num / Cond

#[derive(Clone, Copy, PartialEq, Eq)]
enum NameKind {
    Field,
    Condition,
    Filter,
}

struct Typer<'a> {
    names: &'a HashMap<String, (NameKind, Arc<str>, Pos)>,
    file: Arc<str>,
}

impl Typer<'_> {
    fn err<T>(&self, pos: Pos, msg: impl Into<String>) -> Result<T, LangError> {
        Err(LangError { file: self.file.clone(), pos, msg: msg.into() })
    }

    fn describe(n: &Node) -> &'static str {
        match n.e {
            E::Cmp(..) | E::And(_) | E::Or(_) | E::Not(_) => "a condition",
            E::Dur(_) => "a duration",
            _ => "a number",
        }
    }

    fn num(&self, n: &Node) -> Result<Num, LangError> {
        match &n.e {
            E::Num(x) => Ok(Num::Const(*x)),
            E::Clock(m) => Ok(Num::Const(*m)),
            E::Dur(_) => self.err(n.pos, "a duration only goes inside a function, like high(5m)"),
            E::Neg(a) => Ok(-self.num(a)?),
            E::Bin(op, a, b) => {
                let (a, b) = (self.num(a)?, self.num(b)?);
                Ok(match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    _ => a / b,
                })
            }
            E::Name(s) => {
                if let Some(m) = bare_measure(s) {
                    return Ok(Num::M(m));
                }
                if s == "halted" {
                    return self.err(n.pos, "`halted` is a condition, not a number");
                }
                match self.names.get(s) {
                    Some((NameKind::Field, name, _)) => Ok(Num::Ref(name.clone())),
                    Some((NameKind::Condition, ..)) => self.err(n.pos, format!("`{s}` is a condition, not a number")),
                    Some((NameKind::Filter, ..)) => self.err(n.pos, format!("`{s}` is a filter; filters can't be referenced")),
                    None => self.err(n.pos, format!("`{s}` is not defined (and isn't a builtin)")),
                }
            }
            E::Call(f, args) => self.num_call(n, f, args),
            _ => self.err(n.pos, format!("expected a number, found {}", Self::describe(n))),
        }
    }

    fn cond(&self, n: &Node) -> Result<Cond, LangError> {
        match &n.e {
            E::Cmp(op, a, b) => {
                let (a, b) = (self.num(a)?, self.num(b)?);
                self.check_float_units(n.pos, &a, &b)?;
                Ok(Cond::Cmp(Box::new(a), *op, Box::new(b)))
            }
            E::And(parts) => {
                self.check_satisfiable(parts)?;
                Ok(Cond::All(parts.iter().map(|p| self.cond(p)).collect::<Result<_, _>>()?))
            }
            E::Or(parts) => Ok(Cond::Any(parts.iter().map(|p| self.cond(p)).collect::<Result<_, _>>()?)),
            E::Not(a) => Ok(Cond::not(self.cond(a)?)),
            E::Name(s) if s == "halted" => Ok(Cond::Event(Event::Halted)),
            E::Name(s) => match self.names.get(s) {
                Some((NameKind::Condition, name, _)) => Ok(Cond::Ref(name.clone())),
                _ => self.err(n.pos, format!("expected a condition, found `{s}` (a number); compare it with something")),
            },
            E::Call(f, args) if f == "new_high" || f == "new_low" => {
                let span = self.one_dur(n, f, args)?;
                Ok(Cond::Event(if f == "new_high" { Event::NewHigh { span_ms: span } } else { Event::NewLow { span_ms: span } }))
            }
            _ => self.err(n.pos, format!("expected a condition, found {}; compare it with something", Self::describe(n))),
        }
    }

    /// `float` is in shares. Primus wrote it in millions (`FloatShares <= 20`), and its
    /// `Float <= 10000000` meant ten trillion shares and filtered nothing for months
    /// (PM_LowFloat_Coil_Up). Both mistakes are refused here.
    fn check_float_units(&self, pos: Pos, a: &Num, b: &Num) -> Result<(), LangError> {
        let c = match (a, b) {
            (Num::M(Measure::Float), x) | (x, Num::M(Measure::Float)) => match const_value(x) {
                Some(c) => c,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        if c < 10_000.0 {
            return self.err(pos, format!("`float` is in shares: {c} means {c} shares. For Primus's millions write e.g. 20_000_000"));
        }
        if c >= 100_000_000_000.0 {
            return self.err(pos, format!("`float` against {c} shares restricts nothing (more than any float). Was this Primus millions written as shares?"));
        }
        Ok(())
    }

    /// Bounds on the same quantity, ANDed together, that no value can meet: the Momo VWAP
    /// Divergence defect (`Last - VWAP <= -0.75 AND Last - VWAP >= 0.75`), which ran "healthy" and
    /// never fired. Only constant bounds directly inside one `and` (brackets flattened) are seen.
    fn check_satisfiable(&self, parts: &[Node]) -> Result<(), LangError> {
        fn flatten<'n>(parts: &'n [Node], out: &mut Vec<&'n Node>) {
            for p in parts {
                match &p.e {
                    E::And(inner) => flatten(inner, out),
                    _ => out.push(p),
                }
            }
        }
        /// (bound, strict, where)
        type Bound = Option<(f64, bool, Pos)>;
        let mut flat = vec![];
        flatten(parts, &mut flat);
        let mut seen: HashMap<String, (Bound, Bound)> = HashMap::new();
        for p in flat {
            let E::Cmp(op, a, b) = &p.e else { continue };
            let (Ok(a), Ok(b)) = (self.num(a), self.num(b)) else { continue };
            let (q, op, c) = match (const_value(&a), const_value(&b)) {
                (None, Some(c)) => (a, *op, c),
                (Some(c), None) => (b, op.flip(), c),
                _ => continue,
            };
            let key = q.to_string();
            let e = seen.entry(key.clone()).or_insert((None, None));
            let tighter = |cur: Bound, upper: bool, strict: bool| {
                cur.is_none_or(|(x, s, _)| if upper { c < x } else { c > x } || (c == x && strict && !s))
            };
            match op {
                CmpOp::Ge | CmpOp::Gt => {
                    let strict = op == CmpOp::Gt;
                    if tighter(e.0, false, strict) {
                        e.0 = Some((c, strict, p.pos));
                    }
                }
                CmpOp::Le | CmpOp::Lt => {
                    let strict = op == CmpOp::Lt;
                    if tighter(e.1, true, strict) {
                        e.1 = Some((c, strict, p.pos));
                    }
                }
            }
            if let (Some((lo, ls, lp)), Some((hi, hs, hp))) = (e.0, e.1) {
                if lo > hi || (lo == hi && (ls || hs)) {
                    let (first, second) = if lp.line <= hp.line { (lp, hp) } else { (hp, lp) };
                    return self.err(
                        second,
                        format!(
                            "this can never be true: `{key}` must be {} {lo} and {} {hi} (line {}), so the filter could never fire. Did you mean `or`, or is one bound's sign wrong?",
                            if ls { ">" } else { ">=" },
                            if hs { "<" } else { "<=" },
                            first.line
                        ),
                    );
                }
            }
        }
        Ok(())
    }

    fn arity(&self, n: &Node, f: &str, args: &[Node], want: usize) -> Result<(), LangError> {
        if args.len() != want {
            let shape = CALLS.iter().find(|c| c.0 == f).map_or("", |c| c.1);
            return self.err(n.pos, format!("`{f}` takes {want} argument(s): {shape}"));
        }
        Ok(())
    }

    fn one_dur(&self, n: &Node, f: &str, args: &[Node]) -> Result<i64, LangError> {
        self.arity(n, f, args, 1)?;
        self.dur(&args[0])
    }

    fn dur(&self, a: &Node) -> Result<i64, LangError> {
        match a.e {
            E::Dur(ms) => Ok(ms),
            _ => self.err(a.pos, format!("expected a duration like 5m, found {}", Self::describe(a))),
        }
    }

    fn count(&self, a: &Node, what: &str) -> Result<u16, LangError> {
        match a.e {
            E::Num(x) if x.fract() == 0.0 && x >= 1.0 && x <= u16::MAX as f64 => Ok(x as u16),
            _ => self.err(a.pos, format!("{what} must be a whole number ≥ 1")),
        }
    }

    fn centi(&self, a: &Node) -> Result<u16, LangError> {
        match a.e {
            E::Num(x) if x >= 0.0 && (x * 100.0).fract().abs() < 1e-9 && x * 100.0 <= u16::MAX as f64 => Ok((x * 100.0).round() as u16),
            _ => self.err(a.pos, "expected a constant with at most two decimals, like 2 or 2.5"),
        }
    }

    fn candle_tf(&self, a: &Node) -> Result<u16, LangError> {
        let ms = self.dur(a)?;
        if ms != crate::state::CANDLE_MS {
            return self.err(a.pos, "only 5-minute candles are kept (5m)");
        }
        Ok((ms / 60_000) as u16)
    }

    fn window(&self, n: &Node, f: &str, args: &[Node]) -> Result<Win, LangError> {
        if args.is_empty() || args.len() > 3 {
            return self.err(n.pos, format!("`{f}` takes a window: high(5m) | high(5m, 1) | high(30m, clock) | high(30m, clock, 1)"));
        }
        let span = self.dur(&args[0])?;
        let (mut clock, mut back) = (false, 0i64);
        for a in &args[1..] {
            match &a.e {
                E::Name(s) if s == "clock" && !clock => clock = true,
                E::Num(x) if x.fract() == 0.0 && *x >= 1.0 && back == 0 => back = *x as i64,
                _ => return self.err(a.pos, format!("after the span, `{f}` takes `clock` and/or a whole number of windows back (P1 = 1)")),
            }
        }
        let w = Win { span_ms: span, offset_ms: back * span, align: if clock { Align::Clock } else { Align::Rolling } };
        if clock && (3_600_000 % span != 0) {
            return self.err(args[0].pos, "a clock window's span must divide an hour");
        }
        Ok(w)
    }

    fn num_call(&self, n: &Node, f: &str, args: &[Node]) -> Result<Num, LangError> {
        let candle = |ind: CandleInd, tf: u16| Ok(Num::M(Measure::Candle { tf_min: tf, ind }));
        match f {
            "high" => Ok(Num::M(Measure::High(self.window(n, f, args)?))),
            "low" => Ok(Num::M(Measure::Low(self.window(n, f, args)?))),
            "volume" => Ok(Num::M(Measure::Volume(self.window(n, f, args)?))),
            "open" => Ok(Num::M(Measure::Open(self.window(n, f, args)?))),
            "close" => Ok(Num::M(Measure::Close(self.window(n, f, args)?))),
            "ema" | "sma" | "rsi" => {
                self.arity(n, f, args, 2)?;
                let tf = self.candle_tf(&args[0])?;
                let len = self.count(&args[1], "the period")?;
                candle(
                    match f {
                        "ema" => CandleInd::Ema(len),
                        "sma" => CandleInd::Sma(len),
                        _ => CandleInd::Rsi(len),
                    },
                    tf,
                )
            }
            "bb_upper" | "bb_lower" => {
                self.arity(n, f, args, 3)?;
                let tf = self.candle_tf(&args[0])?;
                let len = self.count(&args[1], "the period")?;
                let k_centi = self.centi(&args[2])?;
                candle(if f == "bb_upper" { CandleInd::BbUpper { len, k_centi } } else { CandleInd::BbLower { len, k_centi } }, tf)
            }
            "daily_ema" => {
                self.arity(n, f, args, 1)?;
                Ok(Num::M(Measure::DailyEma(self.count(&args[0], "the period")?)))
            }
            "vwap_upper" | "vwap_lower" => {
                self.arity(n, f, args, 1)?;
                Ok(Num::M(Measure::VwapBand { upper: f == "vwap_upper", k_centi: self.centi(&args[0])? }))
            }
            "imb_buy" | "imb_sell" => {
                self.arity(n, f, args, 1)?;
                let venue = match &args[0].e {
                    E::Name(s) if s == "nyse" => Venue::Nyse,
                    E::Name(s) if s == "nasdaq" => Venue::Nasdaq,
                    E::Name(s) if s == "any" => Venue::Any,
                    _ => return self.err(args[0].pos, "venue is nyse, nasdaq or any"),
                };
                Ok(Num::M(Measure::Imbalance { buy: f == "imb_buy", venue }))
            }
            "if" => {
                self.arity(n, f, args, 3)?;
                Ok(if_(self.cond(&args[0])?, self.num(&args[1])?, self.num(&args[2])?))
            }
            "max" | "min" => {
                self.arity(n, f, args, 2)?;
                let (a, b) = (self.num(&args[0])?, self.num(&args[1])?);
                Ok(if f == "max" { a.max(b) } else { a.min(b) })
            }
            "abs" => {
                self.arity(n, f, args, 1)?;
                Ok(self.num(&args[0])?.abs())
            }
            "new_high" | "new_low" => self.err(n.pos, format!("`{f}` is a condition, not a number")),
            _ => self.err(n.pos, format!("`{f}` is not a function")),
        }
    }
}

/// A number known when the program loads: a literal, or a negated one.
fn const_value(n: &Num) -> Option<f64> {
    match n {
        Num::Const(c) => Some(*c),
        Num::Neg(x) => const_value(x).map(|c| -c),
        _ => None,
    }
}

fn bare_measure(s: &str) -> Option<Measure> {
    Some(match s {
        "last" => Measure::Last,
        "day_volume" => Measure::DayVolume { extended: false },
        "day_volume_ext" => Measure::DayVolume { extended: true },
        "day_high" => Measure::DayHigh,
        "day_low" => Measure::DayLow,
        "open" => Measure::SessionOpen,
        "prev_close" => Measure::PrevClose,
        "prev_high" => Measure::PrevHigh,
        "prev_low" => Measure::PrevLow,
        "prev_month_high" => Measure::PrevMonthHigh,
        "prev_month_low" => Measure::PrevMonthLow,
        "atr" => Measure::Atr,
        "adv" => Measure::AvgDailyVolume,
        "vwap" => Measure::Vwap,
        "vwap_ext" => Measure::VwapExt,
        "pm_high" => Measure::PremarketHigh,
        "pm_low" => Measure::PremarketLow,
        "pm_volume" => Measure::PremarketVolume,
        "time" => Measure::Time,
        "float" => Measure::Float,
        "ind_bid" => Measure::IndBid,
        "ind_ask" => Measure::IndAsk,
        "clear_price" => Measure::ClearPrice,
        "near_price" => Measure::NearPrice,
        _ => return None,
    })
}

fn reserved(s: &str) -> bool {
    BARE.contains(&s)
        || CALLS.iter().any(|c| c.0 == s)
        || ITEM_WORDS.contains(&s)
        || SETTING_WORDS.contains(&s)
        || LOGIC_WORDS.iter().any(|w| s.eq_ignore_ascii_case(w))
        || matches!(s, "clock" | "nyse" | "nasdaq" | "any")
}

/// Read every `*.scan` file in `dir` (not recursive), in name order, and parse them as one
/// program. I/O problems come back as a [`LangError`] naming the file, at line 0.
pub fn load_dir(dir: &std::path::Path) -> Result<Program, LangError> {
    let io = |file: &std::path::Path, e: std::io::Error| LangError { file: Arc::from(file.display().to_string()), pos: Pos::default(), msg: e.to_string() };
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| io(dir, e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "scan"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(LangError { file: Arc::from(dir.display().to_string()), pos: Pos::default(), msg: "no .scan files here".into() });
    }
    let mut texts = vec![];
    for p in &paths {
        texts.push((p.display().to_string(), std::fs::read_to_string(p).map_err(|e| io(p, e))?));
    }
    let refs: Vec<(&str, &str)> = texts.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    parse_sources(&refs)
}

/// Parse one source.
pub fn parse(src: &str) -> Result<Program, LangError> {
    parse_sources(&[("<input>", src)])
}

/// Parse several sources as one program. Names are global across them; errors name the source.
pub fn parse_sources(sources: &[(&str, &str)]) -> Result<Program, LangError> {
    let mut items: Vec<(Arc<str>, Item)> = vec![];
    for (file, src) in sources {
        let file: Arc<str> = Arc::from(*file);
        let lexed = lex(&file, src)?;
        let mut p = Parser { file: &file, toks: lexed.toks, i: 0 };
        for it in p.items()? {
            items.push((file.clone(), it));
        }
    }

    let mut names: HashMap<String, (NameKind, Arc<str>, Pos)> = HashMap::new();
    for (file, it) in &items {
        let (name, pos, kind) = match it {
            Item::Field { name, pos, .. } => (name, *pos, NameKind::Field),
            Item::Condition { name, pos, .. } => (name, *pos, NameKind::Condition),
            Item::Filter(f) => (&f.name, f.pos, NameKind::Filter),
        };
        let e = |msg: String| Err(LangError { file: file.clone(), pos, msg });
        if reserved(name) {
            return e(format!("`{name}` is a builtin name; pick another"));
        }
        if let Some((_, _, first)) = names.get(name) {
            return e(format!("`{name}` is already defined (line {})", first.line));
        }
        names.insert(name.clone(), (kind, Arc::from(name.as_str()), pos));
    }

    let mut program = Program::default();
    for (file, it) in &items {
        let ty = Typer { names: &names, file: file.clone() };
        match it {
            Item::Field { name, body, .. } => program = program.define_num(name, ty.num(body)?),
            Item::Condition { name, body, .. } => program = program.define_cond(name, ty.cond(body)?),
            Item::Filter(f) => {
                let mut def = FilterDef::new(&f.name, ty.cond(&f.when)?, 0);
                def.kind = f.kind;
                def.side = f.side;
                def.hours = f.hours;
                def.screen = f.screen.as_deref().map(Arc::from);
                if let Some((ms, p)) = f.cooldown_ms {
                    if f.kind == Kind::List {
                        return ty.err(p, "lists have no cooldown: a member stays listed while its condition holds");
                    }
                    def.cooldown_ms = ms;
                }
                if let Some(r) = &f.rank {
                    if f.kind == Kind::Trigger {
                        return ty.err(r.pos, "only lists are ranked");
                    }
                    def.rank = Some(ty.num(r)?);
                }
                for (col, p) in &f.show {
                    def.columns.push((Arc::from(col.as_str()), ty.num(&Node { e: E::Name(col.clone()), pos: *p })?));
                }
                program = program.filter(def);
            }
        }
    }
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Compiled, Ctx, Tri};
    use crate::state::tests::minutes;

    fn eval(src: &str, filter: usize) -> Tri {
        let p = parse(src).unwrap_or_else(|e| panic!("{e}"));
        let c = Compiled::new(&p).unwrap();
        let s = minutes(10); // last 18.5, high(5m) 19, low(5m) 14, 09:30..09:39
        let mut ctx = Ctx::new(&c, &s);
        c.eval(filter, &mut ctx)
    }

    fn error(src: &str) -> String {
        match parse(src) {
            Ok(_) => panic!("parsed: {src}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn a_whole_program_parses_and_evaluates() {
        let src = "
            # fields and conditions may come after their use
            trigger up {
                side long
                cooldown 1m
                show LIFT
                when liquid AND LIFT >= 4.5 and new_high(5m)
            }
            field LIFT = last - low(5m)
            condition liquid = day_volume >= 1_000 and last >= 2
        ";
        assert_eq!(eval(src, 0), Tri::True);
        let p = parse(src).unwrap();
        let f = &p.filters[0];
        assert_eq!((f.kind, f.side, f.hours, f.cooldown_ms), (Kind::Trigger, Side::Long, Hours::Rth, 60_000));
        assert_eq!(&*f.columns[0].0, "LIFT");
        assert_eq!(f.cond.to_string(), "liquid AND (LIFT >= 4.5) AND new_high(5m)");
    }

    #[test]
    fn precedence_and_windows() {
        let p = parse("field X = 1 + 2 * 3 - -4 / 2\nfield Y = high(5m, 1) - low(30m, clock) + volume(15m, clock, 2)").unwrap();
        assert_eq!(p.num_defs[0].1.to_string(), "((1 + (2 * 3)) - (-4 / 2))");
        assert_eq!(p.num_defs[1].1.to_string(), "((high(5m, 5m back) - low(30m clock)) + volume(15m clock, 30m back))");
        let c = parse("condition C = not a or b and c\ncondition a = 1 < 2\ncondition b = 1 < 2\ncondition c = 1 < 2").unwrap();
        assert_eq!(c.cond_defs[0].1.to_string(), "NOT (a) OR (b AND c)");
    }

    #[test]
    fn times_durations_and_builtins() {
        assert_eq!(eval("trigger t { when time >= 9:30 and time < 9:40 }", 0), Tri::True);
        let p = parse(
            "field A = ema(5m, 7) - sma(5m, 20) + rsi(5m, 14) + bb_upper(5m, 20, 2) + bb_lower(5m, 20, 2.5)
             field B = daily_ema(20) + vwap_upper(2) + vwap_lower(1.5) + imb_buy(nyse) + imb_sell(any)
             field C = if(time <= 570, 0.01, if(time <= 572, 0.022, 1)) + max(1, 2) + min(1, 2) + abs(-1) + .25
             condition D = halted and new_low(10m)
             list L { rank A  show A, B  when D }",
        )
        .unwrap();
        assert_eq!(p.num_defs[2].1.to_string(), "((((if(time <= 570, 0.01, if(time <= 572, 0.022, 1)) + max(1, 2)) + min(1, 2)) + abs(-1)) + 0.25)");
        assert_eq!(p.filters[0].kind, Kind::List);
        assert_eq!(p.filters[0].columns.len(), 2);
        Compiled::new(&p).unwrap();
    }

    #[test]
    fn errors_say_where_and_why() {
        assert_eq!(error("field X = last +"), "<input>:1:17: expected a value, found end of file");
        assert!(error("field X = nope").contains("`nope` is not defined"));
        assert!(error("condition C = last").contains("expected a condition, found `last`"));
        assert!(error("field X = last > 2").contains("expected a number, found a condition"));
        assert!(error("trigger t { when last = 2 }").contains("no `=` test"));
        assert!(error("trigger t { when last == 2 }").contains("no equality test"));
        assert!(error("trigger t { when 1 < 2 < 3 }").contains("don't chain"));
        assert!(error("field last = 1").contains("builtin name"));
        assert!(error("field X = 1\nfield X = 2").contains("already defined (line 1)"));
        assert!(error("trigger t { when ema(15m, 7) > 0 }").contains("only 5-minute candles"));
        assert!(error("trigger t { when high(7m, clock) > 0 }").contains("divide an hour"));
        assert!(error("list l { cooldown 1m when 1 < 2 }").contains("lists have no cooldown"));
        assert!(error("trigger t { rank last when 1 < 2 }").contains("only lists are ranked"));
        assert!(error("trigger t { side up when 1 < 2 }").contains("long, short or neutral"));
        assert!(error("trigger t { when 1 < 2").contains("expected `}`"));
        assert!(error("trigger t { when high(5) > 0 }").contains("expected a duration"));
        assert!(error("trigger t { when last > 5x }").contains("runs into a word"));
        assert!(error("trigger t { when time > 9:3 }").contains("not a time of day"));
        assert!(error("trigger t { when last > 2\nfield X = 1 }").contains("starts a new item"));
    }

    #[test]
    fn load_time_checks_refuse_filters_that_cannot_work() {
        // Momo VWAP Divergence: both bounds on one quantity, ANDed.
        let e = error("trigger t {\n when last - vwap <= -0.75\n  and last / vwap <= 0.97\n  and last - vwap >= 0.75 }");
        assert!(e.contains("<input>:4:") && e.contains("can never be true") && e.contains("(line 2)"), "{e}");
        // Brackets are flattened; a constant on the left is flipped; strictness counts.
        assert!(error("trigger t { when (last > 5 and day_volume > 1) and 5 > last }").contains("can never be true"));
        assert!(error("trigger t { when last >= 5 and last < 5 }").contains("can never be true"));
        // ... but touching inclusive bounds, OR, and different quantities are fine.
        parse("trigger t { when last >= 5 and last <= 5 }").unwrap();
        parse("trigger t { when last - vwap <= -0.75 or last - vwap >= 0.75 }").unwrap();
        parse("trigger t { when last - vwap <= -0.75 and last - vwap_ext >= 0.75 }").unwrap();
        // Float is in shares.
        assert!(error("trigger t { when float <= 20 }").contains("in shares"));
        assert!(error("trigger t { when float <= 10_000_000_000_000 }").contains("restricts nothing"));
        assert!(error("trigger t { when 10 >= float }").contains("in shares"));
        parse("trigger t { when float <= 20_000_000 }").unwrap();
    }

    #[test]
    fn multiple_sources_share_names_and_errors_name_the_file() {
        let p = parse_sources(&[("a.scan", "field X = last"), ("b.scan", "trigger t { when X > 1 }")]).unwrap();
        assert_eq!(p.filters.len(), 1);
        let e = parse_sources(&[("a.scan", "field X = last"), ("b.scan", "trigger t { when Y > 1 }")]).unwrap_err();
        assert_eq!(&*e.file, "b.scan");
    }
}
