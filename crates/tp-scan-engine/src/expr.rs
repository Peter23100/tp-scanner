//! Filter expressions as a tree. This is the evaluator's input, **not** the user-facing language.
//!
//! There is no text syntax yet, deliberately: the user's exported Primus filters are the
//! conformance set the syntax gets designed against. Until then, filters are built in Rust with
//! the helpers below, e.g.
//!
//! ```
//! use tp_scan_engine::expr::{m, n, Cond};
//! use tp_scan_engine::measure::Measure;
//! use tp_scan_engine::state::Win;
//! let spike = Cond::all([
//!     m(Measure::Last).ge(n(2.0)),
//!     (m(Measure::Last) - m(Measure::Low(Win::rolling_min(5)))).ge(m(Measure::Atr) * n(0.3)),
//! ]);
//! assert_eq!(spike.to_string(), "(last >= 2) AND ((last - low(5m)) >= (atr * 0.3))");
//! ```
//!
//! The shape is fixed by the investigation: boolean combinations of numeric comparisons, named
//! numeric intermediates, no loops, no arrays, no state across evaluations. Numeric and boolean
//! expressions are separate types, so "compare a condition to a number" can't be built.

use crate::measure::{Event, Measure};
use std::fmt;
use std::ops;
use std::sync::Arc;

/// A numeric expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Num {
    Const(f64),
    M(Measure),
    /// A named intermediate, defined once in the [`crate::compile::Program`] and computed once
    /// per symbol per cycle however many filters use it.
    Ref(Arc<str>),
    Add(Box<Num>, Box<Num>),
    Sub(Box<Num>, Box<Num>),
    Mul(Box<Num>, Box<Num>),
    /// Division by zero is unknown, not infinity.
    Div(Box<Num>, Box<Num>),
    Neg(Box<Num>),
    Abs(Box<Num>),
    Max(Box<Num>, Box<Num>),
    Min(Box<Num>, Box<Num>),
}

/// Ordering comparisons only. Float equality is a trap in a filter language, so it isn't offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }

    pub fn apply(self, a: f64, b: f64) -> bool {
        match self {
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        }
    }
}

/// A condition.
#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Cmp(Box<Num>, CmpOp, Box<Num>),
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
    Event(Event),
    /// A named shared condition, such as the single `Tradeable()` predicate every filter ANDs
    /// with (investigation §7c): tune it in one place and every filter follows.
    Ref(Arc<str>),
}

/// A measurement.
pub fn m(x: Measure) -> Num {
    Num::M(x)
}

/// A constant.
pub fn n(x: f64) -> Num {
    Num::Const(x)
}

/// A reference to a named numeric intermediate.
pub fn def(name: &str) -> Num {
    Num::Ref(Arc::from(name))
}

impl Num {
    fn cmp(self, op: CmpOp, o: Num) -> Cond {
        Cond::Cmp(Box::new(self), op, Box::new(o))
    }
    pub fn lt(self, o: Num) -> Cond {
        self.cmp(CmpOp::Lt, o)
    }
    pub fn le(self, o: Num) -> Cond {
        self.cmp(CmpOp::Le, o)
    }
    pub fn gt(self, o: Num) -> Cond {
        self.cmp(CmpOp::Gt, o)
    }
    pub fn ge(self, o: Num) -> Cond {
        self.cmp(CmpOp::Ge, o)
    }
    pub fn abs(self) -> Num {
        Num::Abs(Box::new(self))
    }
    pub fn max(self, o: Num) -> Num {
        Num::Max(Box::new(self), Box::new(o))
    }
    pub fn min(self, o: Num) -> Num {
        Num::Min(Box::new(self), Box::new(o))
    }
}

macro_rules! binop {
    ($tr:ident, $f:ident, $v:ident) => {
        impl ops::$tr for Num {
            type Output = Num;
            fn $f(self, o: Num) -> Num {
                Num::$v(Box::new(self), Box::new(o))
            }
        }
    };
}
binop!(Add, add, Add);
binop!(Sub, sub, Sub);
binop!(Mul, mul, Mul);
binop!(Div, div, Div);

impl ops::Neg for Num {
    type Output = Num;
    fn neg(self) -> Num {
        Num::Neg(Box::new(self))
    }
}

impl Cond {
    pub fn all(cs: impl IntoIterator<Item = Cond>) -> Cond {
        Cond::All(cs.into_iter().collect())
    }
    pub fn any(cs: impl IntoIterator<Item = Cond>) -> Cond {
        Cond::Any(cs.into_iter().collect())
    }
    #[allow(clippy::should_implement_trait)]
    pub fn not(c: Cond) -> Cond {
        Cond::Not(Box::new(c))
    }
    pub fn named(name: &str) -> Cond {
        Cond::Ref(Arc::from(name))
    }
    pub fn event(e: Event) -> Cond {
        Cond::Event(e)
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Num::Const(x) => write!(f, "{x}"),
            Num::M(m) => write!(f, "{m}"),
            Num::Ref(r) => write!(f, "{r}"),
            Num::Add(a, b) => write!(f, "({a} + {b})"),
            Num::Sub(a, b) => write!(f, "({a} - {b})"),
            Num::Mul(a, b) => write!(f, "({a} * {b})"),
            Num::Div(a, b) => write!(f, "({a} / {b})"),
            Num::Neg(a) => write!(f, "-{a}"),
            Num::Abs(a) => write!(f, "abs({a})"),
            Num::Max(a, b) => write!(f, "max({a}, {b})"),
            Num::Min(a, b) => write!(f, "min({a}, {b})"),
        }
    }
}

impl fmt::Display for Cond {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let join = |f: &mut fmt::Formatter<'_>, cs: &[Cond], sep: &str| -> fmt::Result {
            for (i, c) in cs.iter().enumerate() {
                if i > 0 {
                    write!(f, " {sep} ")?;
                }
                match c {
                    Cond::All(_) | Cond::Any(_) | Cond::Cmp(..) => write!(f, "({c})")?,
                    _ => write!(f, "{c}")?,
                }
            }
            Ok(())
        };
        match self {
            Cond::Cmp(a, op, b) => write!(f, "{a} {} {b}", op.symbol()),
            Cond::All(cs) => join(f, cs, "AND"),
            Cond::Any(cs) => join(f, cs, "OR"),
            Cond::Not(c) => write!(f, "NOT ({c})"),
            Cond::Event(e) => write!(f, "{e}"),
            Cond::Ref(r) => write!(f, "{r}"),
        }
    }
}
