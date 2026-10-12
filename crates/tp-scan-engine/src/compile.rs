//! Compiles a [`Program`] into a tree of closures, and explains any evaluation clause by clause.
//!
//! - **Closure tree, built once.** Each node becomes a boxed closure at compile time, so an
//!   evaluation never re-dispatches on node type. No JIT: the investigation puts the whole load at
//!   ~20% of one core.
//! - **Memoized once per symbol per cycle.** Every distinct measurement and every named definition
//!   gets one slot, shared by all filters. `atr` in forty filters is computed once.
//! - **Three-valued logic.** A comparison with an unknown side is [`Tri::Unknown`], never a
//!   silent false or a zero. `AND` is false if any part is false, else unknown if any part is
//!   unknown; `OR` is the dual. A filter fires only on `True`, but an explanation shows *which*
//!   clause was unknown.
//! - **Introspection built in.** In trace mode each node records its value, so "which clause was
//!   false" is answered by the same code that evaluated it. Trace mode turns off short-circuiting
//!   so every clause shows a value; expressions are pure, so the result can't change.
//! - Non-finite arithmetic (x/0, overflow, NaN) is unknown.

use crate::expr::{Attr, CmpOp, Cond, Num};
use crate::measure::Measure;
use crate::state::SymbolState;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

/// Kleene three-valued truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    pub fn from_bool(b: bool) -> Tri {
        if b {
            Tri::True
        } else {
            Tri::False
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Tri::True => "T",
            Tri::False => "F",
            Tri::Unknown => "?",
        }
    }
}

impl std::ops::Not for Tri {
    type Output = Tri;
    fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

/// What a filter produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Emits a signal each time the condition is true, subject to the cooldown (a Primus filter).
    Trigger,
    /// Maintains a list: a symbol is a member while the condition is true (a Primus static).
    List,
}

/// Which way a filter leans. Display only (Primus panels: green long, red short).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Long,
    Short,
    Neutral,
}

/// When a filter is evaluated, by the session phase of the newest bar. Outside its hours a
/// filter is not evaluated at all: triggers don't fire and list membership is left as it was.
///
/// This replaces the Primus `YES`/`NO` extended-hours flags as the way to keep an RTH filter
/// from firing after hours (Blowoff_Reversal fired every few minutes into 19:56 on 09-05).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hours {
    /// 09:30–16:00.
    Rth,
    /// 04:00–09:30.
    Pre,
    /// 04:00–20:00.
    Extended,
    /// Every bar, whatever its phase.
    Any,
}

impl Hours {
    pub fn admits(self, p: tp_marketdata::time::Phase) -> bool {
        use tp_marketdata::time::Phase;
        match self {
            Hours::Rth => p == Phase::Rth,
            Hours::Pre => p == Phase::Pre,
            Hours::Extended => p != Phase::Closed,
            Hours::Any => true,
        }
    }
}

/// A filter as written.
#[derive(Clone, Debug)]
pub struct FilterDef {
    pub name: Arc<str>,
    pub cond: Cond,
    /// Minimum time between two signals from this filter for one symbol (Primus
    /// `MaxTriggersCount`). 0 = every true evaluation fires. Ignored by lists.
    pub cooldown_ms: i64,
    pub kind: Kind,
    pub side: Side,
    pub hours: Hours,
    /// Which screen the filter's panel sits on. Display only.
    pub screen: Option<Arc<str>>,
    /// Values shown next to each signal or list member (Primus panel columns such as `ATR_POS`).
    pub columns: Vec<(Arc<str>, Num)>,
    /// Lists are sorted by this, highest first (Primus `RNK_UP`). Unknown ranks sort last.
    pub rank: Option<Num>,
    /// The basket the filter draws from, if any. Its rules are already ANDed into `cond`, in
    /// front; this is the name, for display.
    pub basket: Option<Arc<str>>,
}

/// A named basket of stocks: the reference-data and price-range rules a filter can require with
/// `basket NAME`. Kept in this structured form for the app's basket editor; the rules themselves
/// are compiled into each filter that uses the basket.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BasketDef {
    pub name: Arc<str>,
    /// Security types allowed (empty = any).
    pub types: Vec<String>,
    /// Listing exchanges allowed (empty = any).
    pub exchanges: Vec<String>,
    /// `price`, `float`, `adv` or `atr`, with an optional minimum and maximum.
    pub ranges: Vec<(String, Option<f64>, Option<f64>)>,
}

impl FilterDef {
    /// A trigger evaluated on every bar, with no columns.
    pub fn new(name: &str, cond: Cond, cooldown_ms: i64) -> Self {
        FilterDef {
            name: Arc::from(name),
            cond,
            cooldown_ms,
            kind: Kind::Trigger,
            side: Side::Neutral,
            hours: Hours::Any,
            screen: None,
            columns: vec![],
            rank: None,
            basket: None,
        }
    }
}

/// Everything the scanner runs: shared definitions plus filters.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub num_defs: Vec<(Arc<str>, Num)>,
    pub cond_defs: Vec<(Arc<str>, Cond)>,
    pub filters: Vec<FilterDef>,
    pub baskets: Vec<BasketDef>,
}

