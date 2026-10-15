//! A tiny expression language for the Formula modulator: `sin(t * 2) * a + b`.
//!
//! Variables: `t` (seconds), `beat` (beats since the device started), `bpm`, `a` `b` `c` `d` (the device's
//! four knobs, 0..1), `note` (last note on the track, 0..119), `vel` (its velocity, 0..1), `pi`.
//! Operators: `+ - * / % ^` and unary minus; parentheses. Functions: `sin cos tan abs sqrt floor ceil fract
//! sign exp ln min max pow mod clamp mix tri saw square step`. Compiled once to a postfix program; evaluating
//! it never allocates.

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Num(f32),
    Var(u8),
    Neg,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    /// Function id and argument count.
    Call(u8, u8),
}

const VARS: [&str; 11] = ["t", "beat", "bpm", "a", "b", "c", "d", "note", "vel", "pi", "tau"];
/// (name, argument count)
const FUNCS: [(&str, u8); 22] = [
    ("sin", 1),
    ("cos", 1),
    ("tan", 1),
    ("abs", 1),
    ("sqrt", 1),
    ("floor", 1),
    ("ceil", 1),
    ("fract", 1),
    ("sign", 1),
    ("exp", 1),
    ("ln", 1),
    ("min", 2),
    ("max", 2),
    ("pow", 2),
    ("mod", 2),
    ("clamp", 3),
    ("mix", 3),
    ("tri", 1),
    ("saw", 1),
    ("square", 1),
    ("step", 2),
    ("noise", 1),
];

/// Values the formula can read.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub t: f32,
    pub beat: f32,
    pub bpm: f32,
    pub knobs: [f32; 4],
    pub note: f32,
    pub vel: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Formula {
    ops: Vec<Op>,
}

const MAX_OPS: usize = 256;
const STACK: usize = 32;

struct Parser<'a> {
    src: &'a [u8],
    i: usize,
    ops: Vec<Op>,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.src.len() && self.src[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.src.get(self.i).copied()
    }

    fn push(&mut self, op: Op) -> Result<(), String> {
        if self.ops.len() >= MAX_OPS {
            return Err("formula is too long".into());
        }
        self.ops.push(op);
        Ok(())
    }

    fn expr(&mut self) -> Result<(), String> {
        self.term()?;
        while let Some(c) = self.peek() {
            let op = match c {
                b'+' => Op::Add,
                b'-' => Op::Sub,
                _ => break,
            };
            self.i += 1;
            self.term()?;
            self.push(op)?;
        }
        Ok(())
    }

    fn term(&mut self) -> Result<(), String> {
        self.unary()?;
        while let Some(c) = self.peek() {
            let op = match c {
                b'*' => Op::Mul,
                b'/' => Op::Div,
                b'%' => Op::Rem,
                _ => break,
            };
            self.i += 1;
            self.unary()?;
            self.push(op)?;
        }
        Ok(())
    }

    fn unary(&mut self) -> Result<(), String> {
        if self.peek() == Some(b'-') {
            self.i += 1;
            self.unary()?;
            return self.push(Op::Neg);
        }
        if self.peek() == Some(b'+') {
            self.i += 1;
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<(), String> {
        self.atom()?;
        if self.peek() == Some(b'^') {
            self.i += 1;
            self.unary()?; // right associative, binds tighter than unary minus on the left
            self.push(Op::Pow)?;
        }
        Ok(())
    }

    fn atom(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > 24 {
            return Err("formula is nested too deeply".into());
        }
        let r = self.atom_inner();
        self.depth -= 1;
        r
    }

    fn atom_inner(&mut self) -> Result<(), String> {
        match self.peek() {
            None => Err("the formula ends too early".into()),
            Some(b'(') => {
                self.i += 1;
                self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("missing )".into());
                }
                self.i += 1;
                Ok(())
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self.i < self.src.len()
                    && (self.src[self.i].is_ascii_digit() || self.src[self.i] == b'.')
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.src[start..self.i]).unwrap_or("");
                let n: f32 = text.parse().map_err(|_| format!("bad number {text:?}"))?;
                self.push(Op::Num(n))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let start = self.i;
                while self.i < self.src.len()
                    && (self.src[self.i].is_ascii_alphanumeric() || self.src[self.i] == b'_')
                {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.src[start..self.i]).unwrap_or("").to_ascii_lowercase();
                if self.peek() == Some(b'(') {
                    let Some(fi) = FUNCS.iter().position(|f| f.0 == name) else {
                        return Err(format!("unknown function {name}"));
                    };
                    self.i += 1;
                    let mut n = 0;
                    if self.peek() != Some(b')') {
                        loop {
                            self.expr()?;
                            n += 1;
                            match self.peek() {
                                Some(b',') => self.i += 1,
                                Some(b')') => break,
                                _ => return Err("expected , or )".into()),
                            }
                        }
                    }
                    self.i += 1;
                    if n != FUNCS[fi].1 {
                        return Err(format!("{name} takes {} argument(s), not {n}", FUNCS[fi].1));
                    }
                    self.push(Op::Call(fi as u8, n))
                } else if let Some(vi) = VARS.iter().position(|v| *v == name) {
                    self.push(Op::Var(vi as u8))
                } else {
                    Err(format!("unknown name {name} (try t, beat, a, b, c, d, note, vel)"))
                }
            }
            Some(c) => Err(format!("unexpected {:?}", c as char)),
        }
    }
}

