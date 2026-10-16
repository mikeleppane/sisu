//! Last use: turns each read of a counted local that is its last use into `ExprKind::Move`.
//! A read is a last use when, on every path from it, the local is not read again before it
//! is assigned or its scope ends.
//!
//! The walk goes backward through each function, in the order codegen evaluates nodes, with
//! the set of locals live after the current point. It must match codegen exactly:
//! - `Call`, `New`, `ArrayLit` and `Math` evaluate their arguments left to right, and
//!   `Interpolate` its parts; `Index` evaluates base then index; `Contains` base then value;
//!   `ArrayFill` value then length; `Equal`, `Is`, `Concat` and arithmetic lhs then rhs.
//! - `If` and `IfSome` evaluate the condition or scrutinee, then one branch.
//! - `Match` evaluates its scrutinee, then tests the arms' patterns in order and runs the body
//!   of the first that matches, its bindings defined. So the set live at an arm's test is
//!   what its body reads joined with what the next arm's test reads, its bindings killed. A
//!   guard runs after its arm's test, and goes on to the body or to the next arm's test; with
//!   an `|` in the pattern it can run again, for a later alternative, so it is a loop body.
//!   A guard never moves its arm's bindings, which the body or the next alternative still holds.
//! - Assigning a local evaluates the value, then kills the local; assigning a field evaluates
//!   the base, then the value.
//! - Assigning an element evaluates a field root's base, then the indexes from the root
//!   outwards, then the value, then uses the root local without moving it. An `Append` to a
//!   field or an element evaluates as the assignment to it does.
//! - `Return` and `Panic` leave nothing live, `Break` takes the loop's exit set, `Continue`
//!   its head set.
//! - `&&`, `||` and each step of a comparison chain have an edge that skips the rest.

use std::mem;

use rustc_hash::FxHashMap;

use crate::tir::{
    Arm, BinaryOp, Block, Expr, ExprKind, LocalId, Pattern, PatternKind, Place, Program, Stmt,
};

/// Rewrites each read of a counted local that is its last use into `ExprKind::Move`.
pub(crate) fn mark(program: &mut Program) {
    for function in &mut program.functions {
        let mut walk = Walk {
            counted: function
                .locals
                .iter()
                .map(|l| l.ty.is_counted(&program.enums))
                .collect(),
            live: vec![false; function.locals.len()],
            pinned: vec![false; function.locals.len()],
            rewrite: true,
            loops: Vec::new(),
            trail: Vec::new(),
            undoing: false,
        };
        walk.block(&mut function.body);
    }
}

/// A backward liveness walk over one function.
struct Walk {
    /// Indexed by `LocalId`: whether the local is counted, so that its last read moves.
    counted: Vec<bool>,
    /// Indexed by `LocalId`: whether the local is read later on some path.
    live: Vec<bool>,
    /// Indexed by `LocalId`: whether the local is a binding of the arm whose guard is being
    /// walked, which no read moves.
    pinned: Vec<bool>,
    /// Whether the walk rewrites last reads; a loop's first pass only computes its head set.
    rewrite: bool,
    /// The enclosing loops, innermost last: the set live after each, and at its head.
    loops: Vec<(Vec<bool>, Vec<bool>)>,
    /// Inside a `match`'s arms: each write to `live`, as the local and the value it had, so that
    /// an arm is undone in the locals it wrote rather than in every local.
    trail: Vec<(usize, bool)>,
    /// Whether the walk is inside a `match`'s arms; outside them nothing is undone, so the trail
    /// records nothing.
    undoing: bool,
}

impl Walk {
    fn block(&mut self, block: &mut Block) {
        if let Some(value) = &mut block.value {
            self.expr(value);
        }
        for stmt in block.stmts.iter_mut().rev() {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Let { local, init } => {
                self.set(local.0, false);
                self.expr(init);
            }
            // Loading the old value to release it is not a read.
            Stmt::Assign {
                place: Place::Local(local),
                value,
            } => {
                self.set(local.0, false);
                self.expr(value);
            }
            Stmt::Assign { place, value } | Stmt::Append { place, value } => {
                // An element write loads its root local's slot after the value, and keeps it.
                if let Some(root) = root_local(place) {
                    self.set(root.0, true);
                }
                self.expr(value);
                self.place(place);
            }
            Stmt::Expr(e) => self.expr(e),
        }
    }

