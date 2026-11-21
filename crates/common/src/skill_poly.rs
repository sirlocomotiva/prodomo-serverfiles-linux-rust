//! The skill formula evaluator: the hosted subset of `CPoly`
//! (`server/server/libpoly/Poly.cpp`).
//!
//! Legacy compiles each `skill_proto` polynomial once (`CSkillProto::kPointPoly` etc.,
//! `G/skill.cpp`) and evaluates it per cast and per victim with the cast's variables
//! (`G/char_skill.cpp:2153-2234`, `:1397-1467`). The evaluator is a recursive-descent
//! parser (`expr → term → factor → expo`) over a token stack, evaluated in `double`.
//!
//! This port covers exactly the vocabulary the seven skill rows the owner's skill mobs
//! name use (20, 256, 258, 263, 264, 265, 266 — ledger 229u): decimal literals, the
//! variables `atk k lv iq str dex con maxhp maxsp ar chain def odef horse_level wep mtk
//! mwep maxv ek`, the operators `+ - * / % ^`, parentheses, and unary plus/minus. The
//! grammar below mirrors `CPoly::expr/term/factor/expo` including its quirks:
//!
//! - A leading `+`/`-` (and one after `(`) is parsed as `0 <op> operand`
//!   (`Poly.cpp:expr` pushes a zero `NUM`). So `-x` is `0 - x`; `--x` is a parse
//!   error (the zero's `term` → `expo` meets the second `-`).
//! - `^` binds TIGHTER than `*` (`factor → expo {^ expo}`, `term → factor {* / %}
//!   factor`), and chains LEFT-associatively (`2^3^2` is `(2^3)^2 = 64`).
//! - `/` and `%` by an exact zero return 0 for the WHOLE evaluation (`Eval` returns,
//!   `Poly.cpp:100-117`), not just the operation. `%` is `fmod`.
//! - A name is only letters (`isalpha`, `lexan`); digits and `_` end it. `atk2` lexes
//!   as `atk` followed by `2`, which is a parse error. Every hosted poly respects this.
//! - A number is digits with an optional fraction (`1.` is 1.0); no exponent, no hex.
//! - An empty poly returns 0. Legacy reads stack garbage (`save[-1]`, `Poly.cpp:219`),
//!   a Defect the owner's 266 row (empty point and duration polys) triggers; the
//!   Rewrite evaluates the no-op (ledger 229u).
//! - An unknown variable reads 0 (`CSymTable`'s `dVal(0)`, `SymTable.cpp:7`), and
//!   setting a name the poly never mentions is silently dropped (`SetVar`'s
//!   `find == -1` arm, `Poly.cpp:562-571`).
//! - A parse error refuses the row at load (naming it); legacy logs `sys_err` and
//!   skips the skill (`G/skill.cpp`), so an unparseable row never casts either.
//!
//! Refused, not ported: every function (`number`, `floor`, `min`, `max`, trig, log,
//! `abs`, `sign`, `rt`) — only PC skill rows use them — and anything outside the ASCII
//! above. A refused poly is a load error naming the row, mirroring legacy's
//! `sys_err` + skip (`G/skill.cpp`).

use std::collections::BTreeMap;

/// One compiled skill formula.
#[derive(Debug, Clone, PartialEq)]
pub struct Poly {
    ops: Vec<Op>,
    numbers: Vec<f64>,
    vars: Vec<String>,
}