impl Formula {
    /// Compile `src`. An empty formula is valid and evaluates to 0.
    pub fn compile(src: &str) -> Result<Formula, String> {
        if src.trim().is_empty() {
            return Ok(Formula::default());
        }
        if src.len() > 400 {
            return Err("formula is too long".into());
        }
        let mut p = Parser { src: src.as_bytes(), i: 0, ops: Vec::new(), depth: 0 };
        p.expr()?;
        if p.peek().is_some() {
            return Err(format!("unexpected {:?}", p.src[p.i] as char));
        }
        let f = Formula { ops: p.ops };
        f.check_stack()?;
        Ok(f)
    }

    fn check_stack(&self) -> Result<(), String> {
        let mut depth: i32 = 0;
        for op in &self.ops {
            depth += match op {
                Op::Num(_) | Op::Var(_) => 1,
                Op::Neg => 0,
                Op::Call(_, n) => 1 - *n as i32,
                _ => -1,
            };
            if depth < 1 || depth as usize > STACK {
                return Err("formula is too complicated".into());
            }
        }
        if depth == 1 {
            Ok(())
        } else {
            Err("formula is malformed".into())
        }
    }

    /// Evaluate, clamped to -1..1 (non-finite results become 0).
    pub fn eval(&self, inp: &Inputs) -> f32 {
        if self.ops.is_empty() {
            return 0.0;
        }
        let mut st = [0.0f32; STACK];
        let mut sp = 0usize;
        for op in &self.ops {
            match *op {
                Op::Num(n) => {
                    st[sp] = n;
                    sp += 1;
                }
                Op::Var(v) => {
                    st[sp] = match v {
                        0 => inp.t,
                        1 => inp.beat,
                        2 => inp.bpm,
                        3..=6 => inp.knobs[(v - 3) as usize],
                        7 => inp.note,
                        8 => inp.vel,
                        9 => std::f32::consts::PI,
                        _ => std::f32::consts::TAU,
                    };
                    sp += 1;
                }
                Op::Neg => st[sp - 1] = -st[sp - 1],
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem | Op::Pow => {
                    let (a, b) = (st[sp - 2], st[sp - 1]);
                    sp -= 1;
                    st[sp - 1] = match op {
                        Op::Add => a + b,
                        Op::Sub => a - b,
                        Op::Mul => a * b,
                        Op::Div => {
                            if b.abs() < 1e-9 {
                                0.0
                            } else {
                                a / b
                            }
                        }
                        Op::Rem => {
                            if b.abs() < 1e-9 {
                                0.0
                            } else {
                                a.rem_euclid(b)
                            }
                        }
                        _ => a.powf(b),
                    };
                }
                Op::Call(f, n) => {
                    let n = n as usize;
                    let args = &st[sp - n..sp];
                    let a = args[0];
                    let r = match FUNCS[f as usize].0 {
                        "sin" => a.sin(),
                        "cos" => a.cos(),
                        "tan" => a.tan(),
                        "abs" => a.abs(),
                        "sqrt" => a.max(0.0).sqrt(),
                        "floor" => a.floor(),
                        "ceil" => a.ceil(),
                        "fract" => a - a.floor(),
                        "sign" => {
                            if a > 0.0 {
                                1.0
                            } else if a < 0.0 {
                                -1.0
                            } else {
                                0.0
                            }
                        }
                        "exp" => a.clamp(-30.0, 30.0).exp(),
                        "ln" => a.max(1e-12).ln(),
                        "min" => a.min(args[1]),
                        "max" => a.max(args[1]),
                        "pow" => a.powf(args[1]),
                        "mod" => {
                            if args[1].abs() < 1e-9 {
                                0.0
                            } else {
                                a.rem_euclid(args[1])
                            }
                        }
                        "clamp" => {
                            let (lo, hi) = (args[1].min(args[2]), args[1].max(args[2]));
                            a.clamp(lo, hi)
                        }
                        "mix" => a + (args[1] - a) * args[2],
                        "tri" => 1.0 - 4.0 * ((a - a.floor()) - 0.5).abs(),
                        "saw" => (a - a.floor()) * 2.0 - 1.0,
                        "square" => {
                            if a - a.floor() < 0.5 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        "step" => {
                            if a >= args[1] {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        _ => {
                            // noise(x): a repeatable pseudo-random value per whole number of x
                            let mut h = (a.floor() as i32 as u32).wrapping_mul(2_654_435_761);
                            h ^= h >> 15;
                            h = h.wrapping_mul(2_246_822_519);
                            (h >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
                        }
                    };
                    sp -= n;
                    st[sp] = r;
                    sp += 1;
                }
            }
        }
        let v = st[0];
        if v.is_finite() {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, inp: &Inputs) -> f32 {
        Formula::compile(src).unwrap_or_else(|e| panic!("{src}: {e}")).eval(inp)
    }

    #[test]
    fn arithmetic_and_precedence() {
        let i = Inputs::default();
        assert_eq!(eval("1 + 2 * 3", &i), 1.0); // 7 clamps to 1
        assert!((eval("(1 + 2) * 0.1", &i) - 0.3).abs() < 1e-6);
        assert!((eval("-0.5 + 0.25", &i) + 0.25).abs() < 1e-6);
        assert!((eval("2 ^ 3 ^ 2 / 1000", &i) - 0.512).abs() < 1e-6, "right associative power");
        assert!((eval("-2 ^ 2 / 10", &i) + 0.4).abs() < 1e-6, "unary minus applies after the power");
        assert!((eval("7 % 4 / 10", &i) - 0.3).abs() < 1e-6);
        assert_eq!(eval("", &i), 0.0);
        assert_eq!(eval("1 / 0", &i), 0.0);
    }

    #[test]
    fn variables_and_functions() {
        let i = Inputs { t: 0.25, beat: 2.0, bpm: 120.0, knobs: [0.1, 0.2, 0.3, 0.4], note: 60.0, vel: 0.5 };
        assert!((eval("sin(t * pi * 2)", &i) - 1.0).abs() < 1e-5);
        assert!((eval("a + b + c + d", &i) - 1.0).abs() < 1e-6);
        assert!((eval("vel", &i) - 0.5).abs() < 1e-6);
        assert!((eval("(note - 48) / 24", &i) - 0.5).abs() < 1e-6);
        assert!((eval("min(0.3, max(0.1, 0.2))", &i) - 0.2).abs() < 1e-6);
        assert!((eval("clamp(5, 0, 0.7)", &i) - 0.7).abs() < 1e-6);
        assert!((eval("mix(0, 1, 0.25)", &i) - 0.25).abs() < 1e-6);
        assert!((eval("saw(beat / 4)", &i) - 0.0).abs() < 1e-6);
        assert_eq!(eval("square(0.25)", &i), 1.0);
        assert!(eval("noise(3)", &i).abs() <= 1.0);
    }

    #[test]
    fn mistakes_are_reported() {
        for (src, needle) in [
            ("1 +", "too early"),
            ("(1", "missing )"),
            ("1 2", "unexpected"),
            ("foo", "unknown name"),
            ("foo(1)", "unknown function"),
            ("sin(1, 2)", "takes 1"),
            ("min(1)", "takes 2"),
            ("1 $ 2", "unexpected"),
            ("sin(", "too early"),
        ] {
            let e = Formula::compile(src).unwrap_err();
            assert!(e.contains(needle), "{src}: {e}");
        }
        assert!(Formula::compile(&"(".repeat(100)).is_err());
        assert!(Formula::compile("1+".repeat(300).trim_end_matches('+')).is_err());
    }

    #[test]
    fn random_text_never_panics() {
        let mut seed = 3u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        let alphabet = b"abcdt0123456789.+-*/%^(),  sinmaxpow";
        for _ in 0..20_000 {
            let len = rnd() % 30;
            let s: String = (0..len).map(|_| alphabet[rnd() as usize % alphabet.len()] as char).collect();
            if let Ok(f) = Formula::compile(&s) {
                let v = f.eval(&Inputs {
                    t: 1e9,
                    beat: -5.0,
                    bpm: 0.0,
                    knobs: [f32::NAN, 2.0, -1.0, 0.0],
                    note: 255.0,
                    vel: 9.0,
                });
                assert!((-1.0..=1.0).contains(&v));
            }
        }
    }
}