impl Program {
    /// Whether a filter reads premarket data: `pm_high`, `pm_low`, `pm_volume`, `vwap_ext` or the
    /// extended-session volume, directly or through the fields and conditions it uses; or it runs
    /// in the premarket only (`hours pre`). The app's charts keep the premarket on screen after the
    /// open only for a symbol a filter like this put there.
    pub fn uses_premarket(&self, filter: &str) -> bool {
        let Some(f) = self.filters.iter().find(|f| &*f.name == filter) else { return false };
        if f.hours == Hours::Pre {
            return true;
        }
        let mut seen: Vec<Arc<str>> = vec![];
        self.cond_pm(&f.cond, &mut seen)
    }

    fn cond_pm(&self, c: &Cond, seen: &mut Vec<Arc<str>>) -> bool {
        match c {
            Cond::Cmp(a, _, b) => self.num_pm(a, seen) || self.num_pm(b, seen),
            Cond::All(cs) | Cond::Any(cs) => cs.iter().any(|c| self.cond_pm(c, seen)),
            Cond::Not(c) => self.cond_pm(c, seen),
            Cond::Event(_) | Cond::Is(..) => false,
            Cond::Ref(name) => {
                if seen.contains(name) {
                    return false;
                }
                seen.push(name.clone());
                self.cond_defs.iter().find(|(n, _)| n == name).is_some_and(|(_, c)| self.cond_pm(c, seen))
            }
        }
    }

    fn num_pm(&self, e: &Num, seen: &mut Vec<Arc<str>>) -> bool {
        match e {
            Num::Const(_) => false,
            Num::M(m) => matches!(m, Measure::PremarketHigh | Measure::PremarketLow | Measure::PremarketVolume | Measure::VwapExt | Measure::DayVolume { extended: true }),
            Num::Ref(name) => {
                if seen.contains(name) {
                    return false;
                }
                seen.push(name.clone());
                self.num_defs.iter().find(|(n, _)| n == name).is_some_and(|(_, e)| self.num_pm(e, seen))
            }
            Num::Add(a, b) | Num::Sub(a, b) | Num::Mul(a, b) | Num::Div(a, b) | Num::Max(a, b) | Num::Min(a, b) => self.num_pm(a, seen) || self.num_pm(b, seen),
            Num::Neg(a) | Num::Abs(a) => self.num_pm(a, seen),
            Num::If(c, a, b) => self.cond_pm(c, seen) || self.num_pm(a, seen) || self.num_pm(b, seen),
        }
    }

    /// The same definitions with only the named filter: one filter run on its own (a symbol
    /// diagnosis, a before/after test of an edit). `None` when there is no such filter.
    pub fn only(&self, filter: &str) -> Option<Program> {
        let f = self.filters.iter().find(|f| &*f.name == filter)?.clone();
        Some(Program { num_defs: self.num_defs.clone(), cond_defs: self.cond_defs.clone(), filters: vec![f], baskets: self.baskets.clone() })
    }

    pub fn define_num(mut self, name: &str, e: Num) -> Self {
        self.num_defs.push((Arc::from(name), e));
        self
    }
    pub fn define_cond(mut self, name: &str, c: Cond) -> Self {
        self.cond_defs.push((Arc::from(name), c));
        self
    }
    pub fn filter(mut self, f: FilterDef) -> Self {
        self.filters.push(f);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    UnknownName(Arc<str>),
    DuplicateName(Arc<str>),
    DuplicateFilter(Arc<str>),
    Cycle(Arc<str>),
    Empty(&'static str),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::UnknownName(n) => write!(f, "`{n}` is not defined"),
            CompileError::DuplicateName(n) => write!(f, "`{n}` is defined twice"),
            CompileError::DuplicateFilter(n) => write!(f, "filter `{n}` appears twice"),
            CompileError::Cycle(n) => write!(f, "`{n}` depends on itself"),
            CompileError::Empty(what) => write!(f, "empty {what}"),
        }
    }
}

impl std::error::Error for CompileError {}

type NumFn = Box<dyn Fn(&mut Ctx<'_>) -> Option<f64> + Send + Sync>;
type CondFn = Box<dyn Fn(&mut Ctx<'_>) -> Tri + Send + Sync>;

enum NumSlot {
    Measure(Measure),
    Def(NumFn),
}

/// What kind of node an explanation entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Num,
    Cmp,
    And,
    Or,
    Not,
    Event,
    /// A named shared condition (its own clauses are not expanded).
    Ref,
}

/// One node of a compiled filter, for explanations.
#[derive(Debug, Clone)]
struct NodeInfo {
    label: String,
    kind: NodeKind,
    children: Vec<usize>,
}

/// A node's recorded value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TraceVal {
    Num(Option<f64>),
    Bool(Tri),
    /// Only possible for nodes inside a named condition's own evaluation, which is shared and
    /// not part of this filter's tree.
    NotEvaluated,
}

const NO_NODE: usize = usize::MAX;