/// A stack-machine operation, as `CPoly::tokenBase` keeps it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    /// A literal, by index into [`Poly::numbers`].
    Num(usize),
    /// A variable, by index into [`Poly::vars`].
    Var(usize),
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`, whole-eval 0 on an exact-zero divisor.
    Div,
    /// `fmod`, whole-eval 0 on an exact-zero divisor.
    Mod,
    /// `powf`.
    Pow,
}

/// A poly that failed to compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolyError {
    /// The byte offset the parse stopped at.
    pub offset: usize,
    /// What was wrong.
    pub kind: PolyErrorKind,
}

/// Why a poly was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolyErrorKind {
    /// An unexpected byte or end of input.
    Unexpected,
    /// A `(` without its `)`.
    Unclosed,
    /// A function call or any other construct outside the hosted subset.
    Unsupported,
}

impl Poly {
    /// Compile `source`, as `CPoly::Analyze` does.
    ///
    /// # Errors
    ///
    /// Returns [`PolyError`] for anything outside the hosted subset (a function call,
    /// a stray byte, an unclosed parenthesis). An empty or blank source compiles to
    /// the 0 poly (see the module docs).
    pub fn parse(source: &str) -> Result<Self, PolyError> {
        let mut parser = Parser {
            bytes: source.as_bytes(),
            at: 0,
            numbers: Vec::new(),
            ops: Vec::new(),
            vars: Vec::new(),
        };
        parser.skip_blank();
        if parser.at >= parser.bytes.len() {
            return Ok(Self {
                ops: Vec::new(),
                numbers: Vec::new(),
                vars: Vec::new(),
            });
        }
        parser.expr()?;
        parser.skip_blank();
        if parser.at < parser.bytes.len() {
            return Err(parser.error(PolyErrorKind::Unexpected));
        }
        if parser.ops.is_empty() {
            return Err(parser.error(PolyErrorKind::Unexpected));
        }
        Ok(Self {
            ops: parser.ops,
            numbers: parser.numbers,
            vars: parser.vars,
        })
    }

    /// Evaluate the poly with `vars`, as `CPoly::Eval` does, in `f64`.
    ///
    /// A name the poly never mentions is dropped; a name it mentions but `vars`
    /// lacks reads 0. `/0`, `%0` and the empty poly answer 0.
    #[must_use]
    pub fn eval(&self, vars: &BTreeMap<&str, f64>) -> f64 {
        let mut stack: Vec<f64> = Vec::new();
        for op in &self.ops {
            match *op {
                Op::Num(index) => stack.push(self.numbers[index]),
                Op::Var(index) => {
                    stack.push(vars.get(self.vars[index].as_str()).copied().unwrap_or(0.0));
                }
                Op::Add => {
                    let rhs = stack.pop().unwrap_or(0.0);
                    let lhs = stack.pop().unwrap_or(0.0);
                    stack.push(lhs + rhs);
                }
                Op::Sub => {
                    let rhs = stack.pop().unwrap_or(0.0);
                    let lhs = stack.pop().unwrap_or(0.0);
                    stack.push(lhs - rhs);
                }
                Op::Mul => {
                    let rhs = stack.pop().unwrap_or(0.0);
                    let lhs = stack.pop().unwrap_or(0.0);
                    stack.push(lhs * rhs);
                }
                Op::Div => {
                    let rhs = stack.pop().unwrap_or(0.0);
                    let lhs = stack.pop().unwrap_or(0.0);
                    if rhs == 0.0 {
                        return 0.0;
                    }
                    stack.push(lhs / rhs);
                }
                Op::Mod => {
                    let rhs = stack.pop().unwrap_or(0.0);
                    let lhs = stack.pop().unwrap_or(0.0);
                    if rhs == 0.0 {
                        return 0.0;
                    }
                    stack.push(lhs % rhs);
                }
                Op::Pow => {
                    let rhs = stack.pop().unwrap_or(0.0);
                    let lhs = stack.pop().unwrap_or(0.0);
                    stack.push(lhs.powf(rhs));
                }
            }
        }
        stack.pop().unwrap_or(0.0)
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    numbers: Vec<f64>,
    ops: Vec<Op>,
    vars: Vec<String>,
}

impl Parser<'_> {
    fn error(&self, kind: PolyErrorKind) -> PolyError {
        PolyError {
            offset: self.at,
            kind,
        }
    }

    fn skip_blank(&mut self) {
        while self.at < self.bytes.len()
            && (self.bytes[self.at] == b' ' || self.bytes[self.at] == b'\t')
        {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_blank();
        self.bytes.get(self.at).copied()
    }

    /// `CPoly::expr`: a leading sign becomes `0 <op> operand` — the zero is pushed
    /// and the loop below consumes the sign as the operator.
    fn expr(&mut self) -> Result<(), PolyError> {
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.push_number(0.0);
        } else {
            self.term()?;
        }
        loop {
            match self.peek() {
                Some(b'+') => {
                    self.at += 1;
                    self.term()?;
                    self.ops.push(Op::Add);
                }
                Some(b'-') => {
                    self.at += 1;
                    self.term()?;
                    self.ops.push(Op::Sub);
                }
                _ => return Ok(()),
            }
        }
    }

    /// `CPoly::term`.
    fn term(&mut self) -> Result<(), PolyError> {
        self.factor()?;
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.at += 1;
                    self.factor()?;
                    self.ops.push(Op::Mul);
                }
                Some(b'/') => {
                    self.at += 1;
                    self.factor()?;
                    self.ops.push(Op::Div);
                }
                Some(b'%') => {
                    self.at += 1;
                    self.factor()?;
                    self.ops.push(Op::Mod);
                }
                _ => return Ok(()),
            }
        }
    }

    /// `CPoly::factor`: `^` chains left-associatively.
    fn factor(&mut self) -> Result<(), PolyError> {
        self.expo()?;
        loop {
            match self.peek() {
                Some(b'^') => {
                    self.at += 1;
                    self.expo()?;
                    self.ops.push(Op::Pow);
                }
                _ => return Ok(()),
            }
        }
    }

    /// `CPoly::expo`: parentheses, numbers, names. A name followed by `(` is a
    /// function call, which the hosted subset refuses.
    fn expo(&mut self) -> Result<(), PolyError> {
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                self.expr()?;
                if self.peek() == Some(b')') {
                    self.at += 1;
                    Ok(())
                } else {
                    Err(self.error(PolyErrorKind::Unclosed))
                }
            }
            Some(byte) if byte.is_ascii_digit() => {
                self.number();
                Ok(())
            }
            Some(byte) if byte.is_ascii_alphabetic() => self.name(),
            _ => Err(self.error(PolyErrorKind::Unexpected)),
        }
    }

    /// `lexan`'s number: digits, then an optional fraction folded in decimally.
    /// Infallible: `expo` only calls it on a leading digit.
    fn number(&mut self) {
        let mut value = 0.0;
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_digit() {
            value = value * 10.0 + f64::from(self.bytes[self.at] - b'0');
            self.at += 1;
        }
        if self.at < self.bytes.len() && self.bytes[self.at] == b'.' {
            self.at += 1;
            let mut place = 0.1;
            while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_digit() {
                value += place * f64::from(self.bytes[self.at] - b'0');
                place *= 0.1;
                self.at += 1;
            }
        }
        self.push_number(value);
    }

    /// `lexan`'s name: letters only. A `(` after it is refused (a function call).
    fn name(&mut self) -> Result<(), PolyError> {
        let start = self.at;
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_alphabetic() {
            self.at += 1;
        }
        let name = &self.bytes[start..self.at];
        if self.peek() == Some(b'(') {
            return Err(self.error(PolyErrorKind::Unsupported));
        }
        let name = String::from_utf8_lossy(name).into_owned();
        if let Some(index) = self.vars.iter().position(|known| *known == name) {
            self.ops.push(Op::Var(index));
        } else {
            self.ops.push(Op::Var(self.vars.len()));
            self.vars.push(name);
        }
        Ok(())
    }

    fn push_number(&mut self, value: f64) {
        self.ops.push(Op::Num(self.numbers.len()));
        self.numbers.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &'a [(&'a str, f64)]) -> BTreeMap<&'a str, f64> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn hosted_polys_evaluate() {
        // Every distinct hosted poly string, at one fixed var set, against hand values.
        let bound = vars(&[
            ("atk", 1000.0),
            ("k", 0.5),
            ("lv", 90.0),
            ("iq", 20.0),
            ("str", 17.0),
            ("dex", 19.0),
            ("con", 18.0),
            ("maxhp", 5000.0),
            ("ar", 0.9),
            ("def", 100.0),
            ("wep", 50.0),
        ]);
        let cases = [
            (
                "-(2*atk + (atk + dex*3 + str*5 + con)*k)",
                -(2.0 * 1000.0 + (1000.0 + 19.0 * 3.0 + 17.0 * 5.0 + 18.0) * 0.5),
            ),
            ("-5*k*atk", -5.0 * 0.5 * 1000.0),
            ("-maxhp*k", -5000.0 * 0.5),
            ("-70", -70.0),
            ("3", 3.0),
            ("20", 20.0),
            ("4", 4.0),
            ("8", 8.0),
            ("12", 12.0),
            ("5", 5.0),
            ("", 0.0),
        ];
        for (source, want) in cases {
            let poly = Poly::parse(source).unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
            let got = poly.eval(&bound);
            assert!(
                (got - want).abs() < 1e-9,
                "{source:?}: got {got}, want {want}"
            );
        }
    }

    #[test]
    fn grammar_quirks_match_cpoly() {
        let bound = vars(&[("x", 3.0)]);
        // Unary minus is `0 - x`.
        assert_eq!(
            Poly::parse("-x").unwrap().eval(&bound).to_bits(),
            (-3.0_f64).to_bits()
        );
        // `--x`: legacy pushes 0, then `term` → `expo` sees `-` and errors.
        assert!(Poly::parse("--x").is_err());
        // `^` binds tighter than `*`, and chains left.
        assert_eq!(
            Poly::parse("2*3^2").unwrap().eval(&bound).to_bits(),
            18.0_f64.to_bits()
        );
        assert_eq!(
            Poly::parse("2^3^2").unwrap().eval(&bound).to_bits(),
            64.0_f64.to_bits()
        );
        // `%` is `fmod`.
        assert_eq!(
            Poly::parse("7%3").unwrap().eval(&bound).to_bits(),
            1.0_f64.to_bits()
        );
        // `/0` and `%0` zero the whole evaluation.
        assert_eq!(
            Poly::parse("1+1/0").unwrap().eval(&bound).to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(
            Poly::parse("1+1%0").unwrap().eval(&bound).to_bits(),
            0.0_f64.to_bits()
        );
        // Unknown vars read 0; unset names are dropped silently.
        assert_eq!(
            Poly::parse("x+missing").unwrap().eval(&bound).to_bits(),
            3.0_f64.to_bits()
        );
        // A name ends at a digit; the digit then fails the parse... but `atk2`
        // lexes `atk` then `2`, and `expr` stops after `atk` with trailing bytes.
        assert!(Poly::parse("atk2").is_err());
    }

    #[test]
    fn refused_constructs_error() {
        assert_eq!(
            Poly::parse("number(1,2)").unwrap_err().kind,
            PolyErrorKind::Unsupported
        );
        assert_eq!(
            Poly::parse("floor(k)").unwrap_err().kind,
            PolyErrorKind::Unsupported
        );
        assert_eq!(
            Poly::parse("(1+2").unwrap_err().kind,
            PolyErrorKind::Unclosed
        );
        assert!(Poly::parse("1+").is_err());
        assert!(Poly::parse(")").is_err());
    }

    #[test]
    fn truncation_is_toward_zero() {
        // `(int)double` at the call sites: the evaluator returns the double, the cast
        // truncates. Pin the direction for negatives.
        let bound = vars(&[("k", 0.06)]);
        let poly = Poly::parse("-5*k*atk").unwrap();
        let mut with_atk = bound.clone();
        with_atk.insert("atk", 999.0);
        let value = poly.eval(&with_atk);
        assert!((value - -299.7).abs() < 1e-9);
        assert_eq!(crate::cfloat::f64_to_i32(value), -299);
    }
}
