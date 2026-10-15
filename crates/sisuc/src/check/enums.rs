//! Enum declarations: their variants and payloads, the names they cannot take, which enums no
//! value can be built of, the boxes that break a cycle, and which enums are counted.

use rustc_hash::FxHashMap;

use super::{Checker, Definition, DiagnosticKind};
use crate::ast;
use crate::diagnostic::{BuiltIn, Unbuilt};
use crate::tir::{self, EnumId, Type};

pub(super) struct EnumInfo {
    pub(super) name: ast::Ident,
    /// The first variant of each name, in declaration order.
    pub(super) variants: Vec<VariantInfo>,
}

pub(super) struct VariantInfo {
    pub(super) name: ast::Ident,
    /// The payload's field types; `None` is `Error`.
    pub(super) fields: Vec<Option<Type>>,
}

/// The enum that a payload field of type `ty` holds inline: an enum's, or an optional enum's.
fn inline_enum(ty: &Type) -> Option<EnumId> {
    match ty {
        Type::Enum(id) => Some(*id),
        Type::Optional(payload) => inline_enum(payload),
        _ => None,
    }
}

/// The name milestone 8's prelude takes, which no item, variant or binding may take, and what
/// it is there.
fn reserved_name(name: &str) -> Option<BuiltIn> {
    match name {
        "Option" | "Result" => Some(BuiltIn::Type),
        "Some" | "Ok" | "Err" => Some(BuiltIn::Constructor),
        _ => None,
    }
}

