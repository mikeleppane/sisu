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
    let result = solver.solve(stacks.into());
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
    let missing = result.witness.map(|witness| {
        let top = solver.nodes[witness].pat;
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
    /// A value that only the last row, the sentinel, matches: a stack of one pattern per
    /// column, shared with the matrices that found it.
    witness: Option<StackId>,
}

/// A matrix that specializing a column makes: the bucket of the variant `order`, or the
/// default, which stands for the variant `order` when a row names another and for `_` when none
/// does.
struct Child {
    order: usize,
    bucket: bool,
    /// The parent's rows in it, in order.
    members: Vec<usize>,
}

/// A matrix being solved: its first column split into `children`, of which the first `next`
/// are solved.
struct Frame {
    rows: Box<[StackId]>,
    col: Col,
    children: Vec<Child>,
    next: usize,
    /// What the solved children say of each row.
    reach: Vec<Option<Reach>>,
    /// The witness first in constructor order so far: its child's order, the child, and the
    /// child's witness.
    best: Option<(usize, usize, StackId)>,
}

impl Frame {
    /// Takes in what the child `next` found, and moves on to the next child.
    fn feed(&mut self, solved: &Solved) {
        let child = &self.children[self.next];
        merge(&mut self.reach, &child.members, solved);
        if let Some(witness) = solved.witness
            && self.best.is_none_or(|(order, ..)| child.order < order)
        {
            self.best = Some((child.order, self.next, witness));
        }
        self.next += 1;
    }
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

    /// Field `k` of `pat`, a variant pattern or `_`.
    fn field(&self, pat: PatId, k: usize) -> PatId {
        match &self.pats[pat] {
            Pat::Wild => WILD,
            Pat::Variant { fields, .. } => fields[k],
        }
    }

    /// The matrix of `rows`, whose last row has only wildcards, solved. A matrix in progress
    /// waits on a stack of frames, not on the Rust stack, so a payload thousands of fields wide
    /// does not overflow it.
    fn solve(&mut self, rows: Box<[StackId]>) -> Rc<Solved> {
        let mut frames = Vec::new();
        let mut done = self.start(rows, &mut frames);
        loop {
            if let Some(solved) = done.take() {
                let Some(top) = frames.last_mut() else {
                    return solved;
                };
                top.feed(&solved);
            }
            let top = frames.last().expect("a frame should wait for its children");
            if top.next == top.children.len() {
                let frame = frames.pop().expect("a frame should wait for its children");
                done = Some(self.finish(frame));
            } else {
                let rows = self.child_rows(top);
                done = self.start(rows, &mut frames);
            }
        }
    }

    /// The matrix of `rows` solved, when the cut or the memo solves it at once; otherwise
    /// `None`, with a frame for it pushed onto `frames`.
    fn start(&mut self, rows: Box<[StackId]>, frames: &mut Vec<Frame>) -> Option<Rc<Solved>> {
        // The all-wildcard row cut: the first row takes every value the matrix stands for.
        if self.nodes[rows[0]].wild {
            let mut reach = vec![Reach::Taken(0); rows.len()];
            reach[0] = Reach::Reached;
            // Alone, the first row is the sentinel, already a `_` for each column.
            let witness = (rows.len() == 1).then_some(rows[0]);
            return Some(Rc::new(Solved { reach, witness }));
        }
        if let Some(solved) = self.memo.get(&rows) {
            return Some(Rc::clone(solved));
        }
        frames.push(self.frame(rows));
        None
    }

    /// The frame that specializes the first column of `rows`: one bucket per variant a row
    /// names, in the order the rows name them, then the default when a variant is left.
    fn frame(&self, rows: Box<[StackId]>) -> Frame {
        let col = self.nodes[rows[0]].col;
        let mut bucket = FxHashMap::default();
        let mut children: Vec<Child> = Vec::new();
        let mut wild = Vec::new();
        for (i, &row) in rows.iter().enumerate() {
            if let Some((variant, _)) = self.variant(self.nodes[row].pat) {
                let b = *bucket.entry(variant).or_insert_with(|| {
                    // A bucket also holds every wildcard row above its first row.
                    children.push(Child {
                        order: variant,
                        bucket: true,
                        members: wild.clone(),
                    });
                    children.len() - 1
                });
                children[b].members.push(i);
            } else {
                // shortcut: a wildcard row joins every bucket, so n named variants above n
                // wildcard rows cost n²; share the wildcard rows between buckets if a real
                // program hits it.
                for child in &mut children {
                    child.members.push(i);
                }
                wild.push(i);
            }
        }
        let variants = col.map_or(0, |e| self.enums[e].variants.len());
        if children.is_empty() || children.len() < variants {
            let order = (0..children.len())
                .find(|v| !bucket.contains_key(v))
                .unwrap_or(children.len());
            children.push(Child {
                order,
                bucket: false,
                members: wild,
            });
        }
        Frame {
            reach: vec![None; rows.len()],
            rows,
            col,
            children,
            next: 0,
            best: None,
        }
    }

    /// The rows of the child `next` of `frame`: a bucket's rows with the variant's fields in
    /// place of their first column, the default's without it.
    fn child_rows(&mut self, frame: &Frame) -> Box<[StackId]> {
        let child = &frame.children[frame.next];
        let heads: Vec<_> = child
            .members
            .iter()
            .map(|&i| self.nodes[frame.rows[i]])
            .collect();
        if !child.bucket {
            return heads.into_iter().map(|head| head.tail).collect();
        }
        let enums = self.enums;
        let enum_id = frame.col.expect("a variant pattern should sit at an enum");
        let payload = &enums[enum_id].variants[child.order].fields;
        heads
            .into_iter()
            .map(|head| {
                payload
                    .iter()
                    .enumerate()
                    .rev()
                    .fold(head.tail, |tail, (k, field)| {
                        let pat = self.field(head.pat, k);
                        self.push(pat, column(&field.ty), tail)
                    })
            })
            .collect()
    }

    /// What `frame`, all of whose children are solved, says; also kept for its rows.
    fn finish(&mut self, frame: Frame) -> Rc<Solved> {
        let witness = frame.best.map(|(order, child, witness)| {
            let head = match frame.col {
                // A bucket's witness starts with the variant's fields: they become its payload.
                Some(enum_id) if frame.children[child].bucket => {
                    let count = self.enums[enum_id].variants[order].fields.len();
                    let mut fields = Vec::with_capacity(count);
                    let mut rest = witness;
                    for _ in 0..count {
                        let node = self.nodes[rest];
                        fields.push(node.pat);
                        rest = node.tail;
                    }
                    let head = self.pat(Pat::Variant {
                        enum_id,
                        variant: order,
                        fields: fields.into(),
                    });
                    return self.push(head, frame.col, rest);
                }
                // The default stands for a variant when a row names another.
                Some(enum_id) if frame.children.len() > 1 => {
                    let count = self.enums[enum_id].variants[order].fields.len();
                    self.pat(Pat::Variant {
                        enum_id,
                        variant: order,
                        fields: vec![WILD; count].into(),
                    })
                }
                _ => WILD,
            };
            self.push(head, frame.col, witness)
        });
        let solved = Rc::new(Solved {
            reach: frame
                .reach
                .into_iter()
                .map(|reach| reach.expect("every row should land in a child matrix"))
                .collect(),
            witness,
        });
        self.memo.insert(frame.rows, Rc::clone(&solved));
        solved
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