pub struct CompiledFilter {
    pub name: Arc<str>,
    pub cooldown_ms: i64,
    pub kind: Kind,
    pub side: Side,
    pub hours: Hours,
    pub screen: Option<Arc<str>>,
    columns: Vec<(Arc<str>, NumFn)>,
    rank: Option<NumFn>,
    root: CondFn,
    nodes: Vec<NodeInfo>,
    /// Every numeric slot this filter reads, directly or through definitions, in first-use
    /// order: the measurement vector a signal carries.
    slots: Vec<usize>,
}

/// A compiled program. Immutable and shareable across threads.
pub struct Compiled {
    num_slots: Vec<NumSlot>,
    num_labels: Vec<Arc<str>>,
    cond_slots: Vec<CondFn>,
    filters: Vec<CompiledFilter>,
}

/// Evaluation context for one symbol at one instant. Holds the memo, so build a fresh one per
/// symbol per cycle.
pub struct Ctx<'a> {
    state: &'a SymbolState,
    num_slots: &'a [NumSlot],
    cond_slots: &'a [CondFn],
    num_memo: Vec<Option<Option<f64>>>,
    cond_memo: Vec<Option<Tri>>,
    trace: Option<Vec<TraceVal>>,
}

impl<'a> Ctx<'a> {
    pub fn new(c: &'a Compiled, state: &'a SymbolState) -> Self {
        Ctx {
            state,
            num_slots: &c.num_slots,
            cond_slots: &c.cond_slots,
            num_memo: vec![None; c.num_slots.len()],
            cond_memo: vec![None; c.cond_slots.len()],
            trace: None,
        }
    }

    fn num_slot(&mut self, i: usize) -> Option<f64> {
        if let Some(v) = self.num_memo[i] {
            return v;
        }
        let slots = self.num_slots;
        let v = match &slots[i] {
            NumSlot::Measure(m) => m.eval(self.state),
            NumSlot::Def(f) => f(self),
        }
        .filter(|x| x.is_finite());
        self.num_memo[i] = Some(v);
        v
    }

    fn cond_slot(&mut self, i: usize) -> Tri {
        if let Some(v) = self.cond_memo[i] {
            return v;
        }
        let slots = self.cond_slots;
        // A shared definition's own nodes are not this filter's nodes: evaluate it untraced.
        let saved = self.trace.take();
        let v = slots[i](self);
        self.trace = saved;
        self.cond_memo[i] = Some(v);
        v
    }

    /// The symbol state this context evaluates against.
    pub fn state(&self) -> &'a SymbolState {
        self.state
    }

    fn tracing(&self) -> bool {
        self.trace.is_some()
    }

    fn rec(&mut self, id: usize, v: TraceVal) {
        if id != NO_NODE {
            if let Some(t) = self.trace.as_mut() {
                t[id] = v;
            }
        }
    }
}

/// Direct references found while compiling one body.
#[derive(Default)]
struct Refs {
    nums: Vec<usize>,
    conds: Vec<usize>,
}

struct Compiler {
    measure_slots: HashMap<Measure, usize>,
    num_labels: Vec<Arc<str>>,
    /// `Some(def index)` for slots that are named definitions.
    slot_def: Vec<Option<usize>>,
    num_names: HashMap<Arc<str>, usize>,
    cond_names: HashMap<Arc<str>, usize>,
}

impl Compiler {
    fn measure(&mut self, m: Measure) -> usize {
        if let Some(&i) = self.measure_slots.get(&m) {
            return i;
        }
        let i = self.num_labels.len();
        self.measure_slots.insert(m, i);
        self.num_labels.push(Arc::from(m.to_string()));
        self.slot_def.push(None);
        i
    }

    fn node(nodes: &mut Vec<NodeInfo>, traced: bool, e: &dyn fmt::Display, kind: NodeKind) -> usize {
        if !traced {
            return NO_NODE;
        }
        nodes.push(NodeInfo { label: e.to_string(), kind, children: vec![] });
        nodes.len() - 1
    }

    fn set_children(nodes: &mut [NodeInfo], id: usize, ch: Vec<usize>) {
        if id != NO_NODE {
            nodes[id].children = ch;
        }
    }