    /// A field or element place's operands, last evaluated first: the indexes from the
    /// outermost in, then a field root's base.
    fn place(&mut self, place: &mut Place) {
        match place {
            Place::Local(_) => {}
            Place::Field { base, .. } => self.expr(base),
            Place::Index { base, index } => {
                self.expr(index);
                self.place(base);
            }
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per node kind, so the evaluation order reads in one place"
    )]
    fn expr(&mut self, e: &mut Expr) {
        if let ExprKind::Local(local) | ExprKind::Move(local) = e.kind {
            if self.rewrite && self.counted[local.0] && !self.live[local.0] && !self.pinned[local.0]
            {
                e.kind = ExprKind::Move(local);
            }
            self.set(local.0, true);
            return;
        }
        match &mut e.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Char(_)
            | ExprKind::Str(_)
            | ExprKind::None
            | ExprKind::ReadLine
            | ExprKind::Local(_)
            | ExprKind::Move(_) => {}
            ExprKind::Call { args, .. }
            | ExprKind::New { args, .. }
            | ExprKind::Variant { args, .. }
            | ExprKind::ArrayLit(args)
            | ExprKind::Interpolate { parts: args, .. }
            | ExprKind::Math { operands: args, .. } => {
                for arg in args.iter_mut().rev() {
                    self.expr(arg);
                }
            }
            ExprKind::Field { base: operand, .. }
            | ExprKind::Len(operand)
            | ExprKind::Chars(operand)
            | ExprKind::Bytes(operand)
            | ExprKind::Words(operand)
            | ExprKind::FromChars(operand)
            | ExprKind::Sum(operand)
            | ExprKind::Wrap(operand)
            | ExprKind::Convert(operand)
            | ExprKind::Print { operand, .. }
            | ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary {
                op: BinaryOp::And | BinaryOp::Or,
                lhs,
                rhs,
            } => {
                let after = self.live.clone();
                self.expr(rhs);
                self.join(&after);
                self.expr(lhs);
            }
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Concat { lhs, rhs }
            | ExprKind::Equal { lhs, rhs, .. }
            | ExprKind::Is { lhs, rhs }
            | ExprKind::ArrayFill {
                value: lhs,
                len: rhs,
            }
            | ExprKind::Index {
                base: lhs,
                index: rhs,
            }
            | ExprKind::Contains {
                base: lhs,
                value: rhs,
            } => {
                self.expr(rhs);
                self.expr(lhs);
            }
            ExprKind::Compare { operands, .. } => self.compare(operands),
            ExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.branches(then_block, None, else_block.as_mut());
                self.expr(cond);
            }
            ExprKind::IfSome {
                bind,
                scrutinee,
                then_block,
                else_block,
            } => {
                self.branches(then_block, Some(*bind), else_block.as_mut());
                self.expr(scrutinee);
            }
            ExprKind::Match {
                scrutinee, arms, ..
            } => {
                self.arms(arms);
                self.expr(scrutinee);
            }
            ExprKind::Loop(body) => self.looped(body),
            ExprKind::Break => {
                let (exit, _) = self
                    .loops
                    .last()
                    .expect("the checker should reject `break` outside a loop")
                    .clone();
                self.assign(&exit);
            }
            ExprKind::Continue => {
                let (_, head) = self
                    .loops
                    .last()
                    .expect("the checker should reject `continue` outside a loop")
                    .clone();
                self.assign(&head);
            }
            ExprKind::Return(value) => {
                self.clear();
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            ExprKind::Panic(message) => {
                self.clear();
                self.expr(message);
            }
            ExprKind::Block(block) => self.block(block),
        }
    }

    /// A comparison chain. After each operand but the first, a failed pair skips to the
    /// chain's end.
    fn compare(&mut self, operands: &mut [Expr]) {
        let after = self.live.clone();
        let (first, rest) = operands
            .split_first_mut()
            .expect("a comparison chain should have at least two operands");
        for operand in rest.iter_mut().rev() {
            self.join(&after);
            self.expr(operand);
        }
        self.expr(first);
    }

    /// Both branches from the set live after them, joined. `bind` is an `IfSome`'s binding,
    /// which the then block starts by defining.
    fn branches(
        &mut self,
        then_block: &mut Block,
        bind: Option<LocalId>,
        else_block: Option<&mut Block>,
    ) {
        let after = self.live.clone();
        self.block(then_block);
        if let Some(bind) = bind {
            self.set(bind.0, false);
        }
        let then_live = self.live.clone();
        self.assign(&after);
        if let Some(else_block) = else_block {
            self.block(else_block);
        }
        self.join(&then_live);
    }

    /// A loop's head set is the fixed point of walking its body from that set. Liveness is a
    /// gen/kill problem, so walking once from the empty set already reaches it. Rewriting then
    /// takes a second walk from the head set. Each call walks its body at most twice; a loop
    /// at depth d is walked d + 1 times in all, since only the outermost pass rewrites twice.
    fn looped(&mut self, body: &mut Block) {
        let exit = self.live.clone();
        let rewrite = mem::replace(&mut self.rewrite, false);
        let head = self.pass(body, exit.clone(), vec![false; self.live.len()]);
        self.rewrite = rewrite;
        if rewrite {
            self.pass(body, exit, head);
        }
    }

    /// Walks `body` with the given exit and head sets; the body falls through to its head.
    /// Leaves the set live at the head.
    fn pass(&mut self, body: &mut Block, exit: Vec<bool>, head: Vec<bool>) -> Vec<bool> {
        self.assign(&head);
        self.loops.push((exit, head));
        self.block(body);
        self.loops.pop();
        self.live.clone()
    }

    /// A `match`'s arms, from the set live after them. Each body is walked from that set, and
    /// the set live at the tests is their union, the bindings killed. Each arm is undone through
    /// the trail, so an arm costs the locals it changes, not every local of the function. The
    /// arms go last to first, so a guard's failure edge, the next arm's test, holds the union
    /// of the later arms: the set after the `match`, changed only where `changed` says.
    fn arms(&mut self, arms: &mut [Arm]) {
        let mark = self.trail.len();
        let outer = mem::replace(&mut self.undoing, true);
        // Each local an arm wrote: the last arm that did, how many did, and whether one left it
        // live, its bindings left out.
        let mut changed: FxHashMap<usize, (usize, usize, bool)> = FxHashMap::default();
        // The locals some later arm leaves live at its test where the set after the `match`
        // has them dead, or that an arm wrote: the sparse part of the failure edge's set.
        let mut later = Vec::new();
        let count = arms.len();
        for (i, arm) in arms.iter_mut().enumerate().rev() {
            self.block(&mut arm.body);
            for bind in &arm.bindings {
                self.pinned[bind.0] = true;
            }
            if let Some(guard) = &mut arm.guard {
                let done = count - 1 - i;
                // Whether a local this arm wrote, `after` after the `match`, is live at the next
                // arm's test: some later arm left it live, or left it alone. A guarded arm is
                // never last, since a guard covers nothing, so a later arm exists.
                let failing = |local, after| match changed.get(&local) {
                    Some(&(_, wrote, live)) => live || (after && wrote < done),
                    None => after,
                };
                let retries = has_or(&arm.pattern);
                self.guard(guard, retries, mark, &later, failing);
            }
            // Newest first, so the first entry of a local finds the arm's final value in `live`.
            for (local, was) in self.trail.drain(mark..).rev() {
                let now = mem::replace(&mut self.live[local], was);
                if self.pinned[local] {
                    continue;
                }
                let (last, count, live) = changed.entry(local).or_insert((usize::MAX, 0, false));
                if *last != i {
                    *last = i;
                    *count += 1;
                    if now && !*live {
                        later.push(local);
                    }
                    *live |= now;
                }
            }
            for bind in &arm.bindings {
                self.pinned[bind.0] = false;
            }
        }
        self.undoing = outer;
        for (local, (_, count, live)) in changed {
            // An arm that left the local alone kept the value it has after the `match`.
            let kept = count < arms.len() && self.live[local];
            self.set(local, live || kept);
        }
        for arm in arms {
            for bind in &arm.bindings {
                self.set(bind.0, false);
            }
        }
    }

    /// A guard, after its arm's body is walked from the set after the `match`. It runs on to
    /// the body or to the next arm's test, so it is walked from both sets joined: the body's,
    /// which the trail from `mark` holds, and the next test's, which is `later` and each
    /// local of the trail that `failing` says, given whether it was live after the `match`.
    /// When it `retries`, for a later alternative, its entry set joins its exit set too.
    fn guard(
        &mut self,
        guard: &mut Expr,
        retries: bool,
        mark: usize,
        later: &[usize],
        failing: impl Fn(usize, bool) -> bool,
    ) {
        // The oldest entry of a local holds its value after the `match`.
        let mut after = FxHashMap::default();
        for &(local, was) in &self.trail[mark..] {
            after.entry(local).or_insert(was);
        }
        for (local, was) in after {
            if !self.live[local] && failing(local, was) {
                self.set(local, true);
            }
        }
        for &local in later {
            if !self.live[local] {
                self.set(local, true);
            }
        }
        // shortcut: a guard that compares, or holds `&&`, `||` or an `if`, copies the dense
        // live set as any such expression does (`.scratch/diagnostics-scale/issues/02`), so
        // many such guards among many locals are quadratic; fix it with that ticket.
        if !retries {
            self.expr(guard);
            return;
        }
        // As a loop body: one walk without rewriting finds the entry set, the fixed point,
        // since liveness is gen/kill; the rewriting walk starts from it joined with the exit
        // set. No frame goes on `loops`, so a `break` in the guard leaves the source loop.
        let start = self.trail.len();
        let rewrite = mem::replace(&mut self.rewrite, false);
        self.expr(guard);
        self.rewrite = rewrite;
        if !rewrite {
            return;
        }
        let entry: Vec<_> = self.trail[start..]
            .iter()
            .map(|&(local, _)| local)
            .filter(|&local| self.live[local])
            .collect();
        for (local, was) in self.trail.drain(start..).rev() {
            self.live[local] = was;
        }
        for local in entry {
            if !self.live[local] {
                self.set(local, true);
            }
        }
        self.expr(guard);
    }

    /// Sets whether `local` is live, recording the write while an arm may be undone.
    fn set(&mut self, local: usize, live: bool) {
        let was = mem::replace(&mut self.live[local], live);
        if self.undoing {
            self.trail.push((local, was));
        }
    }

    /// Makes `other` the live set. Outside an arm nothing is recorded, so it copies; inside
    /// one it records only the locals it changes.
    fn assign(&mut self, other: &[bool]) {
        if !self.undoing {
            self.live.copy_from_slice(other);
            return;
        }
        for (local, &live) in other.iter().enumerate() {
            if self.live[local] != live {
                self.set(local, live);
            }
        }
    }

    /// Adds `other`, a set the walk held earlier in the same arm, to the live set. It records
    /// nothing: each local it makes live again was written since `other` was taken, and the
    /// trail holds that write, whose undo restores the local.
    fn join(&mut self, other: &[bool]) {
        for (live, other) in self.live.iter_mut().zip(other) {
            *live |= *other;
        }
    }

    /// Empties the live set. Outside an arm nothing is recorded, so it fills; inside one it
    /// records only the locals it changes.
    fn clear(&mut self) {
        if !self.undoing {
            self.live.fill(false);
            return;
        }
        for local in 0..self.live.len() {
            if self.live[local] {
                self.set(local, false);
            }
        }
    }
}