impl Checker<'_> {
    /// Whether `name` is one milestone 8's prelude takes; reports it when it is.
    pub(super) fn reserved(&mut self, name: &ast::Ident) -> bool {
        let Some(kind) = reserved_name(name.name.as_str()) else {
            return false;
        };
        self.report(DiagnosticKind::BuiltInName {
            name: name.name.to_string(),
            at: name.span,
            kind,
        });
        true
    }

    /// Checks the variants of `e`, enum `id`, and resolves their payload types and its method
    /// signatures, adding each method to `declared` with `self`'s type. Its variants and methods
    /// share a namespace; each payload's labels have one of their own.
    pub(super) fn enum_signatures<'a>(
        &mut self,
        e: &'a ast::Enum,
        id: EnumId,
        declared: &mut Vec<(&'a ast::Function, Option<Type>)>,
    ) {
        let owner = e.name.name.as_str();
        if e.variants.is_empty() {
            self.report(DiagnosticKind::EmptyEnum {
                name: owner.to_string(),
                at: e.name.span,
            });
        }
        let mut first = FxHashMap::default();
        for variant in &e.variants {
            let name = &variant.name;
            if name.name.as_str().starts_with(char::is_uppercase) {
                self.reserved(name);
            } else {
                self.report(DiagnosticKind::VariantCase {
                    name: name.name.to_string(),
                    at: name.span,
                });
            }
            let payload = variant.payload.as_deref().unwrap_or_default();
            let labeled = |field: &ast::PayloadField| field.label.is_some();
            if let Some(field) = payload
                .iter()
                .find(|field| labeled(field) != labeled(&payload[0]))
            {
                self.report(DiagnosticKind::MixedLabels {
                    variant: name.name.to_string(),
                    at: field
                        .label
                        .as_ref()
                        .map_or(field.ty.span, |label| label.span),
                });
            }
            let mut labels = FxHashMap::default();
            for label in payload.iter().filter_map(|field| field.label.as_ref()) {
                self.first_definition(&mut labels, Definition::Field, Some(owner), label);
            }
            let fields = payload
                .iter()
                .map(|field| self.resolve(&field.ty))
                .collect();
            if self.first_definition(&mut first, Definition::Variant, Some(owner), name) {
                self.enums[id.0].variants.push(VariantInfo {
                    name: name.clone(),
                    fields,
                });
            }
        }
        for method in &e.methods {
            self.first_definition(&mut first, Definition::Method, Some(owner), &method.name);
            self.signature(method, format!("{owner}.{}", method.name.name));
            declared.push((method, Some(Type::Enum(id))));
        }
    }

    /// Lowers every enum: boxes each payload field whose enum lies in the declaring enum's
    /// component of the holds-inline graph, and marks the enums with counted parts. Reports
    /// the enums no value can be built of; `cyclic` marks the classes on a cycle of fields,
    /// which `never_constructed` reports.
    pub(super) fn lower_enums(&mut self, cyclic: &[bool]) -> Vec<tir::Enum> {
        let held: Vec<Vec<usize>> = self
            .enums
            .iter()
            .map(|e| {
                let fields = e.variants.iter().flat_map(|v| v.fields.iter().flatten());
                fields.filter_map(inline_enum).map(|id| id.0).collect()
            })
            .collect();
        let component = components(&held);
        let mut lowered: Vec<tir::Enum> = self
            .enums
            .iter()
            .enumerate()
            .map(|(e, info)| {
                let variants: Vec<tir::Variant> = info
                    .variants
                    .iter()
                    .map(|v| tir::Variant {
                        name: v.name.name.to_string(),
                        fields: v
                            .fields
                            .iter()
                            .map(|ty| {
                                // An `Error` field's error stops the program before codegen.
                                let ty = ty.clone().unwrap_or(Type::Never);
                                let boxed = inline_enum(&ty)
                                    .is_some_and(|f| component[f.0] == component[e]);
                                tir::Payload { ty, boxed }
                            })
                            .collect(),
                    })
                    .collect();
                // Every member of a component of more than one enum holds another inline, so
                // the component has a box exactly when its every member has one.
                let boxed = variants.iter().flat_map(|v| &v.fields).any(|f| f.boxed);
                tir::Enum {
                    name: info.name.name.to_string(),
                    variants,
                    counted: false,
                    cycle: boxed.then_some(component[e]),
                }
            })
            .collect();
        // A component is numbered after every component it holds, and holds its own members
        // only through boxes, so counting in component order sees each inline enum counted.
        let mut order: Vec<usize> = (0..lowered.len()).collect();
        order.sort_by_key(|&e| component[e]);
        for e in order {
            let counted = lowered[e]
                .variants
                .iter()
                .flat_map(|v| &v.fields)
                .any(|f| f.boxed || f.ty.is_counted(&lowered));
            lowered[e].counted = counted;
        }
        self.never_built(cyclic);
        lowered
    }

    /// Reports each enum no value can be built of. Which classes and enums can be built is a
    /// least fixed point over them all: a class can when every field's type can, an enum when
    /// every field of some variant can, and every other type always can. A class in `cyclic`
    /// counts as built, and so does an enum with no variant, since each has its error already.
    /// Each component of the rest that needs no other is reported once, at its first enum; a
    /// type that only needs one of those is not.
    fn never_built(&mut self, cyclic: &[bool]) {
        let classes = self.classes.len();
        let node = |ty: &Option<Type>| match ty {
            Some(Type::Class(id)) => Some(id.0),
            Some(Type::Enum(id)) => Some(classes + id.0),
            _ => None,
        };
        // Each class, and each variant, as (its type's node, the nodes it needs built).
        let mut slots: Vec<(usize, Vec<usize>)> = Vec::new();
        for (c, class) in self.classes.iter().enumerate() {
            let needs = if cyclic[c] {
                Vec::new()
            } else {
                class.fields.iter().filter_map(|f| node(&f.ty)).collect()
            };
            slots.push((c, needs));
        }
        for (e, info) in self.enums.iter().enumerate() {
            if info.variants.is_empty() {
                slots.push((classes + e, Vec::new()));
            }
            for variant in &info.variants {
                slots.push((
                    classes + e,
                    variant.fields.iter().filter_map(node).collect(),
                ));
            }
        }
        let nodes = classes + self.enums.len();
        let mut waiting = vec![Vec::new(); nodes];
        for (s, (_, needs)) in slots.iter().enumerate() {
            for &need in needs {
                waiting[need].push(s);
            }
        }
        let mut missing: Vec<usize> = slots.iter().map(|(_, needs)| needs.len()).collect();
        let mut built = vec![false; nodes];
        let mut ready: Vec<usize> = Vec::new();
        let complete = |s: usize, built: &mut Vec<bool>, ready: &mut Vec<usize>| {
            let owner = slots[s].0;
            if !built[owner] {
                built[owner] = true;
                ready.push(owner);
            }
        };
        for s in (0..slots.len()).filter(|&s| missing[s] == 0) {
            complete(s, &mut built, &mut ready);
        }
        while let Some(n) = ready.pop() {
            for &s in &waiting[n] {
                missing[s] -= 1;
                if missing[s] == 0 {
                    complete(s, &mut built, &mut ready);
                }
            }
        }
        // What each node no value can be built of needs that none can be built of.
        let mut needs = vec![Vec::new(); nodes];
        for (owner, wants) in &slots {
            if !built[*owner] {
                needs[*owner].extend(wants.iter().copied().filter(|&n| !built[n]));
            }
        }
        let component = components(&needs);
        let mut leaves = vec![true; nodes];
        for (n, wants) in needs.iter().enumerate() {
            if wants.iter().any(|&w| component[w] != component[n]) {
                leaves[component[n]] = false;
            }
        }
        let mut reported = vec![false; nodes];
        for e in 0..self.enums.len() {
            let n = classes + e;
            if built[n] || !leaves[component[n]] || reported[component[n]] {
                continue;
            }
            reported[component[n]] = true;
            let chain = self.chain(n, &built);
            let name = &self.enums[e].name;
            self.report(DiagnosticKind::NeverConstructed {
                name: name.name.to_string(),
                at: name.span,
                cause: Unbuilt::Chain(chain),
            });
        }
    }

    /// The shortest chain of fields from the enum at node `start` back to it, through types
    /// none of which can be `built`: each link's holder (`E.V` or `K.f`) and the type held.
    fn chain(&self, start: usize, built: &[bool]) -> Vec<(String, String)> {
        let classes = self.classes.len();
        let node = |ty: &Type| match ty {
            Type::Class(id) => Some(id.0),
            Type::Enum(id) => Some(classes + id.0),
            _ => None,
        };
        // Each link out of node `n`: its holder, and the type it holds.
        let links = |n: usize| -> Vec<(String, &Type)> {
            if n < classes {
                let class = &self.classes[n];
                let fields = class.fields.iter();
                let fields = fields.filter_map(|f| Some((f.name.name.as_str(), f.ty.as_ref()?)));
                fields
                    .map(|(field, ty)| (format!("{}.{field}", class.name.name), ty))
                    .collect()
            } else {
                let e = &self.enums[n - classes];
                e.variants
                    .iter()
                    .flat_map(|v| v.fields.iter().flatten().map(move |ty| (v, ty)))
                    .map(|(v, ty)| (format!("{}.{}", e.name.name, v.name.name), ty))
                    .collect()
            }
        };
        let mut came_from: Vec<Option<(usize, String, &Type)>> = vec![None; built.len()];
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(n) = queue.pop_front() {
            for (holder, ty) in links(n) {
                let Some(m) = node(ty).filter(|&m| !built[m]) else {
                    continue;
                };
                if m == start {
                    let mut chain = vec![(holder, self.type_name(ty))];
                    let mut at = n;
                    while at != start {
                        let (from, holder, ty) = came_from[at]
                            .take()
                            .expect("a node the walk reached should know where it came from");
                        chain.push((holder, self.type_name(ty)));
                        at = from;
                    }
                    chain.reverse();
                    return chain;
                }
                if came_from[m].is_none() {
                    came_from[m] = Some((n, holder, ty));
                    queue.push_back(m);
                }
            }
        }
        panic!("an enum no value can be built of should need itself through its component");
    }
}