    fn num(&mut self, e: &Num, traced: bool, nodes: &mut Vec<NodeInfo>, refs: &mut Refs) -> Result<(NumFn, usize), CompileError> {
        let id = Self::node(nodes, traced, e, NodeKind::Num);
        let f: NumFn = match e {
            Num::Const(x) => {
                let x = *x;
                Box::new(move |c| {
                    c.rec(id, TraceVal::Num(Some(x)));
                    Some(x)
                })
            }
            Num::M(m) => {
                let slot = self.measure(*m);
                refs.nums.push(slot);
                Box::new(move |c| {
                    let v = c.num_slot(slot);
                    c.rec(id, TraceVal::Num(v));
                    v
                })
            }
            Num::Ref(name) => {
                let slot = *self.num_names.get(name).ok_or_else(|| CompileError::UnknownName(name.clone()))?;
                refs.nums.push(slot);
                Box::new(move |c| {
                    let v = c.num_slot(slot);
                    c.rec(id, TraceVal::Num(v));
                    v
                })
            }
            Num::Neg(a) | Num::Abs(a) => {
                let (fa, ia) = self.num(a, traced, nodes, refs)?;
                Self::set_children(nodes, id, vec![ia]);
                let abs = matches!(e, Num::Abs(_));
                Box::new(move |c| {
                    let v = fa(c).map(|x| if abs { x.abs() } else { -x });
                    c.rec(id, TraceVal::Num(v));
                    v
                })
            }
            Num::If(cnd, a, b) => {
                let (fc, ic) = self.cond(cnd, traced, nodes, refs)?;
                let (fa, ia) = self.num(a, traced, nodes, refs)?;
                let (fb, ib) = self.num(b, traced, nodes, refs)?;
                Self::set_children(nodes, id, vec![ic, ia, ib]);
                Box::new(move |c| {
                    let v = match fc(c) {
                        Tri::True => {
                            if c.tracing() {
                                fb(c);
                            }
                            fa(c)
                        }
                        Tri::False => {
                            if c.tracing() {
                                fa(c);
                            }
                            fb(c)
                        }
                        Tri::Unknown => {
                            if c.tracing() {
                                fa(c);
                                fb(c);
                            }
                            None
                        }
                    };
                    c.rec(id, TraceVal::Num(v));
                    v
                })
            }
            Num::Add(a, b) | Num::Sub(a, b) | Num::Mul(a, b) | Num::Div(a, b) | Num::Max(a, b) | Num::Min(a, b) => {
                let (fa, ia) = self.num(a, traced, nodes, refs)?;
                let (fb, ib) = self.num(b, traced, nodes, refs)?;
                Self::set_children(nodes, id, vec![ia, ib]);
                let op: fn(f64, f64) -> Option<f64> = match e {
                    Num::Add(..) => |x, y| Some(x + y),
                    Num::Sub(..) => |x, y| Some(x - y),
                    Num::Mul(..) => |x, y| Some(x * y),
                    Num::Div(..) => |x, y| (y != 0.0).then(|| x / y),
                    Num::Max(..) => |x, y| Some(x.max(y)),
                    _ => |x, y| Some(x.min(y)),
                };
                Box::new(move |c| {
                    let (x, y) = (fa(c), fb(c));
                    let v = match (x, y) {
                        (Some(x), Some(y)) => op(x, y).filter(|r| r.is_finite()),
                        _ => None,
                    };
                    c.rec(id, TraceVal::Num(v));
                    v
                })
            }
        };
        Ok((f, id))
    }

    fn cond(&mut self, e: &Cond, traced: bool, nodes: &mut Vec<NodeInfo>, refs: &mut Refs) -> Result<(CondFn, usize), CompileError> {
        let kind = match e {
            Cond::Cmp(..) => NodeKind::Cmp,
            Cond::All(_) => NodeKind::And,
            Cond::Any(_) => NodeKind::Or,
            Cond::Not(_) => NodeKind::Not,
            Cond::Event(_) | Cond::Is(..) => NodeKind::Event,
            Cond::Ref(_) => NodeKind::Ref,
        };
        let id = Self::node(nodes, traced, e, kind);
        let f: CondFn = match e {
            Cond::Cmp(a, op, b) => {
                let (fa, ia) = self.num(a, traced, nodes, refs)?;
                let (fb, ib) = self.num(b, traced, nodes, refs)?;
                Self::set_children(nodes, id, vec![ia, ib]);
                let op: CmpOp = *op;
                Box::new(move |c| {
                    let (x, y) = (fa(c), fb(c));
                    let v = match (x, y) {
                        (Some(x), Some(y)) => Tri::from_bool(op.apply(x, y)),
                        _ => Tri::Unknown,
                    };
                    c.rec(id, TraceVal::Bool(v));
                    v
                })
            }
            Cond::All(cs) | Cond::Any(cs) => {
                if cs.is_empty() {
                    return Err(CompileError::Empty(if matches!(e, Cond::All(_)) { "AND" } else { "OR" }));
                }
                let is_all = matches!(e, Cond::All(_));
                let mut fs = Vec::with_capacity(cs.len());
                let mut ids = Vec::with_capacity(cs.len());
                for c in cs {
                    let (f, i) = self.cond(c, traced, nodes, refs)?;
                    fs.push(f);
                    ids.push(i);
                }
                Self::set_children(nodes, id, ids);
                // The value that decides the result on its own: False for AND, True for OR.
                let decisive = if is_all { Tri::False } else { Tri::True };
                Box::new(move |c| {
                    let tracing = c.tracing();
                    let mut unknown = false;
                    let mut decided = false;
                    for f in &fs {
                        match f(c) {
                            v if v == decisive => {
                                decided = true;
                                if !tracing {
                                    break;
                                }
                            }
                            Tri::Unknown => unknown = true,
                            _ => {}
                        }
                    }
                    let v = if decided {
                        decisive
                    } else if unknown {
                        Tri::Unknown
                    } else {
                        !decisive
                    };
                    c.rec(id, TraceVal::Bool(v));
                    v
                })
            }
            Cond::Not(a) => {
                let (fa, ia) = self.cond(a, traced, nodes, refs)?;
                Self::set_children(nodes, id, vec![ia]);
                Box::new(move |c| {
                    let v = !fa(c);
                    c.rec(id, TraceVal::Bool(v));
                    v
                })
            }
            Cond::Event(ev) => {
                let ev = *ev;
                Box::new(move |c| {
                    let v = ev.eval(c.state).map_or(Tri::Unknown, Tri::from_bool);
                    c.rec(id, TraceVal::Bool(v));
                    v
                })
            }
            Cond::Is(attr, set) => {
                let (attr, set) = (*attr, set.clone());
                Box::new(move |c| {
                    let r = c.state.reference();
                    let have = match attr {
                        Attr::Type => r.security_type.as_deref(),
                        Attr::Exchange => r.exchange.as_deref(),
                    };
                    let v = have.map_or(Tri::Unknown, |h| Tri::from_bool(set.iter().any(|s| **s == *h)));
                    c.rec(id, TraceVal::Bool(v));
                    v
                })
            }
            Cond::Ref(name) => {
                let slot = *self.cond_names.get(name).ok_or_else(|| CompileError::UnknownName(name.clone()))?;
                refs.conds.push(slot);
                Box::new(move |c| {
                    let v = c.cond_slot(slot);
                    c.rec(id, TraceVal::Bool(v));
                    v
                })
            }
        };
        Ok((f, id))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    OnStack,
    Done,
}

/// The definitions' reference graph.
struct Graph<'g> {
    num_def_refs: &'g [Refs],
    cond_def_refs: &'g [Refs],
    slot_def: &'g [Option<usize>],
}