/// Whether `pattern` holds an `|`, so its arm's guard can run once per alternative.
fn has_or(pattern: &Pattern) -> bool {
    match &pattern.kind {
        PatternKind::Or(_) => true,
        PatternKind::Variant { fields, .. } => fields.iter().any(has_or),
        _ => false,
    }
}

/// The local an element place is rooted at; `None` for a field root, which is not tracked.
fn root_local(place: &Place) -> Option<LocalId> {
    match place {
        Place::Local(local) => Some(*local),
        Place::Field { .. } => None,
        Place::Index { base, .. } => root_local(base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::parser::parse_ok;

    /// Checks `src`, with an empty `main` added when it has none, marks it, and prints it.
    fn marked(src: &str) -> String {
        let src = if src.contains("fn main") {
            src.to_owned()
        } else {
            format!("{src}\nfn main() {{}}")
        };
        let ast = parse_ok(&src);
        let (program, diagnostics) = check(&ast, &src);
        let mut program = program.unwrap_or_else(|| panic!("{diagnostics:?}"));
        mark(&mut program);
        program.to_string()
    }

    /// The printed function `name` of a printed program: one function per line.
    fn function<'a>(printed: &'a str, name: &str) -> &'a str {
        let head = format!("(fn {name} (");
        printed
            .lines()
            .find(|line| line.starts_with(&head))
            .unwrap_or_else(|| panic!("no function {name} in {printed}"))
    }

    /// Asserts that function `name` of `src`, marked, contains each of `parts`.
    fn has(src: &str, name: &str, parts: &[&str]) {
        let printed = marked(src);
        let f = function(&printed, name);
        for part in parts {
            assert!(f.contains(part), "{part} in {f}");
        }
    }

    #[test]
    fn assign_from_own_call() {
        let src = "fn f(x: [i64]) -> [i64] { x }\nfn main() {\n    let mut x = [1]\n    x = f(x)\n    print(x[0])\n}";
        has(
            src,
            "main",
            &["(= x#0 (call f (move x#0)))", "(index (move x#0) 0)"],
        );
        has(src, "f", &["(move x#0)"]);
    }

    #[test]
    fn two_reads_in_one_call() {
        has(
            "fn g(a: [i64], b: [i64]) {}\nfn f(x: [i64]) { g(x, x) }",
            "f",
            &["(call g x#0 (move x#0))"],
        );
    }

    /// `Contains` evaluates its base, then its value: only the value's read may move.
    #[test]
    fn contains_base_then_value() {
        has(
            "fn f(xs: [[i64]]) -> bool { xs.contains(xs[0]) }",
            "f",
            &["(contains xs#0 (index (move xs#0) 0))"],
        );
    }

    /// A variant's arguments are evaluated left to right: only the last one's read may move.
    #[test]
    fn variant_args_left_to_right() {
        has(
            "enum E { V(string, string) }\nfn f(s: string) -> E { E.V(s, s) }",
            "f",
            &["(variant E.V s#0 (move s#0))"],
        );
    }

    /// `Interpolate` evaluates its parts left to right: only the last one's read may move.
    #[test]
    fn interpolate_parts_left_to_right() {
        has(
            "fn f(s: string) -> string { \"{s}{s}\" }",
            "f",
            &["(interp s#0 (move s#0))"],
        );
    }

    /// `Concat` evaluates its left operand, then its right: only the right one's read may move.
    #[test]
    fn concat_lhs_then_rhs() {
        has(
            "fn f(s: string) -> string { s + s }",
            "f",
            &["(concat s#0 (move s#0))"],
        );
    }

    /// A comparison chain evaluates its operands in order, so only the last read moves.
    #[test]
    fn comparison_chain_reads() {
        has(
            "fn f(xs: [i64]) -> bool { 0 < xs.len() }",
            "f",
            &["(len (move xs#0))"],
        );
        has(
            "fn f(xs: [i64]) -> bool { 0 < xs.len() < xs.len() }",
            "f",
            &["(< 0 (len xs#0) < (len (move xs#0)))"],
        );
    }

    #[test]
    fn read_in_loop_body() {
        let printed =
            marked("fn f(xs: [i64], c: bool) {\n    while c {\n        print(xs.len())\n    }\n}");
        assert!(!printed.contains("(move xs"), "{printed}");
    }

    #[test]
    fn read_before_return() {
        let printed = marked(
            "fn f(xs: [i64], c: bool) -> i64 {\n    if c {\n        return xs.len()\n    }\n    xs.len()\n}",
        );
        assert_eq!(printed.matches("(move xs#0)").count(), 2, "{printed}");
    }

    #[test]
    fn read_before_panic() {
        let printed = marked(
            "fn f(xs: [i64], c: bool) -> i64 {\n    if c {\n        panic(\"{xs.len()}\")\n    }\n    xs.len()\n}",
        );
        assert_eq!(printed.matches("(move xs#0)").count(), 2, "{printed}");
    }

    #[test]
    fn read_in_branch_then_after() {
        let printed = marked(
            "fn f(xs: [i64], c: bool) -> i64 {\n    if c {\n        print(xs.len())\n    }\n    xs.len()\n}",
        );
        let f = function(&printed, "f");
        assert_eq!(f.matches("(move xs#0)").count(), 1, "{f}");
        assert!(f.ends_with("(len (move xs#0))))"), "{f}");
    }

    #[test]
    fn read_after_continue() {
        has(
            "fn f(c: bool) {\n    let mut i = 0\n    while i < 3 {\n        i += 1\n        let ys = [i]\n        if c {\n            continue\n        }\n        print(ys.len())\n    }\n}",
            "f",
            &["(len (move ys#"],
        );
    }

    /// An element write uses its root after the value and never moves it, so no read before
    /// it is a last use.
    #[test]
    fn element_write_roots() {
        let printed = marked(
            "fn shared() {\n    let mut a = [1, 2]\n    let b = a\n    a[0] = 9\n}\n\
             fn compound() {\n    let mut xs = [1]\n    xs[0] += 1\n}\n\
             fn during_for() {\n    let mut xs = [1]\n    for x in xs { xs[0] = 1 }\n}\n\
             fn reads_len() {\n    let mut xs = [1]\n    xs[0] = xs.len()\n}\n\
             fn g(v: [i64]) -> i64 { 0 }\n\
             fn passes() {\n    let mut xs = [1]\n    xs[0] = g(xs)\n}",
        );
        assert!(!printed.contains("(move a"), "{printed}");
        assert!(!printed.contains("(move xs"), "{printed}");
    }

    /// An assignment in the operand a short circuit skips does not end liveness on the path
    /// that skips it.
    #[test]
    fn short_circuits() {
        let printed = marked(
            "fn or() {\n    let mut xs = [1]\n    let ok = xs.len() > 0 || if true {\n        xs = [2]\n        true\n    } else {\n        xs = [3]\n        true\n    }\n    print(xs[0])\n}\n\
             fn and() {\n    let mut xs = [1]\n    let ok = xs.len() > 0 && if true {\n        xs = [2]\n        true\n    } else {\n        xs = [3]\n        true\n    }\n    print(xs[0])\n}\n\
             fn chain() {\n    let mut xs = [1]\n    let ok = 0 < xs.len() < if true {\n        xs = [2]\n        5\n    } else {\n        xs = [3]\n        5\n    }\n    print(xs[0])\n}",
        );
        for name in ["or", "and", "chain"] {
            let f = function(&printed, name);
            assert!(f.contains("(len xs#0)"), "{f}");
        }
    }

    #[test]
    fn objects_self_and_if_let() {
        let src = "class N {\n    v: i64\n    fn get(self) -> i64 { self.v }\n}\nfn f(m: N?) -> i64 {\n    if let n = m { n.v } else { 0 }\n}";
        has(src, "N.get", &["(. (move self#0) v)"]);
        has(src, "f", &["(. (move n#1) v)"]);
    }

    /// A field assignment's base, a field root's base and an element write's index are reads
    /// like any other: the last of them moves.
    #[test]
    fn place_operands() {
        let src = "class O {\n    mut n: i64\n}\nclass B {\n    mut xs: [i64]\n}\n\
             fn field(o: O) {\n    print(o.n)\n    o.n = 1\n}\n\
             fn root(b: B) {\n    print(b.xs.len())\n    b.xs[0] = 1\n}\n\
             fn index(xs: [i64]) {\n    let mut ys = [1, 2]\n    print(xs.len())\n    ys[xs.len() - 1] = 5\n}";
        has(
            src,
            "field",
            &["(print (. o#0 n))", "(= (. (move o#0) n) 1)"],
        );
        has(
            src,
            "root",
            &[
                "(print (len (. b#0 xs)))",
                "(= (index (. (move b#0) xs) 0) 1)",
            ],
        );
        has(
            src,
            "index",
            &[
                "(print (len xs#0))",
                "(= (index ys#1 (- (len (move xs#0)) 1)) 5)",
            ],
        );
    }

    /// An `Append` evaluates as the assignment to its place: a field root's base before the
    /// value, and an element's root local after the value, never moved.
    #[test]
    fn append_order() {
        let src = "class P {\n    mut name: string\n}\nfn field(p: P) {\n    p.name = p.name + p.name\n}\nfn element() {\n    let mut xs = [\"a\"]\n    xs[0] = xs[0] + xs[0]\n}";
        has(src, "field", &["(append (. p#0 name) (. (move p#0) name))"]);
        has(src, "element", &["(append (index xs#0 0) (index xs#0 0))"]);
    }

    /// `break` leaves the loop with the set live after it, so a read before the loop stays live.
    #[test]
    fn read_after_break() {
        let printed = marked(
            "fn f(xs: [i64], c: bool) {\n    print(xs.len())\n    while c {\n        break\n    }\n    print(xs.len())\n}",
        );
        let f = function(&printed, "f");
        assert_eq!(f.matches("(move xs#0)").count(), 1, "{f}");
        assert!(f.ends_with("(print (len (move xs#0)))))"), "{f}");
    }

    /// A read in an inner loop stays live into both loops' next iterations.
    #[test]
    fn nested_loops() {
        let printed = marked(
            "fn f(xs: [i64], c: bool) {\n    while c {\n        while c {\n            print(xs.len())\n        }\n    }\n    print(xs.len())\n}",
        );
        let f = function(&printed, "f");
        assert_eq!(f.matches("(move xs#0)").count(), 1, "{f}");
        assert!(f.ends_with("(print (len (move xs#0)))))"), "{f}");
    }

    /// A last use inside an inner loop moves: rewriting reaches every loop depth.
    #[test]
    fn move_in_inner_loop() {
        has(
            "fn f(xs: [i64], c: bool) {\n    while c {\n        while c {\n            print(xs.len())\n            return\n        }\n    }\n}",
            "f",
            &["(len (move xs#0))"],
        );
    }

    /// A guard's reads: a binding the guard reads is pinned, a guard that can run once per
    /// alternative is a loop body, and its `break` or `continue` targets the source loop.
    #[test]
    fn guard_reads() {
        // `s` is the arm's binding: the guard's `return s` retains it.
        has(
            "enum N {\n    Name(string)\n    Other\n}\nfn f(n: N) -> string {\n    match n {\n        Name(s) if (if s == \"\" { return s } else { true }) => s\n        _ => \"x\"\n    }\n}",
            "f",
            &["(block (return s#1))"],
        );
        // An outer `t`, read only in a guard: it can run again with an `Or`, so it never moves;
        // without one, it moves when no later arm reads it.
        let decls =
            "enum E {\n    V(i64)\n    W(i64)\n    Z\n}\nfn keep(t: string) -> bool { true }\n";
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> i64 {{\n    match e {{\n        V(_) | W(_) if keep(t) => 1\n        _ => 0\n    }}\n}}"
            ),
            "f",
            &["(if (call keep t#1))"],
        );
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> i64 {{\n    match e {{\n        V(_) if keep(t) => 1\n        _ => 0\n    }}\n}}"
            ),
            "f",
            &["(if (call keep (move t#1)))"],
        );
        // A later arm that reads `t` keeps it live on the guard's failure edge.
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> i64 {{\n    match e {{\n        V(_) if keep(t) => 1\n        _ => if keep(t) {{ 2 }} else {{ 0 }}\n    }}\n}}"
            ),
            "f",
            &["(if (call keep t#1))", "(call keep (move t#1))"],
        );
        // The first arm writes `u`, but the guard's failure edge reaches the last arm, which
        // leaves `u` alone for the read after the `match`; the second arm also writes it.
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> string {{\n    let mut u = t\n    let _ = match e {{\n        V(_) if keep(u) => {{\n            u = \"y\"\n            1\n        }}\n        W(_) => {{\n            u = \"z\"\n            2\n        }}\n        _ => 0\n    }}\n    u\n}}"
            ),
            "f",
            &["(if (call keep u#2))"],
        );
        // When every later arm writes `u`, the guard's read is its last.
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> string {{\n    let mut u = t\n    let _ = match e {{\n        V(_) if keep(u) => {{\n            u = \"y\"\n            1\n        }}\n        _ => {{\n            u = \"z\"\n            0\n        }}\n    }}\n    u\n}}"
            ),
            "f",
            &["(if (call keep (move u#2)))"],
        );
        // An `|` inside a variant runs the guard again too.
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> i64 {{\n    match e {{\n        V(1 | 2) if keep(t) => 1\n        _ => 0\n    }}\n}}"
            ),
            "f",
            &["(if (call keep t#1))"],
        );
        // A `return` leaves nothing live, even in a guard that runs again.
        has(
            &format!(
                "{decls}fn f(e: E, c: bool, t: string) -> string {{\n    match e {{\n        V(_) | W(_) if (if c {{ return t }} else {{ true }}) => \"a\"\n        _ => \"b\"\n    }}\n}}"
            ),
            "f",
            &["(return (move t#2))"],
        );
        // The guard's body reads `t` as well, so `t` is live at the guard already.
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> i64 {{\n    match e {{\n        V(_) if keep(t) => if keep(t) {{ 1 }} else {{ 2 }}\n        _ => if keep(t) {{ 3 }} else {{ 0 }}\n    }}\n}}"
            ),
            "f",
            &["(if (call keep t#1))"],
        );
        // Only the guarded arm writes `u`, so the failure edge, through arms that leave `u`
        // alone, keeps it live for the read after the `match`.
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> string {{\n    let mut u = t\n    let _ = match e {{\n        V(_) if keep(u) => {{\n            u = \"y\"\n            1\n        }}\n        _ => 0\n    }}\n    u\n}}"
            ),
            "f",
            &["(if (call keep u#2))"],
        );
        // Every arm after the guarded one writes `u`, so an arm before it changes nothing.
        has(
            &format!(
                "{decls}fn f(e: E, t: string) -> string {{\n    let mut u = t\n    let _ = match e {{\n        Z => 2\n        V(_) if keep(u) => {{\n            u = \"y\"\n            1\n        }}\n        _ => {{\n            u = \"z\"\n            0\n        }}\n    }}\n    u\n}}"
            ),
            "f",
            &["(if (call keep (move u#2)))"],
        );
        // `break` and `continue` in a guard reach the `while`, whose exit and head read `t`;
        // every other path from the first `print(t)` returns, so only the guard's jump keeps
        // `t` live there.
        for jump in ["break", "continue"] {
            has(
                &format!(
                    "enum AB {{\n    A\n    B\n    C\n}}\nfn f(e: AB, c: bool, t: string) {{\n    while c {{\n        print(t)\n        let _ = match e {{\n            A | B if (if c {{ {jump} }} else {{ true }}) => 0\n            _ => 1\n        }}\n        return\n    }}\n    print(t)\n}}"
                ),
                "f",
                &["(loop (block (if c#1 (block (print t#2)"],
            );
        }
    }

    /// The keep-alive override for this test is in `.config/nextest.toml`: a set live on each
    /// guard's failure edge copied densely per arm is quadratic in the arms, and this test hangs
    /// instead of failing.
    #[test]
    fn many_guarded_arms_mark_last_uses_in_linear_time() {
        const VARIANTS: usize = 32_000;
        let variants = (0..VARIANTS)
            .map(|i| format!("    V{i}(string)\n"))
            .collect::<Vec<_>>()
            .concat();
        let arms = (0..VARIANTS)
            .map(|i| format!("        V{i}(s) if keep => s\n"))
            .collect::<Vec<_>>()
            .concat();
        let printed = marked(&format!(
            "enum E {{\n{variants}}}\nfn f(e: E, keep: bool) -> string {{\n    match e {{\n{arms}        _ => \"\"\n    }}\n}}"
        ));
        // Each arm's `s` is read once, in its body, so every read is its last use.
        assert_eq!(printed.matches("(block (move s#").count(), VARIANTS);
    }

    #[test]
    fn scalars_never_move() {
        let printed = marked("fn f(n: i64) -> i64 { n }");
        assert!(!printed.contains("(move"), "{printed}");
    }
}
