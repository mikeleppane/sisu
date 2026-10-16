//! Exhaustiveness and reachability of `match`: Maranget's usefulness algorithm ("Warnings for
//! pattern matching", 2007), run once over a matrix of every arm's row plus a last wildcard row,
//! the sentinel. A value reaches the sentinel exactly when it reaches no arm.
//!
//! Specializing a column puts each row in the bucket of its head variant; a wildcard row goes
//! into every bucket and into the default matrix, which stands for every variant no row names.
//! A matrix with no column left is a leaf: its first row takes its values, and the rows after it
//! are not reached there. A matrix whose first row has only wildcards left is cut short the same
//! way. Rows are hash-consed stacks of (pattern, column type), so a matrix is its rows' stack ids,
//! and a matrix met twice is solved once.
//!
//! shortcut: usefulness can meet exponentially many distinct matrices when payload patterns are
//! correlated across fields (arm i needs fields `x_i` and `y_i`, with every x before every y);
//! deciding exhaustiveness is co-NP-complete in general, so no algorithm avoids it; revisit only
//! if a real program hits it.

use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::diagnostic::Span;
use crate::tir::{self, Type};

/// One arm's pattern: a row of the matrix.
pub(super) struct Row<'a> {
    pub(super) pattern: &'a tir::Pattern,
}

/// What `check` found.
pub(super) struct Report {
    /// A value no row matches, when there is one.
    pub(super) missing: Option<Missing>,
    /// Each row that no value reaches, in row order.
    pub(super) unreachable: Vec<Unreachable>,
}

/// A value no row matches.
pub(super) struct Missing {
    /// The value, written as a pattern: `Mul(_, _)`.
    pub(super) pattern: String,
    /// The first row with the value's variant at the top: its span, its pattern, and its
    /// payload, which is all of that variant it covers.
    pub(super) partly: Option<(Span, String, String)>,
}

/// A row that no value reaches.
pub(super) struct Unreachable {
    /// The row's pattern.
    pub(super) at: Span,
    /// The pattern of the row that takes every value of this one, when one row takes them all.
    pub(super) covered_by: Option<Span>,
}