impl Graph<'_> {
    fn refs(&self, d: DefId) -> &Refs {
        match d {
            DefId::Num(i) => &self.num_def_refs[i],
            DefId::Cond(i) => &self.cond_def_refs[i],
        }
    }

    /// The definition behind slot `s`. Slots past the snapshot are measures that filters
    /// interned later, never definitions.
    fn def_at(&self, s: usize) -> Option<usize> {
        self.slot_def.get(s).copied().flatten()
    }

    /// Depth-first; `Err(d)` names a definition on a cycle.
    fn check_acyclic(&self, d: DefId, marks: &mut HashMap<DefId, Mark>) -> Result<(), DefId> {
        match marks.get(&d) {
            Some(Mark::OnStack) => return Err(d),
            Some(Mark::Done) => return Ok(()),
            None => {}
        }
        marks.insert(d, Mark::OnStack);
        let r = self.refs(d);
        for &s in &r.nums {
            if let Some(i) = self.def_at(s) {
                self.check_acyclic(DefId::Num(i), marks)?;
            }
        }
        for &j in &r.conds {
            self.check_acyclic(DefId::Cond(j), marks)?;
        }
        marks.insert(d, Mark::Done);
        Ok(())
    }

    /// Every numeric slot reachable from `r`, in first-use order.
    fn expand(&self, r: &Refs, out: &mut Vec<usize>, seen_slot: &mut HashSet<usize>, seen_cond: &mut HashSet<usize>) {
        for &s in &r.nums {
            if seen_slot.insert(s) {
                out.push(s);
                if let Some(i) = self.def_at(s) {
                    self.expand(self.refs(DefId::Num(i)), out, seen_slot, seen_cond);
                }
            }
        }
        for &j in &r.conds {
            if seen_cond.insert(j) {
                self.expand(self.refs(DefId::Cond(j)), out, seen_slot, seen_cond);
            }
        }
    }
}

/// Graph node for cycle detection and dependency expansion.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DefId {
    Num(usize),  // index into num_defs
    Cond(usize), // index into cond_defs
}