/// Each node's strongly connected component of the graph `edges`, by Tarjan's algorithm,
/// visiting the nodes in order. Components are numbered as they complete, so a component's
/// number is above that of every other component it reaches. The walk keeps its own stack,
/// since a chain of declarations can be long.
fn components(edges: &[Vec<usize>]) -> Vec<usize> {
    const UNSEEN: usize = usize::MAX;
    let mut index = vec![UNSEEN; edges.len()];
    let mut low = vec![0; edges.len()];
    let mut component = vec![UNSEEN; edges.len()];
    // The nodes seen and not yet in a component, and the walk's path with each node's next edge.
    let mut open = Vec::new();
    let mut path: Vec<(usize, usize)> = Vec::new();
    let mut next_index = 0;
    let mut next_component = 0;
    for root in 0..edges.len() {
        if index[root] != UNSEEN {
            continue;
        }
        path.push((root, 0));
        while let Some((n, next)) = path.last_mut() {
            let n = *n;
            if index[n] == UNSEEN {
                index[n] = next_index;
                low[n] = next_index;
                next_index += 1;
                open.push(n);
            }
            if let Some(&m) = edges[n].get(*next) {
                *next += 1;
                if index[m] == UNSEEN {
                    path.push((m, 0));
                } else if component[m] == UNSEEN {
                    low[n] = low[n].min(index[m]);
                }
                continue;
            }
            path.pop();
            if let Some(&(parent, _)) = path.last() {
                low[parent] = low[parent].min(low[n]);
            }
            if low[n] == index[n] {
                while let Some(m) = open.pop() {
                    component[m] = next_component;
                    if m == n {
                        break;
                    }
                }
                next_component += 1;
            }
        }
    }
    component
}