/// Which `rows`, the arms of a `match` on `ty` in order, no value reaches, and a value that
/// none matches. `enums` are the program's.
pub(super) fn check(enums: &[tir::Enum], ty: &Type, rows: &[Row<'_>]) -> Report {
    let mut solver = Solver {
        enums,
        pats: vec![Pat::Wild],
        pat_ids: FxHashMap::default(),
        nodes: vec![Node {
            pat: WILD,
            col: None,
            tail: EMPTY,
            wild: true,
        }],
        node_ids: FxHashMap::default(),
        memo: FxHashMap::default(),
    };
    let col = column(ty);
    let pats: Vec<_> = rows
        .iter()
        .map(|row| solver.intern(row.pattern, col))
        .collect();
    let mut stacks: Vec<_> = pats
        .iter()
        .map(|&pat| solver.push(pat, col, EMPTY))
        .collect();
    stacks.push(solver.push(WILD, col, EMPTY));
    let result = solver.solve(&stacks);
    let unreachable = rows
        .iter()
        .zip(&result.reach)
        .filter_map(|(row, reach)| {
            let covered_by = match *reach {
                Reach::Reached => return None,
                Reach::Taken(by) => Some(rows[by].pattern.span),
                Reach::Several => None,
            };
            Some(Unreachable {
                at: row.pattern.span,
                covered_by,
            })
        })
        .collect();
    let missing = result.witness.as_ref().map(|witness| {
        let top = witness[0];
        // With a value missing, no arm is a wildcard: one would match every value.
        let partly = solver.variant(top).and_then(|(missing, _)| {
            rows.iter()
                .zip(&pats)
                .find_map(|(row, &pat)| match solver.variant(pat) {
                    Some((variant, fields)) if variant == missing => Some((
                        row.pattern.span,
                        solver.print(pat),
                        solver.print_all(fields),
                    )),
                    _ => None,
                })
        });
        Missing {
            pattern: solver.print(top),
            partly,
        }
    });
    Report {
        missing,
        unreachable,
    }
}

/// An interned pattern, its bindings forgotten: an index into `Solver::pats`.
type PatId = usize;

/// The interned `_`, which a binding is too.
const WILD: PatId = 0;

/// A stack of (pattern, column type) pairs, a row's columns from the first: an index into
/// `Solver::nodes`.
type StackId = usize;

/// The stack with no column.
const EMPTY: StackId = 0;

/// The type of a column as coverage sees it: its enum, or `None` for a type whose values no
/// pattern names.
type Col = Option<usize>;

fn column(ty: &Type) -> Col {
    match ty {
        Type::Enum(id) => Some(id.0),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Pat {
    Wild,
    Variant {
        enum_id: usize,
        variant: usize,
        fields: Box<[PatId]>,
    },
}

/// A stack's top: its first column's pattern and type, and the stack below. `wild` is whether
/// every pattern in the stack is `_`.
#[derive(Clone, Copy)]
struct Node {
    pat: PatId,
    col: Col,
    tail: StackId,
    wild: bool,
}

/// Where one row of a matrix stands, from the leaves below it.
#[derive(Clone, Copy)]
enum Reach {
    /// A leaf reached it.
    Reached,
    /// No leaf reached it, and this row, an earlier one, took every leaf's values.
    Taken(usize),
    /// No leaf reached it, and more than one row took the leaves' values.
    Several,
}

impl Reach {
    /// What `self` and `other`, from two sets of leaves, say together.
    fn merge(self, other: Reach) -> Reach {
        match (self, other) {
            (Reach::Reached, _) | (_, Reach::Reached) => Reach::Reached,
            (Reach::Taken(a), Reach::Taken(b)) if a == b => Reach::Taken(a),
            _ => Reach::Several,
        }
    }
}

/// What a matrix's leaves say, by the matrix's own row indices.
struct Solved {
    reach: Vec<Reach>,
    /// A value that only the last row, the sentinel, matches: one pattern per column.
    witness: Option<Vec<PatId>>,
}

struct Solver<'a> {
    enums: &'a [tir::Enum],
    pats: Vec<Pat>,
    pat_ids: FxHashMap<Pat, PatId>,
    nodes: Vec<Node>,
    node_ids: FxHashMap<(PatId, Col, StackId), StackId>,
    /// Each matrix solved so far, by its rows.
    memo: FxHashMap<Box<[StackId]>, Rc<Solved>>,
}

impl Solver<'_> {
    fn pat(&mut self, pat: Pat) -> PatId {
        let pats = &mut self.pats;
        *self.pat_ids.entry(pat).or_insert_with_key(|pat| {
            pats.push(pat.clone());
            pats.len() - 1
        })
    }

    /// `p`, at a column of type `col`.
    fn intern(&mut self, p: &tir::Pattern, col: Col) -> PatId {
        match &p.kind {
            tir::PatternKind::Wild | tir::PatternKind::Bind(_) => WILD,
            tir::PatternKind::Variant { variant, fields } => {
                let enum_id = col.expect("a variant pattern should sit at an enum");
                let enums = self.enums;
                let payload = &enums[enum_id].variants[*variant].fields;
                let fields = fields
                    .iter()
                    .zip(payload)
                    .map(|(field, payload)| self.intern(field, column(&payload.ty)))
                    .collect();
                self.pat(Pat::Variant {
                    enum_id,
                    variant: *variant,
                    fields,
                })
            }
        }
    }

    /// The stack of `pat`, of type `col`, on `tail`.
    fn push(&mut self, pat: PatId, col: Col, tail: StackId) -> StackId {
        let wild = pat == WILD && self.nodes[tail].wild;
        let nodes = &mut self.nodes;
        *self.node_ids.entry((pat, col, tail)).or_insert_with(|| {
            nodes.push(Node {
                pat,
                col,
                tail,
                wild,
            });
            nodes.len() - 1
        })
    }

    /// The variant `id` names and its fields, unless it is `_`.
    fn variant(&self, id: PatId) -> Option<(usize, &[PatId])> {
        match &self.pats[id] {
            Pat::Wild => None,
            Pat::Variant {
                variant, fields, ..
            } => Some((*variant, fields)),
        }
    }

    fn print(&self, id: PatId) -> String {
        match &self.pats[id] {
            Pat::Wild => "_".to_string(),
            Pat::Variant {
                enum_id,
                variant,
                fields,
            } => {
                let name = &self.enums[*enum_id].variants[*variant].name;
                if fields.is_empty() {
                    name.clone()
                } else {
                    format!("{name}({})", self.print_all(fields))
                }
            }
        }
    }

    fn print_all(&self, ids: &[PatId]) -> String {
        let printed: Vec<_> = ids.iter().map(|&id| self.print(id)).collect();
        printed.join(", ")
    }

    /// The matrix of `rows`, whose last row has only wildcards, solved or looked up.
    fn solve(&mut self, rows: &[StackId]) -> Rc<Solved> {
        if let Some(solved) = self.memo.get(rows) {
            return Rc::clone(solved);
        }
        let solved = Rc::new(self.split(rows));
        self.memo.insert(rows.into(), Rc::clone(&solved));
        solved
    }

    /// Solves the matrix of `rows` by specializing its first column.
    fn split(&mut self, rows: &[StackId]) -> Solved {
        let first = self.nodes[rows[0]];
        // The all-wildcard row cut: the first row takes every value the matrix stands for.
        if first.wild {
            let mut reach = vec![Reach::Taken(0); rows.len()];
            reach[0] = Reach::Reached;
            let witness = (rows.len() == 1).then(|| self.wildcards(rows[0]));
            return Solved { reach, witness };
        }
        let heads: Vec<_> = rows.iter().map(|&row| self.nodes[row]).collect();
        let mut named: Vec<_> = heads
            .iter()
            .filter_map(|head| self.variant(head.pat).map(|(v, _)| v))
            .collect();
        named.sort_unstable();
        named.dedup();
        let bucket: FxHashMap<_, _> = named.iter().enumerate().map(|(b, &v)| (v, b)).collect();
        let mut buckets = vec![Vec::new(); named.len()];
        let mut default = Vec::new();
        for (i, head) in heads.iter().enumerate() {
            if let Some((v, _)) = self.variant(head.pat) {
                buckets[bucket[&v]].push(i);
            } else {
                // shortcut: a wildcard row joins every bucket, so n named variants above n
                // wildcard rows cost n²; share the wildcard rows between buckets if a real
                // program hits it.
                for rows in &mut buckets {
                    rows.push(i);
                }
                default.push(i);
            }
        }
        // Every row lands in a bucket or the default, so each gets a `Reach`.
        let mut reach = vec![None; rows.len()];
        // The witness found first in variant order, with the variant it was found under.
        let mut witness: Option<(usize, Vec<PatId>)> = None;
        let variants = first.col.map_or(0, |e| self.enums[e].variants.len());
        for (&variant, members) in named.iter().zip(&buckets) {
            let enum_id = first.col.expect("a variant pattern should sit at an enum");
            let enums = self.enums;
            let payload = &enums[enum_id].variants[variant].fields;
            let child: Vec<_> = members
                .iter()
                .map(|&i| {
                    let head = heads[i];
                    let fields = self.variant(head.pat).map(|(_, f)| f.to_vec());
                    payload
                        .iter()
                        .enumerate()
                        .rev()
                        .fold(head.tail, |tail, (k, field)| {
                            let pat = fields.as_ref().map_or(WILD, |f| f[k]);
                            self.push(pat, column(&field.ty), tail)
                        })
                })
                .collect();
            let solved = self.solve(&child);
            merge(&mut reach, members, &solved);
            // The buckets go in variant order, so the first witness is the first in order.
            if let Some(found) = &solved.witness
                && witness.is_none()
            {
                let (fields, rest) = found.split_at(payload.len());
                let head = self.pat(Pat::Variant {
                    enum_id,
                    variant,
                    fields: fields.into(),
                });
                witness = Some((variant, [&[head], rest].concat()));
            }
        }
        if named.len() < variants || first.col.is_none() {
            let child: Vec<_> = default.iter().map(|&i| heads[i].tail).collect();
            let solved = self.solve(&child);
            merge(&mut reach, &default, &solved);
            // The default stands for the first variant no row names; with none named, for `_`.
            let unnamed = (0..named.len())
                .find(|&v| named[v] != v)
                .unwrap_or(named.len());
            if let Some(found) = &solved.witness
                && witness.as_ref().is_none_or(|(at, _)| unnamed < *at)
            {
                let head = match first.col {
                    Some(enum_id) if !named.is_empty() => {
                        let count = self.enums[enum_id].variants[unnamed].fields.len();
                        self.pat(Pat::Variant {
                            enum_id,
                            variant: unnamed,
                            fields: vec![WILD; count].into(),
                        })
                    }
                    _ => WILD,
                };
                witness = Some((unnamed, [&[head], &found[..]].concat()));
            }
        }
        Solved {
            reach: reach
                .into_iter()
                .map(|reach| reach.expect("every row should land in a child matrix"))
                .collect(),
            witness: witness.map(|(_, witness)| witness),
        }
    }

    /// One `_` for each column of `stack`.
    fn wildcards(&self, mut stack: StackId) -> Vec<PatId> {
        let mut wildcards = Vec::new();
        while stack != EMPTY {
            wildcards.push(WILD);
            stack = self.nodes[stack].tail;
        }
        wildcards
    }
}

/// Adds what `solved` says of its rows, which are the rows `members` of `reach`'s matrix.
fn merge(reach: &mut [Option<Reach>], members: &[usize], solved: &Solved) {
    for (&i, &found) in members.iter().zip(&solved.reach) {
        let found = match found {
            Reach::Taken(by) => Reach::Taken(members[by]),
            found => found,
        };
        reach[i] = Some(reach[i].map_or(found, |seen| seen.merge(found)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::tests::checked;

    /// The report on the `match` that function `t` of `src` ends in, without its last arm;
    /// `src` checks clean.
    fn report(src: &str) -> Report {
        let (program, ds) = checked(src);
        assert!(ds.is_empty(), "{ds:?}");
        let program = program.expect("the source should check clean");
        let t = program
            .functions
            .iter()
            .find(|f| f.name == "t")
            .expect("the source should define `t`");
        let Some(tir::ExprKind::Match {
            scrutinee, arms, ..
        }) = t.body.value.as_ref().map(|value| &value.kind)
        else {
            panic!("`t` should end in a `match`");
        };
        let (_, arms) = arms
            .split_last()
            .expect("the `match` should end in `_ => 0`");
        let rows: Vec<_> = arms
            .iter()
            .map(|arm| Row {
                pattern: &arm.pattern,
            })
            .collect();
        check(&program.enums, &scrutinee.ty, &rows)
    }

    /// The texts of `Missing.partly`: the arm's pattern and its payload.
    type Texts<'a> = Option<(&'a str, &'a str)>;

    #[test]
    fn witnesses() {
        let decls =
            "enum Expr {\n    Num(i64)\n    Add(Expr, Expr)\n    Mul(Expr, Expr)\n}\nfn main() {}";
        // Each row: the scrutinee, the arms before `_ => 0`, the missing pattern, and the
        // texts of the arm that covers part of its variant.
        let rows: [(&str, &[&str], &str, Texts<'_>); 7] = [
            ("e", &["Num(_) => 1", "Add(_, _) => 2"], "Mul(_, _)", None),
            (
                "e",
                &["Num(_) => 1", "Add(Num(_), _) => 2", "Mul(_, _) => 3"],
                "Add(Add(_, _), _)",
                Some(("Add(Num(_), _)", "Num(_), _")),
            ),
            ("i", &[], "_", None),
            // A default before a bucket in variant order wins over it, and one after loses.
            ("e", &["Add(Num(_), _) => 1"], "Num(_)", None),
            (
                "e",
                &["Num(_) => 1", "Add(Num(_), _) => 2"],
                "Add(Add(_, _), _)",
                Some(("Add(Num(_), _)", "Num(_), _")),
            ),
            // A column no arm names a variant in prints as `_`.
            (
                "e",
                &["Num(_) => 1", "Add(_, Num(_)) => 2", "Mul(_, _) => 3"],
                "Add(_, Add(_, _))",
                Some(("Add(_, Num(_))", "_, Num(_)")),
            ),
            // The sentinel alone under two columns gives a `_` for each.
            (
                "e",
                &[
                    "Num(_) => 1",
                    "Add(Num(_), _) => 2",
                    "Add(Add(Num(_), _), _) => 3",
                    "Mul(_, _) => 4",
                ],
                "Add(Add(Add(_, _), _), _)",
                Some(("Add(Num(_), _)", "Num(_), _")),
            ),
        ];
        for (scrutinee, arms, pattern, partly) in rows {
            let arms = arms
                .iter()
                .map(|arm| format!("        {arm}\n"))
                .collect::<Vec<_>>()
                .concat();
            let src = format!(
                "{decls}\nfn t(e: Expr, i: i64) -> i64 {{\n    match {scrutinee} {{\n{arms}        _ => 0\n    }}\n}}"
            );
            let missing = report(&src).missing.expect("the arms should miss a value");
            assert_eq!(missing.pattern, pattern, "{src}");
            let texts = missing
                .partly
                .as_ref()
                .map(|(_, pattern, payload)| (pattern.as_str(), payload.as_str()));
            assert_eq!(texts, partly, "{src}");
        }
    }
}