impl Compiled {
    pub fn new(p: &Program) -> Result<Compiled, CompileError> {
        let mut c = Compiler {
            measure_slots: HashMap::new(),
            num_labels: vec![],
            slot_def: vec![],
            num_names: HashMap::new(),
            cond_names: HashMap::new(),
        };
        // Names first, so bodies can reference definitions declared after them.
        for (i, (name, _)) in p.num_defs.iter().enumerate() {
            if c.num_names.contains_key(name) {
                return Err(CompileError::DuplicateName(name.clone()));
            }
            let slot = c.num_labels.len();
            c.num_names.insert(name.clone(), slot);
            c.num_labels.push(name.clone());
            c.slot_def.push(Some(i));
        }
        for (i, (name, _)) in p.cond_defs.iter().enumerate() {
            if c.num_names.contains_key(name) || c.cond_names.contains_key(name) {
                return Err(CompileError::DuplicateName(name.clone()));
            }
            c.cond_names.insert(name.clone(), i);
        }

        let mut scratch = vec![];
        let mut def_fns: Vec<Option<NumFn>> = vec![];
        let mut num_def_refs = vec![];
        for (_, e) in &p.num_defs {
            let mut r = Refs::default();
            let (f, _) = c.num(e, false, &mut scratch, &mut r)?;
            def_fns.push(Some(f));
            num_def_refs.push(r);
        }
        let mut cond_slots = vec![];
        let mut cond_def_refs = vec![];
        for (_, e) in &p.cond_defs {
            let mut r = Refs::default();
            let (f, _) = c.cond(e, false, &mut scratch, &mut r)?;
            cond_slots.push(f);
            cond_def_refs.push(r);
        }

        // Filters may intern new measures below, so the graph reads a snapshot of the def slots.
        let slot_def = c.slot_def.clone();
        let graph = Graph { num_def_refs: &num_def_refs, cond_def_refs: &cond_def_refs, slot_def: &slot_def };
        let mut marks: HashMap<DefId, Mark> = HashMap::new();
        let name_of = |d: DefId| match d {
            DefId::Num(i) => p.num_defs[i].0.clone(),
            DefId::Cond(i) => p.cond_defs[i].0.clone(),
        };
        for i in 0..p.num_defs.len() {
            graph.check_acyclic(DefId::Num(i), &mut marks).map_err(|d| CompileError::Cycle(name_of(d)))?;
        }
        for j in 0..p.cond_defs.len() {
            graph.check_acyclic(DefId::Cond(j), &mut marks).map_err(|d| CompileError::Cycle(name_of(d)))?;
        }
        let expand = |r: &Refs| {
            let mut out = vec![];
            graph.expand(r, &mut out, &mut HashSet::new(), &mut HashSet::new());
            out
        };

        let mut filters = vec![];
        let mut names = HashSet::new();
        for fd in &p.filters {
            if !names.insert(fd.name.clone()) {
                return Err(CompileError::DuplicateFilter(fd.name.clone()));
            }
            let mut nodes = vec![];
            let mut r = Refs::default();
            let (root, _) = c.cond(&fd.cond, true, &mut nodes, &mut r)?;
            // Columns and rank are display values, not part of the condition: compiled untraced
            // and kept out of the measurement vector.
            let mut display_refs = Refs::default();
            let mut columns = Vec::with_capacity(fd.columns.len());
            for (label, e) in &fd.columns {
                columns.push((label.clone(), c.num(e, false, &mut scratch, &mut display_refs)?.0));
            }
            let rank = match &fd.rank {
                Some(e) => Some(c.num(e, false, &mut scratch, &mut display_refs)?.0),
                None => None,
            };
            filters.push((fd, root, nodes, r, columns, rank));
        }
        let filters = filters
            .into_iter()
            .map(|(fd, root, nodes, r, columns, rank)| CompiledFilter {
                name: fd.name.clone(),
                cooldown_ms: fd.cooldown_ms,
                kind: fd.kind,
                side: fd.side,
                hours: fd.hours,
                screen: fd.screen.clone(),
                columns,
                rank,
                root,
                nodes,
                slots: expand(&r),
            })
            .collect();

        // Assemble numeric slots: definitions by index, measures in the order they were interned.
        let mut num_slots: Vec<Option<NumSlot>> = (0..c.num_labels.len()).map(|_| None).collect();
        for (m, &s) in &c.measure_slots {
            num_slots[s] = Some(NumSlot::Measure(*m));
        }
        for (s, d) in c.slot_def.iter().enumerate() {
            if let Some(i) = d {
                num_slots[s] = Some(NumSlot::Def(def_fns[*i].take().expect("each def compiled once")));
            }
        }
        Ok(Compiled {
            num_slots: num_slots.into_iter().map(|s| s.expect("every slot filled")).collect(),
            num_labels: c.num_labels,
            cond_slots,
            filters,
        })
    }

    pub fn filters(&self) -> &[CompiledFilter] {
        &self.filters
    }

    pub fn filter_index(&self, name: &str) -> Option<usize> {
        self.filters.iter().position(|f| &*f.name == name)
    }

    /// Evaluate filter `k` in `ctx` (short-circuiting, untraced).
    pub fn eval(&self, k: usize, ctx: &mut Ctx<'_>) -> Tri {
        (self.filters[k].root)(ctx)
    }

    /// The measurement vector for filter `k`: every numeric slot it reads, labelled. Computes any
    /// slot that short-circuiting skipped.
    pub fn measurements(&self, k: usize, ctx: &mut Ctx<'_>) -> Vec<(Arc<str>, Option<f64>)> {
        self.filters[k].slots.iter().map(|&s| (self.num_labels[s].clone(), ctx.num_slot(s))).collect()
    }

    /// Filter `k`'s display columns, labelled.
    pub fn columns(&self, k: usize, ctx: &mut Ctx<'_>) -> Vec<(Arc<str>, Option<f64>)> {
        self.filters[k].columns.iter().map(|(l, f)| (l.clone(), f(ctx))).collect()
    }

    /// Filter `k`'s rank, if it has one.
    pub fn rank(&self, k: usize, ctx: &mut Ctx<'_>) -> Option<f64> {
        self.filters[k].rank.as_ref().and_then(|f| f(ctx))
    }

    /// Evaluate filter `k` with tracing on, and return the clause-by-clause tree.
    pub fn explain(&self, k: usize, ctx: &mut Ctx<'_>) -> Explanation {
        let f = &self.filters[k];
        ctx.trace = Some(vec![TraceVal::NotEvaluated; f.nodes.len()]);
        let result = (f.root)(ctx);
        let trace = ctx.trace.take().expect("set above");
        fn build(i: usize, nodes: &[NodeInfo], trace: &[TraceVal]) -> ExplainNode {
            ExplainNode {
                label: nodes[i].label.clone(),
                kind: nodes[i].kind,
                value: trace[i],
                children: nodes[i].children.iter().map(|&c| build(c, nodes, trace)).collect(),
            }
        }
        Explanation { filter: f.name.clone(), result, root: build(0, &f.nodes, &trace) }
    }
}

/// "Which clause was false": a filter's evaluation, node by node.
#[derive(Debug, Clone, PartialEq)]
pub struct Explanation {
    pub filter: Arc<str>,
    pub result: Tri,
    pub root: ExplainNode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainNode {
    pub label: String,
    pub kind: NodeKind,
    pub value: TraceVal,
    pub children: Vec<ExplainNode>,
}

impl Explanation {
    /// The top-level clauses that stopped the filter firing: the false ones, or if none, the
    /// unknown ones.
    pub fn blocking(&self) -> Vec<&ExplainNode> {
        let kids: Vec<&ExplainNode> = match self.root.kind {
            NodeKind::And => self.root.children.iter().collect(),
            _ => vec![&self.root],
        };
        let is = |t: Tri| move |n: &&ExplainNode| n.value == TraceVal::Bool(t);
        let f: Vec<_> = kids.iter().copied().filter(is(Tri::False)).collect();
        if !f.is_empty() {
            f
        } else {
            kids.into_iter().filter(is(Tri::Unknown)).collect()
        }
    }
}

impl fmt::Display for Explanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} => {}", self.filter, self.result.symbol())?;
        fn walk(n: &ExplainNode, depth: usize, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let v = match n.value {
                TraceVal::Bool(t) => format!("[{}]", t.symbol()),
                TraceVal::Num(Some(x)) => format!("= {x}"),
                TraceVal::Num(None) => "= ?".to_string(),
                TraceVal::NotEvaluated => "(not evaluated)".to_string(),
            };
            // Numbers read "label = value"; conditions read "[T/F/?] label".
            if n.kind == NodeKind::Num {
                writeln!(f, "{:w$}{} {v}", "", n.label, w = depth * 2)?;
            } else {
                writeln!(f, "{:w$}{v} {}", "", n.label, w = depth * 2)?;
            }
            for c in &n.children {
                walk(c, depth + 1, f)?;
            }
            Ok(())
        }
        walk(&self.root, 1, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{def, m, n};
    use crate::state::tests::minutes;
    use crate::state::Win;

    fn one(c: Cond) -> Compiled {
        Compiled::new(&Program::default().filter(FilterDef::new("f", c, 0))).unwrap()
    }

    fn run(c: &Compiled, s: &SymbolState) -> Tri {
        let mut ctx = Ctx::new(c, s);
        c.eval(0, &mut ctx)
    }

    #[test]
    fn comparisons_and_arithmetic() {
        let s = minutes(10); // last 18.5, high(5m) 19, low(5m) 14
        let c = one((m(Measure::Last) - m(Measure::Low(Win::rolling_min(5)))).ge(n(4.5)));
        assert_eq!(run(&c, &s), Tri::True);
        let c = one((m(Measure::Last) - m(Measure::Low(Win::rolling_min(5)))).gt(n(4.5)));
        assert_eq!(run(&c, &s), Tri::False);
    }

    #[test]
    fn unknown_propagates_by_kleene_rules() {
        let s = minutes(10); // no daily ref: atr unknown
        let unknown = m(Measure::Atr).gt(n(0.0));
        let t = n(1.0).gt(n(0.0));
        let f = n(0.0).gt(n(1.0));
        assert_eq!(run(&one(unknown.clone()), &s), Tri::Unknown);
        assert_eq!(run(&one(Cond::all([t.clone(), unknown.clone()])), &s), Tri::Unknown);
        assert_eq!(run(&one(Cond::all([f.clone(), unknown.clone()])), &s), Tri::False);
        assert_eq!(run(&one(Cond::any([t.clone(), unknown.clone()])), &s), Tri::True);
        assert_eq!(run(&one(Cond::any([f.clone(), unknown.clone()])), &s), Tri::Unknown);
        assert_eq!(run(&one(Cond::not(unknown)), &s), Tri::Unknown);
        assert_eq!(run(&one((n(1.0) / n(0.0)).gt(n(0.0))), &s), Tri::Unknown, "x/0 is unknown");
    }

    #[test]
    fn named_definitions_and_shared_conditions() {
        let p = Program::default()
            .define_num("range5", m(Measure::High(Win::rolling_min(5))) - m(Measure::Low(Win::rolling_min(5))))
            .define_cond("liquid", m(Measure::DayVolume { extended: false }).ge(n(1000.0)))
            .filter(FilterDef::new("wide", Cond::all([Cond::named("liquid"), def("range5").ge(n(5.0))]), 0));
        let c = Compiled::new(&p).unwrap();
        let s = minutes(10); // range5 = 19 - 14 = 5; day volume 5500
        let mut ctx = Ctx::new(&c, &s);
        assert_eq!(c.eval(0, &mut ctx), Tri::True);
        let v = c.measurements(0, &mut ctx);
        let labels: Vec<&str> = v.iter().map(|(l, _)| &**l).collect();
        assert_eq!(labels, vec!["range5", "high(5m)", "low(5m)", "day_volume"]);
        assert_eq!(v[0].1, Some(5.0));
    }

    #[test]
    fn compile_errors() {
        let bad = |p: Program| Compiled::new(&p).err().unwrap();
        assert_eq!(
            bad(Program::default().filter(FilterDef::new("f", def("nope").gt(n(0.0)), 0))),
            CompileError::UnknownName(Arc::from("nope"))
        );
        assert!(matches!(
            bad(Program::default().define_num("a", def("b")).define_num("b", def("a") + n(1.0))),
            CompileError::Cycle(_)
        ));
        assert!(matches!(bad(Program::default().define_cond("c", Cond::named("c"))), CompileError::Cycle(_)));
        assert!(matches!(bad(Program::default().define_num("a", n(1.0)).define_num("a", n(2.0))), CompileError::DuplicateName(_)));
        assert!(matches!(
            bad(Program::default().filter(FilterDef::new("f", Cond::all([]), 0))),
            CompileError::Empty("AND")
        ));
        let f = FilterDef::new("f", n(1.0).gt(n(0.0)), 0);
        assert!(matches!(bad(Program::default().filter(f.clone()).filter(f)), CompileError::DuplicateFilter(_)));
    }

    #[test]
    fn explain_names_the_blocking_clause_and_agrees_with_eval() {
        let s = minutes(10);
        let c = one(Cond::all([
            m(Measure::Last).ge(n(2.0)),
            m(Measure::Volume(Win::rolling_min(5))).ge(m(Measure::Volume(Win::rolling_min_back(5, 1))) * n(3.0)),
            m(Measure::Atr).gt(n(0.0)),
        ]));
        let mut ctx = Ctx::new(&c, &s);
        let fast = c.eval(0, &mut ctx);
        let mut ctx = Ctx::new(&c, &s);
        let ex = c.explain(0, &mut ctx);
        assert_eq!(ex.result, fast);
        assert_eq!(ex.result, Tri::False);
        // volume(5m) = 600+..+1000 = 4000; 3 × volume(5m, 5m back) = 3 × 1500 = 4500 → false.
        let blocking = ex.blocking();
        assert_eq!(blocking.len(), 1);
        assert!(blocking[0].label.starts_with("volume(5m) >="), "{}", blocking[0].label);
        // Trace mode evaluated the clause after the false one too.
        assert_eq!(ex.root.children[2].value, TraceVal::Bool(Tri::Unknown));
        let text = ex.to_string();
        assert!(text.contains("[F] volume(5m) >= (volume(5m, 5m back) * 3)"), "{text}");
        assert!(text.contains("volume(5m) = 4000"), "{text}");
    }

    #[test]
    fn if_picks_a_branch_and_an_unknown_condition_is_unknown() {
        use crate::expr::if_;
        let s = minutes(10); // last 18.5, no daily ref
        let pick = |c: Cond| {
            let cc = one(if_(c, n(1.0), n(2.0)).ge(n(2.0)));
            run(&cc, &s)
        };
        assert_eq!(pick(m(Measure::Last).gt(n(10.0))), Tri::False, "took the 1.0 branch");
        assert_eq!(pick(m(Measure::Last).lt(n(10.0))), Tri::True, "took the 2.0 branch");
        assert_eq!(pick(m(Measure::Atr).gt(n(0.0))), Tri::Unknown);
        // Nested, the way the RVOL curve is written.
        let curve = if_(m(Measure::Time).le(n(570.0)), n(0.01), if_(m(Measure::Time).le(n(600.0)), n(0.15), n(1.0)));
        let c = one(curve.clone().ge(n(0.15)));
        assert_eq!(run(&c, &s), Tri::True, "09:39 is in the second segment");
        let mut ctx = Ctx::new(&c, &s);
        let ex = c.explain(0, &mut ctx);
        assert!(ex.to_string().contains("if(time <= 570, 0.01, if(time <= 600, 0.15, 1)) = 0.15"), "{ex}");
    }

    #[test]
    fn columns_and_rank_are_computed_but_not_measurements() {
        let mut f = FilterDef::new("f", m(Measure::Last).gt(n(0.0)), 0);
        f.columns = vec![(Arc::from("RANGE5"), m(Measure::High(Win::rolling_min(5))) - m(Measure::Low(Win::rolling_min(5))))];
        f.rank = Some(m(Measure::Last) * n(2.0));
        let c = Compiled::new(&Program::default().filter(f)).unwrap();
        let s = minutes(10);
        let mut ctx = Ctx::new(&c, &s);
        assert_eq!(c.columns(0, &mut ctx), vec![(Arc::from("RANGE5"), Some(5.0))]);
        assert_eq!(c.rank(0, &mut ctx), Some(37.0));
        let labels: Vec<String> = c.measurements(0, &mut ctx).into_iter().map(|(l, _)| l.to_string()).collect();
        assert_eq!(labels, vec!["last"]);
    }

    #[test]
    fn a_measure_used_by_many_filters_occupies_one_slot() {

        let mut p = Program::default();
        for i in 0..40 {
            p = p.filter(FilterDef::new(&format!("f{i}"), m(Measure::Atr).gt(n(i as f64)), 0));
        }
        let c = Compiled::new(&p).unwrap();
        assert_eq!(c.num_slots.len(), 1);
    }
}
