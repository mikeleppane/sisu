//! The parser: tokens to the syntax tree.

use std::ops::Range;

use crate::ast::{
    Arg, BinaryOp, Block, Broken, BrokenKind, Class, CompareOp, Const, Expr, ExprKind, FieldDecl,
    ForSource, Function, Ident, Item, Member, Param, Program, Stmt, StmtKind, TypeExpr, TypeKind,
    UnaryOp,
};
use crate::diagnostic::{Diagnostic, DiagnosticKind, Join, LetField, SemicolonPlace, Span};
use crate::lexer::{Lexed, MIN_MAGNITUDE, Token, TokenKind, literal_too_large};
use crate::symbol::Symbol;

/// What the parser makes of a lexed source.
#[derive(Debug)]
pub(crate) struct Parsed {
    /// The intact items, and the names of the broken ones.
    pub(crate) program: Program,
    /// One error per broken item, and each lexer diagnostic outside every item, by position.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Whether every item is named, so that no use of a lost item can read as "cannot find".
    pub(crate) checkable: bool,
}

/// Parses `lexed`, whose tokens end with `Eof` and whose brackets balance. An item with a
/// syntax error, its own or the lexer's, is broken: it reports its earliest error, and the
/// parser resumes after it.
pub(crate) fn parse(lexed: Lexed) -> Parsed {
    let Lexed {
        tokens,
        diagnostics,
        comments,
        breaks,
    } = lexed;
    Parser {
        tokens: &tokens,
        pos: 0,
        depth: 0,
        lexer: &diagnostics,
        comments: &comments,
    }
    .program(breaks)
}

/// The program of `source`, which must lex and parse without a diagnostic.
#[cfg(test)]
pub(crate) fn parse_ok(source: &str) -> Program {
    let parsed = parse(crate::lexer::lex(source).expect("the source should lex"));
    assert!(
        parsed.diagnostics.is_empty() && parsed.checkable,
        "{source}: {:?}",
        parsed.diagnostics
    );
    parsed.program
}

/// The earlier of `a` and `b`; `a` on a tie.
fn earlier(a: Diagnostic, b: Diagnostic) -> Diagnostic {
    if b.span().start < a.span().start {
        b
    } else {
        a
    }
}

/// How deep the syntax tree may nest. Every later stage walks the tree recursively, and so does
/// dropping it, so the parser is the one place that bounds the stack they need.
const MAX_DEPTH: usize = 256;

/// An infix operator other than the comparisons of level 4.
enum Infix {
    Binary(BinaryOp),
    Is,
    Coalesce,
}

/// The level and operator of an infix operator token.
fn infix_op(kind: &TokenKind) -> Option<(u8, Infix)> {
    Some(match kind {
        TokenKind::OrOr => (1, Infix::Binary(BinaryOp::Or)),
        TokenKind::AndAnd => (2, Infix::Binary(BinaryOp::And)),
        TokenKind::EqEq => (3, Infix::Binary(BinaryOp::Eq)),
        TokenKind::NotEq => (3, Infix::Binary(BinaryOp::Ne)),
        TokenKind::Is => (3, Infix::Is),
        TokenKind::QuestionQuestion => (5, Infix::Coalesce),
        TokenKind::Plus => (6, Infix::Binary(BinaryOp::Add)),
        TokenKind::Minus => (6, Infix::Binary(BinaryOp::Sub)),
        TokenKind::Star => (7, Infix::Binary(BinaryOp::Mul)),
        TokenKind::Slash => (7, Infix::Binary(BinaryOp::Div)),
        TokenKind::Percent => (7, Infix::Binary(BinaryOp::Rem)),
        _ => return None,
    })
}

/// Whether `kind` begins a postfix link (`.`, `?.` or `[`); `postfix` and `at_min_literal`
/// share it.
fn starts_postfix(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Dot | TokenKind::QuestionDot | TokenKind::LBracket
    )
}

fn compare_op(kind: &TokenKind) -> Option<CompareOp> {
    Some(match kind {
        TokenKind::Lt => CompareOp::Lt,
        TokenKind::Le => CompareOp::Le,
        TokenKind::Gt => CompareOp::Gt,
        TokenKind::Ge => CompareOp::Ge,
        _ => return None,
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// While a node's children are parsed: the levels above them. When a parse function returns:
    /// the height of the deepest path through the tree it built, counted from the level it was
    /// entered at, in the same units. A node that wraps what it already parsed, as an operator
    /// or a `.` does in a chain, adds a level to that height, so no way of building a tree gets
    /// past `MAX_DEPTH`. An error ends the parse.
    depth: usize,
    /// The lexer's diagnostics, in source order.
    lexer: &'a [Diagnostic],
    /// Where each comment starts.
    comments: &'a [usize],
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &'a Token {
        // `Eof` is last and never consumed, so `pos` stays in range.
        &self.tokens[self.pos]
    }

    fn bump(&mut self) -> &'a Token {
        let token = self.peek();
        if token.kind != TokenKind::Eof {
            self.pos += 1;
        }
        token
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek().kind == *kind
    }

    /// Consumes the next token if it is `kind`.
    fn eat(&mut self, kind: &TokenKind) -> Option<&'a Token> {
        self.at(kind).then(|| self.bump())
    }

    /// Sets `self.depth`, or reports that the tree is too deep at the next token.
    fn reach(&mut self, depth: usize) -> Result<(), Diagnostic> {
        self.depth = depth;
        if depth > MAX_DEPTH {
            return Err(DiagnosticKind::NestingTooDeep {
                at: self.peek().span,
                limit: MAX_DEPTH,
            }
            .into());
        }
        Ok(())
    }

    /// Enters one more level of nesting for the children that follow.
    fn descend(&mut self) -> Result<(), Diagnostic> {
        self.reach(self.depth + 1)
    }

    /// Parses one child of a node whose children start at depth `base`, and raises `reach`, the
    /// height of the node so far, to the child's. Every parse function follows the same pattern:
    /// reset `self.depth` to the base before each child, and keep the maximum in `reach`.
    fn child<T>(
        &mut self,
        base: usize,
        reach: &mut usize,
        parse: impl FnOnce(&mut Self) -> Result<T, Diagnostic>,
    ) -> Result<T, Diagnostic> {
        self.depth = base;
        let node = parse(self)?;
        *reach = (*reach).max(self.depth);
        Ok(node)
    }

    fn skip_newlines(&mut self) {
        while self.eat(&TokenKind::Newline).is_some() {}
    }

    /// What must follow a statement or a class member: a line break or the closing `}`. A `;`
    /// here ends a complete statement, so it can be removed or the next one moved.
    fn end_of_line(&self) -> Diagnostic {
        if self.at(&TokenKind::Semicolon) {
            // `;` is not `Eof`, so the token after it exists.
            let place = match self.tokens[self.pos + 1].kind {
                TokenKind::Newline | TokenKind::RBrace | TokenKind::Eof => SemicolonPlace::EndsLine,
                TokenKind::Semicolon => SemicolonPlace::Repeated,
                _ => SemicolonPlace::MidLine,
            };
            return DiagnosticKind::Semicolon {
                at: self.peek().span,
                place,
            }
            .into();
        }
        self.expected("end of line")
    }

    /// "expected <what>, found <the next token>", pointing at the next token.
    /// A stray `var` or reserved word always gets its own diagnostic, wherever it appears.
    fn expected(&self, what: &str) -> Diagnostic {
        let found = self.peek();
        match found.kind {
            TokenKind::Var => DiagnosticKind::Var {
                at: found.span,
                field: false,
            },
            TokenKind::Reserved(word) => DiagnosticKind::ReservedWord {
                word,
                at: found.span,
            },
            TokenKind::Todo => DiagnosticKind::ReservedWord {
                word: "todo",
                at: found.span,
            },
            _ => DiagnosticKind::Expected {
                what: what.to_string(),
                found: found.kind.describe(),
                at: found.span,
            },
        }
        .into()
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<&'a Token, Diagnostic> {
        self.eat(kind)
            .ok_or_else(|| self.expected(&kind.describe()))
    }

    fn ident(&mut self, what: &str) -> Result<Ident, Diagnostic> {
        match self.peek().kind {
            TokenKind::Ident(name) => Ok(Ident {
                name,
                span: self.bump().span,
            }),
            _ => Err(self.expected(what)),
        }
    }

    /// An `ident` that names a function, class, constant, field or parameter, where `_` is an
    /// error.
    fn name(&mut self, what: &str) -> Result<Ident, Diagnostic> {
        if self.at(&TokenKind::Underscore) {
            return Err(DiagnosticKind::UnderscoreName {
                at: self.peek().span,
            }
            .into());
        }
        self.ident(what)
    }

    /// The name a `let` or `for` binds, which may be `_`.
    fn binding(&mut self) -> Result<Ident, Diagnostic> {
        if self.at(&TokenKind::Underscore) {
            return Ok(Ident {
                name: Symbol::intern("_"),
                span: self.bump().span,
            });
        }
        self.ident("a binding name")
    }

    /// The only place that reads a type; later milestones extend it.
    fn parse_type(&mut self) -> Result<TypeExpr, Diagnostic> {
        self.descend()?;
        let start = self.peek().span;
        let (kind, mut span) = if self.eat(&TokenKind::LBracket).is_some() {
            let element = self.parse_type()?;
            let close = self.expect(&TokenKind::RBracket)?;
            (TypeKind::Array(Box::new(element)), start.to(close.span))
        } else {
            let ident = self.ident("a type")?;
            (TypeKind::Name(ident.name), ident.span)
        };
        let optional = self.eat(&TokenKind::Question).is_some();
        if optional {
            span = span.to(self.last_span());
        }
        // `??` is one token, so `Tree??` and `Tree? ?` both stop here.
        if matches!(
            self.peek().kind,
            TokenKind::Question | TokenKind::QuestionQuestion
        ) {
            let base = TypeExpr {
                kind,
                optional: false,
                span,
            };
            return Err(DiagnosticKind::OptionalNests {
                ty: base.to_string(),
                at: start.to(self.peek().span),
            }
            .into());
        }
        Ok(TypeExpr {
            kind,
            optional,
            span,
        })
    }

    /// The program, which keeps the lexer's `breaks`.
    fn program(&mut self, breaks: Vec<usize>) -> Parsed {
        let mut items = Vec::new();
        let mut broken = Vec::new();
        let mut diagnostics = Vec::new();
        let mut checkable = true;
        // Where the items lie; each reports the lexer diagnostics inside it itself.
        let mut covered = Vec::new();
        self.skip_newlines();
        while !self.at(&TokenKind::Eof) {
            self.depth = 0;
            let start = self.pos;
            let tokens = self.tokens;
            let previous = tokens[..start].last();
            // A `;` after an item's `}` ends its line, as one does in a block.
            if self.at(&TokenKind::Semicolon)
                && previous.is_some_and(|t| t.kind == TokenKind::RBrace)
                && tokens[start + 1].kind == TokenKind::Newline
            {
                diagnostics.push(self.end_of_line());
                self.bump();
                self.skip_newlines();
                continue;
            }
            let header = self.header(start);
            // An item without a name would be lost, so the checker does not run; it runs on to
            // the next item, so that a foreign function and its body make one error.
            let end = if header.is_some() {
                self.item_end(start)
            } else {
                self.next_line_item(start)
            };
            debug_assert!(
                end > start,
                "an item should hold a token, or the loop would not end"
            );
            // An unnamed item starts where its line does, so text skipped before its first
            // token is its own.
            let line = previous.map_or(0, |t| t.span.end);
            let mut bytes = self.bytes(start, end);
            if header.is_none() {
                bytes.start = line;
            }
            covered.push(bytes);
            let parsed = match self.peek().kind {
                TokenKind::Fn => self.checked(end, |p| {
                    Ok((Item::Function(p.function(false)?), Vec::new()))
                }),
                TokenKind::Class => self
                    .class(end)
                    .map(|(class, errors)| (Item::Class(class), errors)),
                TokenKind::Const => {
                    self.checked(end, |p| Ok((Item::Const(p.constant(end)?), Vec::new())))
                }
                _ => self.checked(end, |p| Err(p.expected("`fn`, `class` or `const`"))),
            };
            match parsed {
                Ok((item, errors)) => {
                    debug_assert!(header.is_some(), "an unnamed item should hold an error");
                    items.push(item);
                    diagnostics.extend(errors);
                }
                Err(error) => {
                    if let Some(header) = header {
                        diagnostics.push(error);
                        broken.push(header);
                    } else {
                        let skipped = self.lexer_in(line..error.span().start).next().cloned();
                        diagnostics.push(skipped.unwrap_or(error));
                        checkable = false;
                    }
                }
            }
            self.skip_newlines();
        }
        let outside = |at: &usize| !covered.iter().any(|r| r.contains(at));
        diagnostics.extend(
            self.lexer
                .iter()
                .filter(|d| outside(&d.span().start))
                .cloned(),
        );
        diagnostics.sort_by_key(|d| d.span().start);
        Parsed {
            program: Program {
                items,
                broken,
                comments: self.comments.to_vec(),
                quotes: self
                    .tokens
                    .iter()
                    .filter(|t| matches!(t.kind, TokenKind::Str(_) | TokenKind::StrHead(_)))
                    .map(|t| t.span.start)
                    .collect(),
                breaks,
            },
            diagnostics,
            checkable,
        }
    }

    /// The name and kind of the item that starts at `start`, when its header is intact: `fn`,
    /// `class` or `const`, then a name, with no lexer diagnostic before the token after the name.
    /// `fn to$tal` lexes as `fn to tal`, so its name is not `to`.
    fn header(&self, start: usize) -> Option<Broken> {
        let kind = match self.tokens[start].kind {
            TokenKind::Fn => BrokenKind::Function,
            TokenKind::Class => BrokenKind::Class,
            TokenKind::Const => BrokenKind::Constant,
            _ => return None,
        };
        // Neither a keyword nor a name is `Eof`, so the two tokens after the keyword exist.
        let TokenKind::Ident(name) = &self.tokens[start + 1].kind else {
            return None;
        };
        let header = self.tokens[start].span.start..self.tokens[start + 2].span.start;
        if self.lexer_in(header).next().is_some() {
            return None;
        }
        Some(Broken {
            name: Ident {
                name: *name,
                span: self.tokens[start + 1].span,
            },
            kind,
        })
    }

    /// The end, exclusive, of the item or member that starts at `start`: past the `}` that
    /// brings the depth back to where it started, or at a line break at that depth, which comes
    /// before any `{`. A closer below that depth (a class's `}`) or `Eof` ends it first. A
    /// constant is one line, as a statement is, so a `}` in its value does not end it.
    fn item_end(&self, start: usize) -> usize {
        let line = self.tokens[start].kind == TokenKind::Const;
        let mut depth = 0usize;
        for (i, token) in self.tokens.iter().enumerate().skip(start) {
            match token.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    if depth == 0 {
                        return i;
                    }
                    depth -= 1;
                    if depth == 0 && token.kind == TokenKind::RBrace && !line {
                        return i + 1;
                    }
                }
                TokenKind::Newline if depth == 0 => return i,
                TokenKind::Eof => return i,
                _ => {}
            }
        }
        unreachable!("the tokens should end with `Eof`")
    }

    /// The end, exclusive, of the unnamed item that starts at `start`: the next line whose first
    /// token is `fn`, `class` or `const` outside brackets, or `Eof`.
    // shortcut: a line break after a token that cannot end a statement (`:`) leaves no `Newline`,
    // so a `fn` after one is not found; the item only runs longer, and the checker is off anyway.
    fn next_line_item(&self, start: usize) -> usize {
        let mut depth = 0usize;
        for (i, token) in self.tokens.iter().enumerate().skip(start) {
            match token.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
                TokenKind::Fn | TokenKind::Class | TokenKind::Const
                    if i > start && depth == 0 && self.tokens[i - 1].kind == TokenKind::Newline =>
                {
                    return i;
                }
                TokenKind::Eof => return i,
                _ => {}
            }
        }
        unreachable!("the tokens should end with `Eof`")
    }

    /// The bytes of the item from `start` to `end`, exclusive. An item that ends past its own `}`,
    /// or past the line break before the next item, ends there. Any other, and a constant, which
    /// is one line, runs up to the token that ends it (a line break, a closer or `Eof`), so that
    /// text skipped before it lies in the item.
    fn bytes(&self, start: usize, end: usize) -> Range<usize> {
        let line = self.tokens[start].kind == TokenKind::Const;
        let last = &self.tokens[end - 1];
        let stop = match (&last.kind, &self.tokens[end].kind) {
            (TokenKind::RBrace, _) if !line => last.span.end,
            // `Eof` sits just after the last code, before any skipped text.
            (_, TokenKind::Eof) => usize::MAX,
            _ => self.tokens[end].span.start,
        };
        self.tokens[start].span.start..stop
    }

    /// The lexer diagnostics that start in `bytes`, in order.
    fn lexer_in(&self, bytes: Range<usize>) -> impl Iterator<Item = &Diagnostic> {
        self.lexer
            .iter()
            .filter(move |d| bytes.contains(&d.span().start))
    }

    /// Parses with `parse` the item or member that starts here and runs to `end`. Its error is
    /// the earlier of its first lexer diagnostic and its parse error, which a skip may have
    /// caused. After an error, the parser resumes at `end`.
    fn checked<T>(
        &mut self,
        end: usize,
        parse: impl FnOnce(&mut Self) -> Result<T, Diagnostic>,
    ) -> Result<T, Diagnostic> {
        let bytes = self.bytes(self.pos, end);
        let parsed = parse(self);
        let lexed = self.lexer_in(bytes).next().cloned();
        let error = match (parsed, lexed) {
            (Ok(parsed), None) => return Ok(parsed),
            (Ok(_), Some(lexed)) => lexed,
            (Err(error), None) => error,
            (Err(error), Some(lexed)) => earlier(lexed, error),
        };
        self.pos = end;
        Err(error)
    }

    /// The class that starts here and runs to `end`, and the error of each broken method. A
    /// method whose error follows its name is broken alone and leaves the class; any other error
    /// breaks the whole class, whose one error is then the earliest in it.
    fn class(&mut self, end: usize) -> Result<(Class, Vec<Diagnostic>), Diagnostic> {
        let bytes = self.bytes(self.pos, end);
        let mut errors = Vec::new();
        let mut methods = Vec::new();
        let parsed = self.class_body(&mut errors, &mut methods);
        let outside = self
            .lexer_in(bytes)
            .find(|d| {
                !methods
                    .iter()
                    .any(|m: &Range<usize>| m.contains(&d.span().start))
            })
            .cloned();
        match (parsed, outside) {
            (Ok(class), None) => Ok((class, errors)),
            (parsed, outside) => {
                self.pos = end;
                let error = errors
                    .into_iter()
                    .chain(outside)
                    .chain(parsed.err())
                    .reduce(earlier)
                    .expect("a broken class should hold an error");
                Err(error)
            }
        }
    }

    /// The class itself, adding each broken method's error to `errors` and its bytes to
    /// `methods`.
    fn class_body(
        &mut self,
        errors: &mut Vec<Diagnostic>,
        methods: &mut Vec<Range<usize>>,
    ) -> Result<Class, Diagnostic> {
        self.expect(&TokenKind::Class)?;
        let name = self.name("a class name")?;
        self.expect(&TokenKind::LBrace)?;
        let mut members = Vec::new();
        let mut broken_methods = Vec::new();
        self.skip_newlines();
        while !self.at(&TokenKind::RBrace) {
            self.depth = 0;
            let start = self.pos;
            let end = self.item_end(start);
            match self.checked(end, Self::member_line) {
                Ok(member) => members.push(member),
                Err(error) => match self.header(start) {
                    Some(Broken {
                        name,
                        kind: BrokenKind::Function,
                    }) => {
                        methods.push(self.bytes(start, end));
                        broken_methods.push(name);
                        errors.push(error);
                    }
                    _ => return Err(error),
                },
            }
            self.skip_newlines();
        }
        self.bump();
        Ok(Class {
            name,
            members,
            broken_methods,
        })
    }

    /// A class member, and the end of its line.
    fn member_line(&mut self) -> Result<Member, Diagnostic> {
        let member = self.member()?;
        if matches!(member, Member::Field(_)) && self.at(&TokenKind::Comma) {
            return Err(DiagnosticKind::FieldComma {
                at: self.peek().span,
            }
            .into());
        }
        if !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Newline) {
            return Err(self.end_of_line());
        }
        Ok(member)
    }

    fn member(&mut self) -> Result<Member, Diagnostic> {
        match self.peek().kind {
            TokenKind::Ident(_) | TokenKind::Underscore | TokenKind::Mut => {
                let mutable = self.eat(&TokenKind::Mut).is_some();
                let name = self.name("a field name")?;
                self.expect(&TokenKind::Colon)?;
                Ok(Member::Field(FieldDecl {
                    mutable,
                    name,
                    ty: self.parse_type()?,
                }))
            }
            TokenKind::Let => Err(DiagnosticKind::FieldLet {
                at: self.peek().span,
                rest: self.field_after_let(),
            }
            .into()),
            TokenKind::Var => Err(DiagnosticKind::Var {
                at: self.peek().span,
                field: true,
            }
            .into()),
            TokenKind::Fn => {
                let method = self.function(true)?;
                if method.self_param.is_none() {
                    return Err(DiagnosticKind::MethodNeedsSelf {
                        at: method.name.span,
                    }
                    .into());
                }
                Ok(Member::Method(method))
            }
            _ => Err(self.expected("a field or `fn`")),
        }
    }

    /// What follows the `let` at the next token: a field that ends the line, or anything else,
    /// shown as `name: Type` or, when no field follows, that placeholder.
    fn field_after_let(&self) -> LetField {
        let mut rest = Parser {
            tokens: self.tokens,
            pos: self.pos + 1,
            depth: self.depth,
            lexer: self.lexer,
            comments: self.comments,
        };
        let mutable = rest.eat(&TokenKind::Mut).is_some();
        let mut field = || -> Result<String, Diagnostic> {
            let name = rest.ident("a field name")?;
            rest.expect(&TokenKind::Colon)?;
            Ok(format!("{}: {}", name.name, rest.parse_type()?))
        };
        match field() {
            Ok(_) if matches!(rest.peek().kind, TokenKind::Newline | TokenKind::RBrace) => {
                LetField::Whole {
                    remove: Span::new(self.peek().span.start, self.tokens[self.pos + 1].span.start),
                }
            }
            Ok(field) => LetField::Other { mutable, field },
            Err(_) => LetField::Other {
                mutable,
                field: "name: Type".to_string(),
            },
        }
    }

    /// `const name: T = value`, which ends the item at `end`.
    fn constant(&mut self, end: usize) -> Result<Const, Diagnostic> {
        self.bump();
        let name = self.name("a constant name")?;
        let ty = match self.eat(&TokenKind::Colon) {
            Some(_) => Some(self.parse_type()?),
            None => None,
        };
        self.depth = 0;
        self.expect(&TokenKind::Eq)?;
        let value = self.expr()?;
        if self.pos != end {
            return Err(self.end_of_line());
        }
        Ok(Const { name, ty, value })
    }

    /// `method` is true inside a class body, where `self` may be the first parameter.
    fn function(&mut self, method: bool) -> Result<Function, Diagnostic> {
        let fn_token = self.expect(&TokenKind::Fn)?;
        let name = self.name("a function name")?;
        self.expect(&TokenKind::LParen)?;
        let mut self_param = None;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                let receiver = method && self_param.is_none() && params.is_empty();
                if self.at(&TokenKind::Mut) {
                    // `mut` is never `Eof`, so the token after it exists.
                    match &self.tokens[self.pos + 1].kind {
                        TokenKind::SelfKw if receiver => {
                            // `self` is not `Eof`, so the token after it exists.
                            let after = &self.tokens[self.pos + 2].kind;
                            return Err(DiagnosticKind::MutSelf {
                                keyword: self.peek().span,
                                receiver: self.tokens[self.pos + 1].span,
                                certain: matches!(after, TokenKind::Comma | TokenKind::RParen),
                            }
                            .into());
                        }
                        TokenKind::SelfKw => {
                            self.bump();
                        }
                        TokenKind::Ident(name) => {
                            return Err(DiagnosticKind::MutParameter {
                                name: name.to_string(),
                                at: self.peek().span,
                            }
                            .into());
                        }
                        _ => {}
                    }
                }
                if self.at(&TokenKind::SelfKw) {
                    if !receiver {
                        return Err(DiagnosticKind::SelfNotFirst {
                            at: self.peek().span,
                        }
                        .into());
                    }
                    self_param = Some(self.bump().span);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                    continue;
                }
                let name = self.name("a parameter name")?;
                self.expect(&TokenKind::Colon)?;
                let ty = self.parse_type()?;
                self.depth = 0;
                params.push(Param { name, ty });
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen)?;
        let ret = match self.eat(&TokenKind::Arrow) {
            Some(_) => Some(self.parse_type()?),
            None => None,
        };
        self.depth = 0;
        let body = self.block()?;
        Ok(Function {
            name,
            self_param,
            params,
            ret,
            span: fn_token.span.to(body.span),
            body,
        })
    }

    fn block(&mut self) -> Result<Block, Diagnostic> {
        self.descend()?;
        let base = self.depth;
        let mut reach = base;
        let open = self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Else) {
                return Err(self.else_on_new_line(stmts.last()));
            }
            stmts.push(self.child(base, &mut reach, Self::stmt)?);
            if !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Newline) {
                return Err(self.end_of_line());
            }
            self.skip_newlines();
        }
        let close = self.bump();
        self.depth = reach;
        Ok(Block {
            stmts,
            span: open.span.to(close.span),
        })
    }

    fn stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.peek().span;
        let base = self.depth;
        let mut reach = base;
        let kind = match self.peek().kind {
            TokenKind::Let => self.let_stmt()?,
            TokenKind::Const => return Err(self.local_const()),
            TokenKind::While => {
                self.bump();
                if let Some((name, value)) = self.child(base, &mut reach, Self::let_binding)? {
                    StmtKind::WhileLet {
                        name,
                        value,
                        body: self.child(base, &mut reach, Self::block)?,
                    }
                } else {
                    let cond = self.child(base, &mut reach, Self::expr)?;
                    StmtKind::While {
                        cond,
                        body: self.child(base, &mut reach, Self::block)?,
                    }
                }
            }
            TokenKind::For => {
                self.bump();
                let name = self.binding()?;
                self.expect(&TokenKind::In)?;
                // A range is only valid here, so the header reads its two sides itself, counted
                // like `expr` does.
                let side = |p: &mut Self| {
                    p.descend()?;
                    p.binary(1)
                };
                let first = self.child(base, &mut reach, side)?;
                let source = if self.eat(&TokenKind::DotDot).is_some() {
                    ForSource::Range {
                        start: first,
                        end: self.child(base, &mut reach, side)?,
                    }
                } else {
                    ForSource::Array(first)
                };
                StmtKind::For {
                    name,
                    source,
                    body: self.child(base, &mut reach, Self::block)?,
                }
            }
            TokenKind::Return => {
                self.bump();
                if self.at(&TokenKind::Newline)
                    || self.at(&TokenKind::RBrace)
                    || self.at(&TokenKind::Semicolon)
                {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.child(base, &mut reach, Self::expr)?))
                }
            }
            TokenKind::Break => {
                self.bump();
                StmtKind::Break
            }
            TokenKind::Continue => {
                self.bump();
                StmtKind::Continue
            }
            TokenKind::LBrace => return Err(DiagnosticKind::BlockAsStatement { at: start }.into()),
            _ => self.expr_or_assign()?,
        };
        // `let_stmt` and `expr_or_assign` leave their own height in `self.depth`.
        self.depth = reach.max(self.depth);
        Ok(Stmt {
            kind,
            span: start.to(self.last_span()),
        })
    }

    /// The `else` that starts a line, after `previous`, the statement before it. The `else` can
    /// move up to the `if` it follows when that has none, and a block or an `if` follows it.
    fn else_on_new_line(&self, previous: Option<&Stmt>) -> Diagnostic {
        let at = self.peek().span;
        let lacks_else = previous.is_some_and(|stmt| {
            matches!(
                &stmt.kind,
                StmtKind::Expr(Expr {
                    kind: ExprKind::If {
                        else_block: None,
                        ..
                    } | ExprKind::IfLet {
                        else_block: None,
                        ..
                    },
                    ..
                })
            )
        });
        let takes_block = matches!(
            self.tokens[self.pos + 1].kind,
            TokenKind::LBrace | TokenKind::If
        );
        // `lacks_else` means the statement ends with the `}` just before the `else` and any
        // line breaks. The join deletes the gap, so a comment in it makes the join a guess.
        let gap = self.last_code_end()..at.start;
        let join = (lacks_else && takes_block).then(|| Join {
            gap: Span::new(gap.start, gap.end),
            certain: !self.comments.iter().any(|c| gap.contains(c)),
        });
        DiagnosticKind::ElseOnNewLine { at, join }.into()
    }

    /// Where the code before the next token ends: the `}` of the statement before it.
    fn last_code_end(&self) -> usize {
        self.tokens[..self.pos]
            .iter()
            .rfind(|t| t.kind != TokenKind::Newline)
            .map_or(0, |t| t.span.end)
    }

    /// The span of the token just consumed.
    fn last_span(&self) -> Span {
        self.tokens[self.pos - 1].span
    }

    /// `let name = value`, after `if` or `while`; `None` when no `let` follows.
    fn let_binding(&mut self) -> Result<Option<(Ident, Expr)>, Diagnostic> {
        if self.eat(&TokenKind::Let).is_none() {
            return Ok(None);
        }
        let name = self.ident("a binding name")?;
        self.expect(&TokenKind::Eq)?;
        Ok(Some((name, self.expr()?)))
    }

    fn let_stmt(&mut self) -> Result<StmtKind, Diagnostic> {
        let base = self.depth;
        let mut reach = base;
        self.bump();
        let mutable = self.eat(&TokenKind::Mut).map(|t| t.span);
        let name = self.binding()?;
        let ty = match self.eat(&TokenKind::Colon) {
            Some(_) => Some(self.child(base, &mut reach, Self::parse_type)?),
            None => None,
        };
        self.expect(&TokenKind::Eq)?;
        let init = self.child(base, &mut reach, Self::expr)?;
        self.depth = reach;
        // Reported last, and only where the statement ends, so that the fix leaves a `let` that
        // parses: past a stray token, the block reports that token's own error.
        if let Some(keyword) = mutable
            && name.name.as_str() == "_"
            && (self.at(&TokenKind::Newline) || self.at(&TokenKind::RBrace))
        {
            return Err(DiagnosticKind::MutOnUnderscore {
                at: name.span,
                keyword,
            }
            .into());
        }
        Ok(StmtKind::Let {
            mutable,
            name,
            ty,
            init,
        })
    }

    /// The error of a `const` statement. It is reported only once the statement parses as a `let`
    /// and ends the line, so that its fix leaves a `let` that parses; before that, the first
    /// error in it is reported.
    fn local_const(&mut self) -> Diagnostic {
        let at = self.peek().span;
        if let Err(error) = self.let_stmt() {
            return error;
        }
        if !self.at(&TokenKind::Newline) && !self.at(&TokenKind::RBrace) {
            return self.end_of_line();
        }
        DiagnosticKind::ConstInFunction { at }.into()
    }

    /// An expression statement, or an assignment when an assignment operator follows.
    fn expr_or_assign(&mut self) -> Result<StmtKind, Diagnostic> {
        if self.at(&TokenKind::Underscore) {
            return Err(self.underscore_statement());
        }
        let base = self.depth;
        let mut reach = base;
        let target = self.child(base, &mut reach, Self::expr)?;
        let op = match self.peek().kind {
            TokenKind::Eq => None,
            TokenKind::PlusEq => Some(BinaryOp::Add),
            TokenKind::MinusEq => Some(BinaryOp::Sub),
            TokenKind::StarEq => Some(BinaryOp::Mul),
            TokenKind::SlashEq => Some(BinaryOp::Div),
            TokenKind::PercentEq => Some(BinaryOp::Rem),
            _ => return Ok(StmtKind::Expr(target)),
        };
        if matches!(target.kind, ExprKind::SelfValue) {
            return Err(DiagnosticKind::AssignToSelf { at: target.span }.into());
        }
        if !is_place(&target) {
            return Err(DiagnosticKind::AssignToExpression { at: target.span }.into());
        }
        if goes_through_safe_access(&target) {
            return Err(DiagnosticKind::AssignThroughSafeAccess { at: target.span }.into());
        }
        let op_span = self.bump().span;
        let value = self.child(base, &mut reach, Self::expr)?;
        self.depth = reach;
        Ok(match op {
            None => StmtKind::Assign { target, value },
            Some(op) => StmtKind::CompoundAssign {
                op,
                op_span,
                target,
                value,
            },
        })
    }

    /// The error of a statement that starts with `_`. A plain `_ = e` whose value parses and ends
    /// the line gets the fix `let _ = e`; a value that fails to parse reports its own error.
    fn underscore_statement(&mut self) -> Diagnostic {
        let at = self.bump().span;
        let mut bind = false;
        if self.eat(&TokenKind::Eq).is_some() {
            if let Err(error) = self.expr() {
                return error;
            }
            bind = matches!(self.peek().kind, TokenKind::Newline | TokenKind::RBrace);
        }
        DiagnosticKind::UnderscoreValue { at, bind }.into()
    }

    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        self.descend()?;
        let e = self.binary(1)?;
        if self.at(&TokenKind::DotDot) {
            return Err(DiagnosticKind::RangeOutsideFor {
                at: self.peek().span,
            }
            .into());
        }
        Ok(e)
    }

    /// Precedence climbing over levels 1 to 7; `min` is the lowest level to accept.
    /// Each operator adds a level: `1 + 1 + 1` builds a tree that is as deep as it is long.
    fn binary(&mut self, min: u8) -> Result<Expr, Diagnostic> {
        let base = self.depth;
        let mut lhs = self.unary()?;
        loop {
            let kind = &self.peek().kind;
            if compare_op(kind).is_some() {
                if min > 4 {
                    return Ok(lhs);
                }
                lhs = self.comparison_chain(lhs, base)?;
            } else if let Some((level, op)) = infix_op(kind) {
                if level < min {
                    return Ok(lhs);
                }
                // The operator is the parent of everything parsed so far: one level more.
                let wrapped = self.depth + 1;
                self.reach(wrapped)?;
                let op_span = self.bump().span;
                self.depth = base + 1;
                // `??` is right-associative: its right side starts at its own level.
                let rhs = self.binary(if matches!(op, Infix::Coalesce) {
                    level
                } else {
                    level + 1
                })?;
                self.depth = self.depth.max(wrapped);
                let span = lhs.span.to(rhs.span);
                let (lhs_box, rhs_box) = (Box::new(lhs), Box::new(rhs));
                lhs = Expr {
                    span,
                    kind: match op {
                        Infix::Binary(op) => ExprKind::Binary {
                            op,
                            op_span,
                            lhs: lhs_box,
                            rhs: rhs_box,
                        },
                        Infix::Is => ExprKind::Is {
                            lhs: lhs_box,
                            rhs: rhs_box,
                        },
                        Infix::Coalesce => ExprKind::Coalesce {
                            lhs: lhs_box,
                            rhs: rhs_box,
                        },
                    },
                };
                // Level 3 is non-associative: a second `==`/`!=`/`is` is an error, not a chain.
                if level == 3
                    && matches!(
                        self.peek().kind,
                        TokenKind::Is | TokenKind::EqEq | TokenKind::NotEq
                    )
                {
                    return Err(DiagnosticKind::EqualityChain {
                        at: self.peek().span,
                    }
                    .into());
                }
            } else {
                return Ok(lhs);
            }
        }
    }

    /// Level 4: `first` and every `<`/`<=`/`>`/`>=` operand after it, as one node.
    fn comparison_chain(&mut self, first: Expr, base: usize) -> Result<Expr, Diagnostic> {
        let start = first.span;
        // The chain is the parent of `first`: one level more.
        let mut reach = self.depth + 1;
        self.reach(reach)?;
        let mut operands = vec![first];
        let mut ops: Vec<CompareOp> = Vec::new();
        while let Some(op) = compare_op(&self.peek().kind) {
            if ops
                .first()
                .is_some_and(|first| first.is_less() != op.is_less())
            {
                return Err(DiagnosticKind::ComparisonDirection {
                    at: self.peek().span,
                }
                .into());
            }
            self.bump();
            ops.push(op);
            self.depth = base + 1;
            operands.push(self.binary(5)?);
            reach = reach.max(self.depth);
        }
        self.depth = reach;
        Ok(Expr {
            span: start.to(self.last_span()),
            kind: ExprKind::Compare { operands, ops },
        })
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        let op = match self.peek().kind {
            TokenKind::Minus => UnaryOp::Neg,
            TokenKind::Bang => UnaryOp::Not,
            _ => return self.postfix(),
        };
        self.descend()?;
        let start = self.bump().span;
        if matches!(op, UnaryOp::Neg) && self.at_min_literal() {
            // `i64::MIN` has no positive twin, so `-9223372036854775808` is one literal.
            let TokenKind::Int { text, .. } = self.peek().kind.clone() else {
                unreachable!("`at_min_literal` saw an `Int`")
            };
            let literal = self.bump().span;
            return Ok(Expr {
                span: start.to(literal),
                kind: ExprKind::Int {
                    value: i64::MIN,
                    text: format!("-{text}"),
                },
            });
        }
        let operand = self.unary()?;
        Ok(Expr {
            span: start.to(operand.span),
            kind: ExprKind::Unary {
                op,
                operand: Box::new(operand),
            },
        })
    }

    /// Whether the next token is the magnitude of `i64::MIN` that nothing postfix follows. A `.`,
    /// `?.` or `[` would apply to the literal before the `-` does, so that literal is too large.
    fn at_min_literal(&self) -> bool {
        // `Int` is never `Eof`, so the token after it exists.
        matches!(
            self.peek().kind,
            TokenKind::Int {
                value: MIN_MAGNITUDE,
                ..
            }
        ) && !starts_postfix(&self.tokens[self.pos + 1].kind)
    }

    /// `primary` followed by any number of `.field`, `.method(args)`, `?.field`,
    /// `?.method(args)` and `[index]`.
    fn postfix(&mut self) -> Result<Expr, Diagnostic> {
        let base = self.depth;
        let mut expr = self.primary()?;
        loop {
            // Each link is the parent of everything parsed so far: one level more.
            let wrapped = self.depth + 1;
            if self.at(&TokenKind::LBracket) {
                self.reach(wrapped)?;
                self.bump();
                self.depth = base + 1;
                let index = self.expr()?;
                let close = self.expect(&TokenKind::RBracket)?;
                self.depth = self.depth.max(wrapped);
                expr = Expr {
                    span: expr.span.to(close.span),
                    kind: ExprKind::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                    },
                };
                continue;
            }
            // `[` was handled above, so only `.` and `?.` reach here.
            if !starts_postfix(&self.peek().kind) {
                return Ok(expr);
            }
            self.reach(wrapped)?;
            let access = self.bump();
            let safe = access.kind == TokenKind::QuestionDot;
            let name = self.ident("a field or method name")?;
            expr = if self.at(&TokenKind::LParen) {
                self.depth = base + 1;
                let (args, close) = self.args()?;
                self.depth = self.depth.max(wrapped);
                Expr {
                    span: expr.span.to(close),
                    kind: ExprKind::MethodCall {
                        receiver: Box::new(expr),
                        method: name,
                        args,
                        safe,
                    },
                }
            } else {
                Expr {
                    span: expr.span.to(name.span),
                    kind: ExprKind::Field {
                        base: Box::new(expr),
                        name,
                        safe,
                    },
                }
            };
        }
    }

    /// `[` after the opening bracket: `]`, `value; len]` or `a, b, …]`.
    fn array(&mut self, open: Span) -> Result<Expr, Diagnostic> {
        let base = self.depth;
        let mut reach = base;
        let mut elements = Vec::new();
        let mut fill = None;
        while !self.at(&TokenKind::RBracket) {
            elements.push(self.child(base, &mut reach, Self::expr)?);
            if elements.len() == 1 && self.eat(&TokenKind::Semicolon).is_some() {
                fill = Some(self.child(base, &mut reach, Self::expr)?);
                break;
            }
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        let close = self.expect(&TokenKind::RBracket)?;
        self.depth = reach;
        let kind = match (fill, elements.pop()) {
            (Some(len), Some(value)) => ExprKind::ArrayFill {
                value: Box::new(value),
                len: Box::new(len),
            },
            (_, last) => {
                elements.extend(last);
                ExprKind::ArrayLit(elements)
            }
        };
        Ok(Expr {
            kind,
            span: open.to(close.span),
        })
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.peek().clone();
        let kind = match token.kind {
            TokenKind::Int { value, text } => {
                // `MIN_MAGNITUDE` fits only as the operand of a `-`, which `unary` takes first.
                let value = i64::try_from(value).map_err(|_| literal_too_large(token.span))?;
                self.bump();
                ExprKind::Int { value, text }
            }
            TokenKind::True | TokenKind::False => {
                self.bump();
                ExprKind::Bool(token.kind == TokenKind::True)
            }
            TokenKind::Char(c) => {
                self.bump();
                ExprKind::Char(c)
            }
            TokenKind::Str(s) => {
                self.bump();
                ExprKind::Str(s)
            }
            TokenKind::StrHead(head) => {
                self.bump();
                return self.interpolation(token.span, head);
            }
            TokenKind::SelfKw => {
                self.bump();
                ExprKind::SelfValue
            }
            TokenKind::NoneKw => {
                self.bump();
                ExprKind::NoneLit
            }
            TokenKind::Ident(name) => {
                self.bump();
                if self.at(&TokenKind::LParen) {
                    let (args, close) = self.args()?;
                    return Ok(Expr {
                        span: token.span.to(close),
                        kind: ExprKind::Call {
                            callee: Ident {
                                name,
                                span: token.span,
                            },
                            args,
                        },
                    });
                }
                ExprKind::Name(name)
            }
            TokenKind::Todo => return self.todo(),
            TokenKind::LParen => {
                self.bump();
                let inner = self.expr()?;
                let close = self.expect(&TokenKind::RParen)?;
                // The parentheses are part of the span, so diagnostics underline them.
                return Ok(Expr {
                    span: token.span.to(close.span),
                    ..inner
                });
            }
            TokenKind::LBracket => {
                self.bump();
                return self.array(token.span);
            }
            TokenKind::If => return self.if_expr(),
            TokenKind::LBrace => {
                let block = self.block()?;
                return Ok(Expr {
                    span: block.span,
                    kind: ExprKind::Block(block),
                });
            }
            TokenKind::Underscore => {
                return Err(DiagnosticKind::UnderscoreValue {
                    at: token.span,
                    bind: false,
                }
                .into());
            }
            _ => return Err(self.expected("an expression")),
        };
        Ok(Expr {
            kind,
            span: token.span,
        })
    }

    /// `todo` or `todo(args)`. Rust's `todo!` and `todo!(...)`, and an empty `todo()`, are
    /// reported with the fix that deletes what is extra.
    fn todo(&mut self) -> Result<Expr, Diagnostic> {
        let keyword = self.bump().span;
        let (tokens, pos) = (self.tokens, self.pos);
        let next = |n: usize| &tokens[(pos + n).min(tokens.len() - 1)];
        let (at, delete) = match (&next(0).kind, &next(1).kind, &next(2).kind) {
            (TokenKind::Bang, TokenKind::LParen, TokenKind::RParen) => {
                (next(0).span, next(0).span.to(next(2).span))
            }
            (TokenKind::Bang, ..) => (next(0).span, next(0).span),
            (TokenKind::LParen, TokenKind::RParen, _) => {
                (next(0).span, next(0).span.to(next(1).span))
            }
            (TokenKind::LParen, ..) => {
                let (args, close) = self.args()?;
                return Ok(Expr {
                    kind: ExprKind::Todo(args),
                    span: keyword.to(close),
                });
            }
            _ => {
                return Ok(Expr {
                    kind: ExprKind::Todo(Vec::new()),
                    span: keyword,
                });
            }
        };
        Err(DiagnosticKind::TodoForm { at, delete }.into())
    }

    /// The rest of a string literal with interpolations, after its `StrHead` at `start` with the
    /// text `head`: each expression, and the text after it, up to the `StrTail`.
    fn interpolation(&mut self, start: Span, head: String) -> Result<Expr, Diagnostic> {
        let text = |value: String, span| {
            (!value.is_empty()).then_some(Expr {
                kind: ExprKind::Str(value),
                span,
            })
        };
        let base = self.depth;
        let mut reach = base;
        let mut parts: Vec<Expr> = text(head, start).into_iter().collect();
        let end = loop {
            parts.push(self.child(base, &mut reach, Self::expr)?);
            let token = self.peek().clone();
            match token.kind {
                TokenKind::StrMiddle(value) => parts.extend(text(value, token.span)),
                TokenKind::StrTail(value) => {
                    parts.extend(text(value, token.span));
                    self.bump();
                    break token.span;
                }
                _ => return Err(self.expected("`}`")),
            }
            self.bump();
        };
        self.depth = reach;
        Ok(Expr {
            kind: ExprKind::Interpolate(parts),
            span: start.to(end),
        })
    }

    /// `(` arguments `)`, and the span of the `)`.
    fn args(&mut self) -> Result<(Vec<Arg>, Span), Diagnostic> {
        self.expect(&TokenKind::LParen)?;
        let base = self.depth;
        let mut reach = base;
        let mut args = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                args.push(self.child(base, &mut reach, Self::arg)?);
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect(&TokenKind::RParen)?;
        self.depth = reach;
        Ok((args, close.span))
    }

    /// A label is an `Ident` followed by `:`, so it takes two tokens of lookahead.
    fn arg(&mut self) -> Result<Arg, Diagnostic> {
        let labeled = matches!(self.peek().kind, TokenKind::Ident(_))
            && self
                .tokens
                .get(self.pos + 1)
                .is_some_and(|t| t.kind == TokenKind::Colon);
        let label = if labeled {
            let label = self.ident("a label")?;
            self.bump();
            Some(label)
        } else {
            None
        };
        Ok(Arg {
            label,
            value: self.expr()?,
        })
    }

    fn if_expr(&mut self) -> Result<Expr, Diagnostic> {
        let start = self.expect(&TokenKind::If)?.span;
        let base = self.depth;
        let mut reach = base;
        let head = match self.child(base, &mut reach, Self::let_binding)? {
            Some((name, value)) => Head::Let(name, value),
            None => Head::Cond(self.child(base, &mut reach, Self::expr)?),
        };
        let then_block = self.child(base, &mut reach, Self::block)?;
        let mut end = then_block.span;
        let else_block = if self.eat(&TokenKind::Else).is_some() {
            let block = if self.at(&TokenKind::If) {
                // `else if` and `else if let` become a block holding the inner `if`.
                let inner = self.child(base, &mut reach, |p| {
                    p.descend()?;
                    p.if_expr()
                })?;
                Block {
                    span: inner.span,
                    stmts: vec![Stmt {
                        span: inner.span,
                        kind: StmtKind::Expr(inner),
                    }],
                }
            } else {
                self.child(base, &mut reach, Self::block)?
            };
            end = block.span;
            Some(block)
        } else {
            None
        };
        let kind = match head {
            Head::Cond(cond) => ExprKind::If {
                cond: Box::new(cond),
                then_block,
                else_block,
            },
            Head::Let(name, value) => ExprKind::IfLet {
                name,
                value: Box::new(value),
                then_block,
                else_block,
            },
        };
        self.depth = reach;
        Ok(Expr {
            span: start.to(end),
            kind,
        })
    }
}

/// A name, a field of any expression, or an index of a place.
fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Name(_) | ExprKind::Field { .. } => true,
        ExprKind::Index { base, .. } => is_place(base),
        _ => false,
    }
}

/// Whether a `?.` sits anywhere in the chain of fields, calls and indexes that makes up `e`.
fn goes_through_safe_access(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Field { base, safe, .. } => *safe || goes_through_safe_access(base),
        ExprKind::MethodCall { receiver, safe, .. } => *safe || goes_through_safe_access(receiver),
        ExprKind::Index { base, .. } => goes_through_safe_access(base),
        _ => false,
    }
}

/// What follows `if`: a condition, or the `let name = value` of an `if let`.
enum Head {
    Cond(Expr),
    Let(Ident, Expr),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::explanations::{apply_fix, fix_edits};
    use crate::diagnostic::{Code, line_col};
    use crate::lexer::lex;

    fn program(src: &str) -> String {
        parse_ok(src).to_string()
    }

    /// The first item, which must be a function.
    fn first_fn(src: &str) -> Function {
        match parse_ok(src).items.into_iter().next() {
            Some(Item::Function(f)) => f,
            other => panic!("expected a function, found {other:?}"),
        }
    }

    fn body(src: &str) -> String {
        first_fn(src).body.to_string()
    }

    /// The parse of `src`, which must lex without a bracket error.
    fn parsed(src: &str) -> Parsed {
        parse(lex(src).expect("source lexes"))
    }

    /// The one diagnostic `src` reports.
    fn parse_diagnostic(src: &str) -> Diagnostic {
        let parsed = parsed(src);
        let [d] = parsed.diagnostics.as_slice() else {
            panic!(
                "{src}: expected one diagnostic, got {:?}",
                parsed.diagnostics
            );
        };
        d.clone()
    }

    fn parse_err(src: &str) -> (Code, usize, usize) {
        let d = parse_diagnostic(src);
        let (line, col) = line_col(src, d.span().start);
        (d.code(), line, col)
    }

    /// `src` with the one fix of its parse error applied, which must be certain.
    fn fixed(src: &str) -> String {
        let d = parse_diagnostic(src);
        let (certain, _) = fix_edits(&d).expect("the error has a fix");
        assert!(certain, "{src}");
        apply_fix(src, &d)
    }

    #[test]
    fn function_with_let() {
        assert_eq!(
            program("fn main() {\n    let x = 1\n}"),
            "(fn main () unit (block (let x 1)))"
        );
    }

    #[test]
    fn params_and_return_type() {
        assert_eq!(
            program("fn f(a: i64, b: bool) -> i64 { a }"),
            "(fn f ((a i64) (b bool)) i64 (block a))"
        );
    }

    /// Each "expected X" error at the token found instead. The first four name a place in the
    /// grammar and the last three the token itself; the `E0011-expected-*` fixtures pin each text.
    #[test]
    fn expectation_errors_sit_on_the_token_found() {
        for (src, line, col) in [
            ("class {}", 1, 7),
            ("class A {\n    mut 1: i64\n}", 2, 9),
            ("fn f(a: ) {}", 1, 9),
            ("fn main() {\n    print([1].0)\n}", 2, 15),
            ("fn f(a) {}", 1, 7),
            ("fn main() {\n    for i 0..3 {\n    }\n}", 2, 11),
            ("fn f {}", 1, 6),
        ] {
            assert_eq!(parse_err(src), (Code::Expected, line, col), "{src}");
        }
    }

    /// A `char` literal where an expression ends is the unexpected token.
    #[test]
    fn char_is_an_unexpected_token() {
        assert_eq!(
            parse_err("fn main() {\n    let b = 'x' 'y'\n}"),
            (Code::Expected, 2, 17)
        );
    }

    /// A `-` applied directly to 2^63 is the one literal `i64::MIN`, with the `-` in its span.
    #[test]
    fn minus_min_magnitude_is_one_literal() {
        assert_eq!(
            body("fn main() { -9223372036854775808 }"),
            "(block -9223372036854775808)"
        );
        assert_eq!(
            body("fn main() { -   9_223_372_036_854_775_808 }"),
            "(block -9223372036854775808)"
        );
        let f = first_fn("fn main() { 1 - -9223372036854775808 }");
        let StmtKind::Expr(sum) = &f.body.stmts[0].kind else {
            panic!("expected an expression statement")
        };
        assert_eq!(sum.to_string(), "(- 1 -9223372036854775808)");
        assert_eq!((sum.span.start, sum.span.end), (12, 36));
    }

    /// The magnitude of `i64::MIN` is `i64::MIN` after a `-` in any base, and too large without.
    #[test]
    fn min_magnitude_in_hex() {
        assert_eq!(
            body("fn main() { -0x8000_0000_0000_0000 }"),
            "(block -9223372036854775808)"
        );
        assert_eq!(
            parse_err("fn main() { 0x8000_0000_0000_0000 }"),
            (Code::IntegerTooLarge, 1, 13)
        );
    }

    /// Anywhere else 2^63 is the one "too large" error, at the literal. The lexer rejects larger.
    #[test]
    fn min_magnitude_elsewhere_is_too_large() {
        for (src, col) in [
            ("fn main() { 9223372036854775808 }", 13),
            ("fn main() { -(9223372036854775808) }", 15),
            ("fn main() { 1 - 9223372036854775808 }", 17),
            // The `.` and `[` bind tighter than the `-`, so they apply to the literal.
            ("fn main() { -9223372036854775808.x }", 14),
            ("fn main() { -9223372036854775808?.x }", 14),
            ("fn main() { -9223372036854775808[0] }", 14),
        ] {
            assert_eq!(parse_err(src), (Code::IntegerTooLarge, 1, col), "{src}");
        }
    }

    #[test]
    fn interpolation_s_expression() {
        assert_eq!(
            expr(r#"print("a{x}b{ "c" + y }")"#),
            r#"(call print (interp "a" x "b" (+ "c" y)))"#
        );
        // Empty text pieces are dropped.
        assert_eq!(expr(r#"print("{x}{y}")"#), "(call print (interp x y))");
    }

    /// An interpolation holds one expression, which ends at its `}`.
    #[test]
    fn interpolation_errors() {
        for (src, col) in [(r#""{x y}""#, 17), (r#""{x +}""#, 18)] {
            assert_eq!(expr_err(src), (Code::Expected, 1, col), "{src}");
        }
    }

    #[test]
    fn char_s_expression() {
        assert_eq!(
            body("fn main() {\n    let c = 'ä'\n    let n = '\\n'\n}"),
            "(block (let c 'ä') (let n '\\n'))"
        );
    }

    #[test]
    fn typed_let_and_let_mut() {
        assert_eq!(
            body("fn main() {\n    let x: i64 = 1\n    let mut y = 2\n}"),
            "(block (let x i64 1) (let mut y 2))"
        );
    }

    #[test]
    fn function_span() {
        assert_eq!(first_fn("fn main() {\n}\n").span, Span::new(0, 13));
    }

    #[test]
    fn return_without_value_before_brace() {
        assert_eq!(body("fn main() { return }"), "(block (return))");
    }

    #[test]
    fn return_with_value() {
        assert_eq!(
            body("fn f() -> i64 {\n    return 1\n}"),
            "(block (return 1))"
        );
    }

    #[test]
    fn break_and_continue() {
        assert_eq!(
            body("fn main() {\n    while c {\n        break\n        continue\n    }\n}"),
            "(block (while c (block (break) (continue))))"
        );
    }

    #[test]
    fn while_loop() {
        assert_eq!(
            body("fn main() {\n    while c { f() }\n}"),
            "(block (while c (block (call f))))"
        );
    }

    #[test]
    fn else_if_chain() {
        assert_eq!(
            body("fn main() { if a { 1 } else if b { 2 } else { 3 } }"),
            "(block (if a (block 1) (block (if b (block 2) (block 3)))))"
        );
    }

    #[test]
    fn multiline_call_args() {
        assert_eq!(
            body("fn main() {\n    f(1,\n      2)\n}"),
            "(block (call f 1 2))"
        );
    }

    #[test]
    fn else_on_new_line_fix() {
        // After an `if` or `if let` without `else`, the gap after the `}` becomes one space.
        for (src, position, gap) in [
            (
                "fn main() {\n    if a {\n    }\n    else {\n    }\n}",
                (4, 5),
                (28, 33),
            ),
            (
                "fn main() {\n    if let n = x {\n    }\n    else {\n    }\n}",
                (4, 5),
                (36, 41),
            ),
            (
                "fn main() {\n    if a {}\n    else if b {}\n}",
                (3, 5),
                (23, 28),
            ),
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::ElseOnNewLine, "{src}");
            assert_eq!(line_col(src, d.span().start), position, "{src}");
            assert_eq!(
                fix_edits(&d),
                Some((true, vec![(gap.0, gap.1, " ".to_string())])),
                "{src}"
            );
        }
    }

    #[test]
    fn certain_else_fix_removes_the_error() {
        let src = "fn main() {\n    if let n = x {\n        print(n)\n    }\n    else if b {\n        print(2)\n    }\n}\n";
        let result = fixed(src);
        assert_eq!(
            result,
            "fn main() {\n    if let n = x {\n        print(n)\n    } else if b {\n        print(2)\n    }\n}\n"
        );
        parse_ok(&result);
    }

    #[test]
    fn else_on_new_line_is_a_guess_over_a_comment() {
        // The join would delete the comment.
        let src = "fn main() {\n    if a {\n    } // done\n    else {\n    }\n}";
        let d = parse_diagnostic(src);
        assert_eq!(
            (d.code(), line_col(src, d.span().start)),
            (Code::ElseOnNewLine, (4, 5))
        );
        assert!(fix_edits(&d).is_some_and(|(certain, _)| !certain));
    }

    #[test]
    fn else_on_new_line_has_no_fix_after_anything_else() {
        for src in [
            // A loop's `}` cannot take an `else`.
            "fn main() {\n    while a {\n    }\n    else {\n    }\n}",
            // The `if` already has its `else`.
            "fn main() {\n    if a {\n    } else {\n    }\n    else {\n    }\n}",
            "fn main() {\n    if a {\n    } else if b {\n    }\n    else {\n    }\n}",
            // Nothing before it.
            "fn main() {\n    else {\n    }\n}",
            "fn main() {\n    print(1)\n    else {\n    }\n}",
            // The `else` has no block to open.
            "fn main() {\n    if a {\n    }\n    else b\n}",
            // Only an `if` statement can take it: not a `let`, nor the `if` inside one.
            "fn main() {\n    let x = 1\n    else {\n    }\n}",
            "fn main() {\n    let x = if a { 1 }\n    else { 2 }\n}",
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::ElseOnNewLine, "{src}");
            assert_eq!(fix_edits(&d), None, "{src}");
        }
    }

    #[test]
    fn two_statements_on_one_line() {
        assert_eq!(
            parse_err("fn main() { let x = 1 2 }"),
            (Code::Expected, 1, 23)
        );
    }

    #[test]
    fn unexpected_eof() {
        let src = "fn\n";
        assert_eq!(parse_err(src), (Code::Expected, 1, 3));
    }

    #[test]
    fn top_level_statement() {
        assert_eq!(parse_err("let x = 1"), (Code::Expected, 1, 1));
    }

    /// Parses `fn main() { <src> }` and prints the first statement.
    fn expr(src: &str) -> String {
        first_fn(&format!("fn main() {{ {src} }}")).body.stmts[0].to_string()
    }

    /// Error code, line and column for `fn main() { <src> }`; the columns start at 13.
    fn expr_err(src: &str) -> (Code, usize, usize) {
        parse_err(&format!("fn main() {{ {src} }}"))
    }

    #[test]
    fn new_tokens_are_errors_where_they_do_not_belong() {
        // `self` is an expression now, so its row hits a different error.
        for src in [
            "class",
            "is",
            "let self = 1",
            "let None = 1",
            ".",
            "?",
            "?.",
            "??",
        ] {
            assert_eq!(expr_err(src).0, Code::Expected, "{src}");
        }
    }

    #[test]
    fn expression_shapes() {
        let rows = [
            ("1 + 2 * 3", "(+ 1 (* 2 3))"),
            ("1 - 2 - 3", "(- (- 1 2) 3)"),
            ("-a * b", "(* (- a) b)"),
            ("!a && b || c", "(|| (&& (! a) b) c)"),
            ("(1 + 2) * 3", "(* (+ 1 2) 3)"),
            ("f(1, g(2))", "(call f 1 (call g 2))"),
            ("a < b", "(< a b)"),
            ("a < b <= c", "(< a b <= c)"),
            ("a > b >= c", "(> a b >= c)"),
            ("a == b < c", "(== a (< b c))"),
            ("a != b", "(!= a b)"),
            ("(a < b) < c", "(< (< a b) c)"),
            ("true && false", "(&& true false)"),
            ("x += 1", "(+= x 1)"),
            ("x %= 2", "(%= x 2)"),
            ("x -= 1", "(-= x 1)"),
            ("x *= 2", "(*= x 2)"),
            ("x /= 2", "(/= x 2)"),
            ("(x) = 1", "(= x 1)"),
            ("a.b.c", "(. (. a b) c)"),
            ("t.insert(1, 2)", "(call (. t insert) 1 2)"),
            (
                "Tree(value: 1, left: x)",
                "(call Tree (: value 1) (: left x))",
            ),
            ("-a.b", "(- (. a b))"),
            ("E()", "(call E)"),
            ("a[i].f = v", "(= (. (index a i) f) v)"),
            ("make().count = 1", "(= (. (call make) count) 1)"),
            ("a.b[i].c[j] = v", "(= (index (. (index (. a b) i) c) j) v)"),
            (
                "make().cells[0] = 1",
                "(= (index (. (call make) cells) 0) 1)",
            ),
            ("a.f += 1", "(+= (. a f) 1)"),
            ("self.left = t", "(= (. self left) t)"),
            ("None", "None"),
            ("f(None)", "(call f None)"),
            ("x ?? 0 < 5", "(< (?? x 0) 5)"),
            ("x ?? 0 + 1", "(?? x (+ 0 1))"),
            ("a ?? b ?? 0", "(?? a (?? b 0))"),
            ("x ?? 0 == 1", "(== (?? x 0) 1)"),
            ("a?.b?.c()", "(call (?. (?. a b) c))"),
            ("a?.b.c", "(. (?. a b) c)"),
            ("!(a is b)", "(! (is a b))"),
            ("a == b && c is d", "(&& (== a b) (is c d))"),
        ];
        for (src, expected) in rows {
            assert_eq!(expr(src), expected, "{src}");
        }
    }

    #[test]
    fn parenthesized_primary() {
        assert_eq!(body("fn main() { (f()) }"), "(block (call f))");
    }

    #[test]
    fn block_expression_positions() {
        let rows = [
            ("let a = { 1 }", "(let a (block 1))"),
            ("f({ 1 })", "(call f (block 1))"),
            ("1 + { 2 }", "(+ 1 (block 2))"),
            ("if { c } { }", "(if (block c) (block))"),
            ("while { c } { }", "(while (block c) (block))"),
            ("for x in { xs } { }", "(for x (block xs) (block))"),
        ];
        for (src, expected) in rows {
            assert_eq!(expr(src), expected, "{src}");
        }
    }

    #[test]
    fn block_statement() {
        for (src, line, col) in [
            ("fn main() {\n    { 1 }\n}", 2, 5),
            ("fn main() {\n    if c { {1} }\n}", 2, 12),
        ] {
            assert_eq!(parse_err(src), (Code::BlockAsStatement, line, col), "{src}");
        }
    }

    #[test]
    fn compound_assignment_spans() {
        let tree = first_fn("fn main() { x += 1 }");
        let stmt = &tree.body.stmts[0];
        assert_eq!(stmt.span, Span::new(12, 18));
        assert!(
            matches!(stmt.kind, StmtKind::CompoundAssign { .. }),
            "{stmt}"
        );
    }

    #[test]
    fn opposite_comparison_directions() {
        assert_eq!(expr_err("a < b > c"), (Code::ComparisonDirection, 1, 19));
    }

    #[test]
    fn mixed_chain_directions_report_at_the_second_operator() {
        assert_eq!(expr_err("a > b <= c"), (Code::ComparisonDirection, 1, 19));
    }

    #[test]
    fn mixed_equality_operators_do_not_chain() {
        assert_eq!(expr_err("a == b != c"), (Code::EqualityChain, 1, 20));
    }

    #[test]
    fn equality_does_not_chain() {
        assert_eq!(
            parse_err("fn main() { a == b == c }"),
            (Code::EqualityChain, 1, 20)
        );
    }

    #[test]
    fn is_does_not_chain() {
        assert_eq!(
            parse_err("fn main() { print(a is b is c) }"),
            (Code::EqualityChain, 1, 26)
        );
    }

    #[test]
    fn mixed_is_and_equality_do_not_chain() {
        assert_eq!(expr_err("a is b == c"), (Code::EqualityChain, 1, 20));
        assert_eq!(expr_err("a == b is c"), (Code::EqualityChain, 1, 20));
    }

    #[test]
    fn cannot_assign_through_safe_access() {
        for src in [
            "a?.b = 1",
            "a?.b.c = 1",
            "a?.b += 1",
            "a?.m().c = 1",
            "a?.b.m().c = 1",
            "xs?.items[0].f = 1",
            "xs?.items[0].f += 1",
            "xs?.a[0] = 1",
        ] {
            let full = format!("fn main() {{ {src} }}");
            assert_eq!(
                parse_err(&full),
                (Code::AssignThroughSafeAccess, 1, 13),
                "{src}"
            );
        }
    }

    #[test]
    fn cannot_assign_to_self() {
        for src in ["self = 1", "self += 1"] {
            let full = format!("fn main() {{ {src} }}");
            assert_eq!(parse_err(&full), (Code::AssignToSelf, 1, 13), "{src}");
            let span = parse_diagnostic(&full).span();
            assert_eq!(span.end - span.start, 4, "{src}");
        }
    }

    #[test]
    fn cannot_assign_to_expression() {
        for src in ["1 + 2 = 3", "f()[0] = 1"] {
            assert_eq!(expr_err(src), (Code::AssignToExpression, 1, 13), "{src}");
        }
    }

    #[test]
    fn underscore_errors() {
        for (src, code, line, col) in [
            (
                "fn main() {\n    let mut _ = 1\n}",
                Code::MutOnUnderscore,
                2,
                13,
            ),
            ("fn main() {\n    print(_)\n}", Code::UnderscoreValue, 2, 11),
            ("fn main() {\n    _ = f()\n}", Code::UnderscoreValue, 2, 5),
            ("fn main() {\n    _ += 1\n}", Code::UnderscoreValue, 2, 5),
            ("fn _() {}", Code::UnderscoreName, 1, 4),
            ("class _ {}", Code::UnderscoreName, 1, 7),
            ("class C {\n    _: i64\n}", Code::UnderscoreName, 2, 5),
            ("class C {\n    mut _: i64\n}", Code::UnderscoreName, 2, 9),
            ("fn f(_: i64) {}", Code::UnderscoreName, 1, 6),
            ("fn f(a: i64, _: i64) {}", Code::UnderscoreName, 1, 14),
            ("fn main() {\n    if let _ = e {}\n}", Code::Expected, 2, 12),
            (
                "fn main() {\n    while let _ = e {}\n}",
                Code::Expected,
                2,
                15,
            ),
        ] {
            assert_eq!(parse_err(src), (code, line, col), "{src}");
        }
    }

    #[test]
    fn const_items() {
        for (src, want) in [
            ("const A = 1", "(const A 1)"),
            ("const B: u8 = 240", "(const B u8 240)"),
            // A constant is one line, so a `}` in its value does not end it.
            (
                "const C = if true { 1 } else { 2 }",
                "(const C (if true (block 1) (block 2)))",
            ),
        ] {
            assert_eq!(program(src), want, "{src}");
        }
        for (src, code, line, col) in [
            ("fn f() { const C = 1 }", Code::ConstInFunction, 1, 10),
            (
                "fn f() {\n    const C: u8 = 1\n}",
                Code::ConstInFunction,
                2,
                5,
            ),
            ("const _ = 1", Code::UnderscoreName, 1, 7),
            ("const A = 1 2", Code::Expected, 1, 13),
            ("const A 1", Code::Expected, 1, 9),
            ("class C {\n    const A = 1\n}", Code::Expected, 2, 5),
            // A `const` statement that does not parse as a `let`, or does not end its line,
            // reports that error first, so the fix to `let` is offered only when it parses.
            ("fn f() { const C }", Code::Expected, 1, 18),
            ("fn f() { const C = 1 2 }", Code::Expected, 1, 22),
        ] {
            assert_eq!(parse_err(src), (code, line, col), "{src}");
        }
    }

    #[test]
    fn const_statement_fix_leaves_a_let_that_parses() {
        for (src, want) in [
            ("fn f() { const C = 1 }", "fn f() { let C = 1 }"),
            ("fn f() { const mut C = 1 }", "fn f() { let mut C = 1 }"),
            ("fn f() { const _: u8 = 1 }", "fn f() { let _: u8 = 1 }"),
        ] {
            let fixed = fixed(src);
            assert_eq!(fixed, want);
            assert!(parsed(&fixed).diagnostics.is_empty(), "{fixed}");
        }
    }

    #[test]
    fn underscore_binds_after_let_and_for() {
        for src in [
            "fn main() { let _ = 1 }",
            "fn main() { let _: i64 = 1 }",
            "fn main() { for _ in 0..3 {} }",
            "fn main() { for _ in xs {} }",
        ] {
            assert!(parsed(src).diagnostics.is_empty(), "{src}");
        }
    }

    #[test]
    fn mut_on_underscore_fix_leaves_a_let_that_parses() {
        for (src, want) in [
            ("fn main() { let mut _ = 1 }", "fn main() { let _ = 1 }"),
            (
                "fn main() { let mut _: i64 = 1 }",
                "fn main() { let _: i64 = 1 }",
            ),
        ] {
            let fixed = fixed(src);
            assert_eq!(fixed, want);
            assert!(parsed(&fixed).diagnostics.is_empty(), "{fixed}");
        }
        // A `let` that does not parse, or does not end its line, reports its own error first and
        // offers no fix: applying one would leave the same error behind.
        assert_eq!(
            parse_err("fn main() { let mut _ }"),
            (Code::Expected, 1, 23)
        );
        assert_eq!(
            parse_err("fn main() { let mut _ = 1 2 }"),
            (Code::Expected, 1, 27)
        );
        assert_eq!(
            parse_err("fn main() { let mut _ = 1; }"),
            (Code::Semicolon, 1, 26)
        );
        // The fixed source reports the same error, four columns (`mut `) earlier.
        assert_eq!(
            parse_err("fn main() { let _ = 1 2 }"),
            (Code::Expected, 1, 23)
        );
        assert_eq!(
            parse_err("fn main() { let _ = 1; }"),
            (Code::Semicolon, 1, 22)
        );
    }

    #[test]
    fn assigning_to_underscore_offers_let_only_when_the_value_parses() {
        for (src, want) in [
            ("fn main() { _ = f() }", "fn main() { let _ = f() }"),
            ("fn main() {\n    _ = 1\n}", "fn main() {\n    let _ = 1\n}"),
        ] {
            let fixed = fixed(src);
            assert_eq!(fixed, want);
            assert!(parsed(&fixed).diagnostics.is_empty(), "{fixed}");
        }
        for src in [
            "fn main() { _ += 1 }",
            "fn main() { _[0] = 1 }",
            "fn main() { _ == 1 }",
            "fn main() { _ = 1; }",
            "fn main() { _ = 1 2 }",
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::UnderscoreValue, "{src}");
            assert!(fix_edits(&d).is_none(), "{src}");
        }
        // The value's own error comes first.
        assert_eq!(parse_err("fn main() { _ = 1 + }"), (Code::Expected, 1, 21));
    }

    #[test]
    fn missing_initializer_at_end_of_line() {
        assert_eq!(
            parse_err("fn main() {\n    let x = 1\n    let y\n}"),
            (Code::Expected, 3, 10)
        );
    }

    #[test]
    fn crlf_end_of_line_column_matches_lf() {
        assert_eq!(
            parse_err("fn main() {\r\n    let y\r\n}\r\n"),
            (Code::Expected, 2, 10)
        );
    }

    #[test]
    fn class_with_members() {
        assert_eq!(
            program(
                "class Tree {\n    value: i64\n    mut left: Tree\n\n    fn sum(self) -> i64 { self.value }\n}"
            ),
            "(class Tree (value i64) (mut left Tree) (fn sum (self) i64 (block (. self value))))"
        );
    }

    #[test]
    fn empty_class() {
        assert_eq!(program("class E {}"), "(class E)");
    }

    #[test]
    fn items_in_any_mix() {
        assert_eq!(
            program("fn f() {}\nclass E {}\nfn g() {}"),
            "(fn f () unit (block))\n(class E)\n(fn g () unit (block))"
        );
    }

    #[test]
    fn method_params_follow_self() {
        assert_eq!(
            program("class A {\n    fn f(self, a: i64, b: bool) {}\n}"),
            "(class A (fn f (self (a i64) (b bool)) unit (block)))"
        );
    }

    #[test]
    fn method_without_self() {
        assert_eq!(
            parse_err("class A {\n    fn f() {}\n}"),
            (Code::MethodNeedsSelf, 2, 8)
        );
    }

    #[test]
    fn self_not_first() {
        assert_eq!(
            parse_err("class A {\n    fn f(x: i64, self) {}\n}"),
            (Code::SelfNotFirst, 2, 18)
        );
    }

    #[test]
    fn self_repeated() {
        assert_eq!(
            parse_err("class A {\n    fn f(self, self) {}\n}"),
            (Code::SelfNotFirst, 2, 16)
        );
    }

    #[test]
    fn self_outside_class() {
        assert_eq!(parse_err("fn f(self) {}"), (Code::SelfNotFirst, 1, 6));
    }

    #[test]
    fn call_of_a_call() {
        assert_eq!(expr_err("f()()"), (Code::Expected, 1, 16));
    }

    #[test]
    fn assign_to_method_call() {
        assert_eq!(expr_err("t.m() = 1"), (Code::AssignToExpression, 1, 13));
    }

    #[test]
    fn bad_class_member() {
        assert_eq!(parse_err("class A { 1 }"), (Code::Expected, 1, 11));
    }

    #[test]
    fn class_members_need_a_line_break() {
        assert_eq!(
            parse_err("class A { x: i64 y: i64 }"),
            (Code::Expected, 1, 18)
        );
    }

    #[test]
    fn optional_types() {
        assert_eq!(
            body("fn main() { let x: Tree? = None }"),
            "(block (let x Tree? None))"
        );
        assert_eq!(
            program("fn f(t: Tree?) -> i64? {}"),
            "(fn f ((t Tree?)) i64? (block))"
        );
        assert_eq!(
            program("class A {\n    mut next: A?\n}"),
            "(class A (mut next A?))"
        );
        // The span runs through the `?`.
        let f = first_fn("fn f() -> Tree? {}");
        assert_eq!(f.ret.expect("has a return type").span, Span::new(10, 15));
    }

    #[test]
    fn if_let_chain() {
        assert_eq!(
            body("fn main() { if let n = node { a } else if let m = other { b } else { c } }"),
            "(block (if-let n node (block a) (block (if-let m other (block b) (block c)))))"
        );
        assert_eq!(
            body("fn main() { if let n = node { a } }"),
            "(block (if-let n node (block a)))"
        );
        assert_eq!(
            body("fn main() { if let n = node { a } else if c { b } }"),
            "(block (if-let n node (block a) (block (if c (block b)))))"
        );
    }

    #[test]
    fn while_let() {
        assert_eq!(
            body("fn main() { while let n = node { f(n) } }"),
            "(block (while-let n node (block (call f n))))"
        );
    }

    #[test]
    fn let_pattern_needs_a_name() {
        let rows = [
            ("if let 1 = x {}", 20),
            ("while let 1 = x {}", 23),
            ("if let n x {}", 22),
            ("if let = n {}", 20),
        ];
        for (src, col) in rows {
            assert_eq!(expr_err(src), (Code::Expected, 1, col), "{src}");
        }
    }

    #[test]
    fn optionals_do_not_nest() {
        // `Tree??` lexes as one token and `Tree? ?` as two; both name the type `Tree??`.
        let rows = [
            (
                "fn main() { let x: Tree?? = None }",
                Span::new(19, 25),
                (1, 20),
            ),
            ("fn f() -> Tree? ? {}", Span::new(10, 17), (1, 11)),
        ];
        for (src, span, position) in rows {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::OptionalNests, "{src}");
            assert_eq!(d.span(), span, "{src}");
            assert_eq!(line_col(src, d.span().start), position, "{src}");
        }
    }

    #[test]
    fn array_literals() {
        let rows = [
            ("[]", "(array)"),
            ("[1]", "(array 1)"),
            ("[1, 2,]", "(array 1 2)"),
            ("[\n        1,\n        2,\n    ]", "(array 1 2)"),
            ("[0; n]", "(array-fill 0 n)"),
            ("[[1], []]", "(array (array 1) (array))"),
        ];
        for (src, want) in rows {
            assert_eq!(expr(src), want, "{src}");
        }
    }

    #[test]
    fn indexing_chains_and_binds_like_a_postfix() {
        let rows = [
            ("a[i][j]", "(index (index a i) j)"),
            ("a.b[0].c", "(. (index (. a b) 0) c)"),
            ("[1, 2][0]", "(index (array 1 2) 0)"),
            ("-a[0]", "(- (index a 0))"),
            ("a[i + 1]", "(index a (+ i 1))"),
        ];
        for (src, want) in rows {
            assert_eq!(expr(src), want, "{src}");
        }
    }

    #[test]
    fn for_loops() {
        let rows = [
            ("for x in xs {}", "(for x xs (block))"),
            ("for i in 2..n + 1 {}", "(for i (.. 2 (+ n 1)) (block))"),
            (
                "for i in 0..xs.len() {}",
                "(for i (.. 0 (call (. xs len))) (block))",
            ),
            (
                "for x in [1, 2] { f(x) }",
                "(for x (array 1 2) (block (call f x)))",
            ),
        ];
        for (src, want) in rows {
            assert_eq!(expr(src), want, "{src}");
        }
    }

    #[test]
    fn array_types_round_trip() {
        assert_eq!(
            program("fn f(a: [i64], b: [Node]?, c: [Node?], d: [[bool]]) {}"),
            "(fn f ((a [i64]) (b [Node]?) (c [Node?]) (d [[bool]])) unit (block))"
        );
    }

    #[test]
    fn array_type_spans() {
        let f = first_fn("fn f(a: [[i64]?]?) {}");
        assert_eq!(f.params[0].ty.span, Span::new(8, 17));
    }

    #[test]
    fn syntax_errors_for_arrays_and_for() {
        let rows = [
            ("let x = 1; 2", Code::Semicolon, 22),
            ("let r = 0..3", Code::RangeOutsideFor, 22),
            ("f(0..3)", Code::RangeOutsideFor, 16),
            ("for i in 0..1..2 {}", Code::Expected, 26),
            ("[1; 2, 3]", Code::Expected, 18),
            ("(1; 2)", Code::Expected, 15),
            ("a[]", Code::Expected, 15),
        ];
        for (src, code, col) in rows {
            assert_eq!(expr_err(src), (code, 1, col), "{src}");
        }
    }

    #[test]
    fn stray_semicolon_in_a_class_body() {
        assert_eq!(
            parse_err("class A {\n    x: i64; y: i64\n}"),
            (Code::Semicolon, 2, 11)
        );
    }

    #[test]
    fn semicolon_fix() {
        for (src, position, at) in [
            ("fn main() {\n    let x = 1;\n}", (2, 14), 25),
            ("fn main() { let x = 1; }", (1, 22), 21),
            (
                "fn main() {\n    print(1); // done\n    print(2)\n}",
                (2, 13),
                24,
            ),
            ("class A {\n    x: i64;\n}", (2, 11), 20),
            // `return;` has no value.
            ("fn main() { return; }", (1, 19), 18),
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::Semicolon, "{src}");
            assert_eq!(line_col(src, d.span().start), position, "{src}");
            assert_eq!(
                fix_edits(&d),
                Some((true, vec![(at, at + 1, String::new())])),
                "{src}"
            );
            assert!(d.help().is_none(), "{src}");
        }
    }

    #[test]
    fn certain_semicolon_fix_removes_the_error() {
        let src = "fn main() {\n    let x = 1;\n    let y = x + 1; // two\n    print(y);\n}\n";
        // One `;` at a time: the parser reports one error per item, and these `;`s share `main`.
        let once = fixed(src);
        assert_eq!(
            once,
            "fn main() {\n    let x = 1\n    let y = x + 1; // two\n    print(y);\n}\n"
        );
        let twice = fixed(&once);
        let thrice = fixed(&twice);
        assert_eq!(
            thrice,
            "fn main() {\n    let x = 1\n    let y = x + 1 // two\n    print(y)\n}\n"
        );
        parse_ok(&thrice);
    }

    #[test]
    fn semicolon_mid_line_is_a_help() {
        // A complete statement, then more code on the line: advice, no edit.
        for src in [
            "fn main() { let x = 1; let y = 2 }",
            "class A {\n    x: i64; y: i64\n}",
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::Semicolon, "{src}");
            assert!(d.help().is_some(), "{src}");
            assert_eq!(fix_edits(&d), None, "{src}");
        }
    }

    #[test]
    fn doubled_semicolon_has_neither() {
        // Moving the next statement does not help: another `;` follows.
        let src = "fn main() { let x = 1;; }";
        let d = parse_diagnostic(src);
        assert_eq!(d.code(), Code::Semicolon);
        assert_eq!(line_col(src, d.span().start), (1, 22));
        assert!(d.help().is_none());
        assert_eq!(fix_edits(&d), None);
    }

    #[test]
    fn semicolon_elsewhere_is_an_expected_error() {
        // Not after a complete statement: the parser wanted something else there.
        for (src, col) in [
            ("fn f();", 7),
            ("fn main() { let x = ; }", 21),
            ("fn main() { [1; 2; 3] }", 18),
            ("fn main() { f(1; 2) }", 16),
            ("fn main() { ; }", 13),
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::Expected, "{src}");
            assert_eq!(line_col(src, d.span().start), (1, col), "{src}");
            assert!(d.help().is_none(), "{src}");
            assert_eq!(fix_edits(&d), None, "{src}");
        }
    }

    #[test]
    fn optional_array_types_do_not_nest() {
        assert_eq!(
            parse_err("fn f(x: [Node]??) {}"),
            (Code::OptionalNests, 1, 9)
        );
    }

    #[test]
    fn var_is_reserved_everywhere() {
        let var = Code::Var;
        assert_eq!(parse_err("fn main() {\n    var x = 1\n}"), (var, 2, 5));
        assert_eq!(parse_err("fn var() {}"), (var, 1, 4));
        assert_eq!(parse_err("fn main() { let var = 1 }"), (var, 1, 17));
        assert_eq!(parse_err("class P {\n    var hp: i64\n}"), (var, 2, 5));
    }

    #[test]
    fn var_has_a_help_and_no_fix() {
        // The binding and the field each name their own idiom.
        let binding = parse_diagnostic("fn main() {\n    var x = 1\n}");
        let field = parse_diagnostic("class P {\n    var hp: i64\n}");
        for d in [&binding, &field] {
            assert!(d.help().is_some());
            assert_eq!(fix_edits(d), None);
        }
    }

    #[test]
    fn reserved_errors() {
        // One row per place that wants a name or an expression.
        for (src, line, col) in [
            ("fn main() {\n    let match = 1\n}", 2, 9),
            ("class C {\n    enum: i64\n}", 2, 5),
            ("fn main() {\n    print(echo)\n}", 2, 11),
            ("fn main() {\n    let todo = 1\n}", 2, 9),
            ("fn todo() {}", 1, 4),
            ("class todo {}", 1, 7),
            ("fn main() {\n    print(p.type)\n}", 2, 13),
            ("import x", 1, 1),
        ] {
            assert_eq!(parse_err(src), (Code::ReservedWord, line, col), "{src}");
        }
    }

    #[test]
    fn todo_forms() {
        for (expr, expected) in [
            ("todo", "(todo)"),
            ("todo(\"why\")", "(todo \"why\")"),
            ("todo(\"a\", \"b\")", "(todo \"a\" \"b\")"),
        ] {
            let src = format!("fn main() {{\n    {expr}\n}}");
            assert_eq!(body(&src), format!("(block {expected})"), "{src}");
        }
        // A `todo` that ends the file has fewer than three tokens after it.
        parse_ok("fn main() {}\nconst X = todo");
        // The arguments are a call's, with its errors.
        assert_eq!(
            parse_err("fn main() {\n    todo(1 2)\n}"),
            (Code::Expected, 2, 12)
        );
    }

    /// Each other way of writing `todo`: the error's position, and the line its fix makes.
    #[test]
    fn todo_misspelled() {
        for (line, col, after) in [
            ("todo()", 9, "todo"),
            ("todo!", 9, "todo"),
            ("todo!(\"why\")", 9, "todo(\"why\")"),
            ("todo!()", 9, "todo"),
        ] {
            let src = format!("fn main() {{\n    {line}\n}}");
            assert_eq!(parse_err(&src), (Code::TodoForm, 2, col), "{src}");
            assert_eq!(fixed(&src), format!("fn main() {{\n    {after}\n}}"));
        }
    }

    #[test]
    fn mut_before_a_reserved_word_is_still_reported() {
        assert_eq!(parse_err("fn f(mut type: i64) {}"), (Code::Expected, 1, 6));
    }

    #[test]
    fn mut_is_a_keyword() {
        assert_eq!(
            parse_err("fn main() { let mut = 1 }"),
            (Code::Expected, 1, 21)
        );
    }

    #[test]
    fn one_line_class() {
        assert_eq!(
            program("class UserId { value: i64 }"),
            "(class UserId (value i64))"
        );
    }

    #[test]
    fn field_let_fix() {
        // The rest of the line is a field: `let ` goes, up to the next token.
        for (src, position, edit) in [
            ("class P {\n    let hp: [Tree?]\n}", (2, 5), (14, 18)),
            ("class P {\n    let mut hp: i64\n}", (2, 5), (14, 18)),
            ("class P { let hp: i64 }", (1, 11), (10, 14)),
            ("class P {\n    let   hp: i64 // note\n}", (2, 5), (14, 20)),
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::FieldLet, "{src}");
            assert_eq!(line_col(src, d.span().start), position, "{src}");
            assert_eq!(
                fix_edits(&d),
                Some((true, vec![(edit.0, edit.1, String::new())])),
                "{src}"
            );
            assert!(d.help().is_none(), "{src}");
        }
    }

    #[test]
    fn certain_field_let_fix_removes_the_error() {
        let src = "class P {\n    let mut hp: i64\n    name: [i64]\n}\n";
        let result = fixed(src);
        assert_eq!(result, "class P {\n    mut hp: i64\n    name: [i64]\n}\n");
        parse_ok(&result);
    }

    #[test]
    fn let_field_with_anything_else_is_a_help() {
        for (src, position) in [
            // Not a field, with and without `mut`.
            ("class P {\n    let hp = 1\n}", (2, 5)),
            ("class C { let mut n = 0 }", (1, 11)),
            ("class P {\n    let\n}", (2, 5)),
            // A field, but the line goes on.
            ("class P {\n    let hp: i64, y: i64\n}", (2, 5)),
            ("class P {\n    let hp: i64 y\n}", (2, 5)),
            ("class P {\n    let mut hp: i64 y\n}", (2, 5)),
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::FieldLet, "{src}");
            assert_eq!(line_col(src, d.span().start), position, "{src}");
            assert!(d.help().is_some(), "{src}");
            assert_eq!(fix_edits(&d), None, "{src}");
        }
    }

    #[test]
    fn comma_after_a_field() {
        let src = "class P {\n    x: i64, y: i64\n}";
        let d = parse_diagnostic(src);
        assert_eq!(d.code(), Code::FieldComma);
        assert_eq!(line_col(src, d.span().start), (2, 11));
        assert!(d.help().is_some());
        assert_eq!(fix_edits(&d), None);
    }

    #[test]
    fn comma_after_a_method_is_not_a_field_comma() {
        assert_eq!(
            parse_err("class C {\n fn f(self) {},\n}"),
            (Code::Expected, 2, 15)
        );
    }

    #[test]
    fn mut_self_fix() {
        // Certain only when `self` ends the parameter: `mut self: i64` stays broken without `mut`.
        for (src, edit, certain) in [
            ("class C {\n    fn f(mut self) {}\n}", (19, 23), true),
            (
                "class C {\n    fn f(mut   self, n: i64) {}\n}",
                (19, 25),
                true,
            ),
            ("class C {\n    fn f(mut self: i64) {}\n}", (19, 23), false),
        ] {
            let d = parse_diagnostic(src);
            assert_eq!(d.code(), Code::MutSelf, "{src}");
            assert_eq!(line_col(src, d.span().start), (2, 10), "{src}");
            assert_eq!(
                fix_edits(&d),
                Some((certain, vec![(edit.0, edit.1, String::new())])),
                "{src}"
            );
            assert!(d.help().is_some(), "{src}");
        }
    }

    #[test]
    fn certain_mut_self_fix_removes_the_error() {
        let src = "class C {\n    mut n: i64\n    fn add(mut self, by: i64) { self.n += by }\n}\n";
        let result = fixed(src);
        assert_eq!(
            result,
            "class C {\n    mut n: i64\n    fn add(self, by: i64) { self.n += by }\n}\n"
        );
        parse_ok(&result);
    }

    #[test]
    fn mut_self_outside_the_receiver_position() {
        assert_eq!(parse_err("fn f(mut self) {}"), (Code::SelfNotFirst, 1, 10));
        assert_eq!(
            parse_err("class C {\n    fn f(self, n: i64, mut self) {}\n}"),
            (Code::SelfNotFirst, 2, 28)
        );
    }

    #[test]
    fn mut_parameter() {
        let src = "fn f(a: i64, mut n: i64) {}";
        let d = parse_diagnostic(src);
        assert_eq!(d.code(), Code::MutParameter);
        assert_eq!(line_col(src, d.span().start), (1, 14));
        // Removing the `mut` would make the body assign to a parameter.
        assert!(d.help().is_some());
        assert_eq!(fix_edits(&d), None);
    }

    #[test]
    fn mut_before_neither_self_nor_a_name() {
        assert_eq!(parse_err("fn f(mut 1) {}"), (Code::Expected, 1, 6));
    }

    #[test]
    fn broken_items_keep_their_names() {
        let src = "fn f() {\n    let = 1\n}\nclass C {\n    x i64\n}\nclass D {\n    fn m(self) {\n        let = 1\n    }\n}\n";
        let parsed = parsed(src);
        assert!(parsed.checkable);
        assert_eq!(parsed.diagnostics.len(), 3, "{:?}", parsed.diagnostics);
        let ident = |name: &str, start| Ident {
            name: Symbol::intern(name),
            span: Span::new(start, start + 1),
        };
        assert_eq!(
            parsed.program.broken,
            [
                Broken {
                    name: ident("f", 3),
                    kind: BrokenKind::Function
                },
                Broken {
                    name: ident("C", 29),
                    kind: BrokenKind::Class
                }
            ]
        );
        match parsed.program.items.as_slice() {
            [Item::Class(d)] => {
                assert_eq!(d.members, []);
                assert_eq!(d.broken_methods, [ident("m", 62)]);
            }
            items => panic!("expected class D alone, found {items:?}"),
        }
    }

    #[test]
    fn earliest_error_in_an_item_wins() {
        // The parse error comes first here.
        assert_eq!(
            parse_err("fn main() {\n    let = 1 $\n}\n"),
            (Code::Expected, 2, 9)
        );
        // On a tie, the lexer's error wins: it may have caused the parse error.
        assert_eq!(
            parse_err("fn main() {\n    let 99999999999999999999 = 1\n}\n"),
            (Code::IntegerTooLarge, 2, 9)
        );
    }

    #[test]
    fn lexer_error_outside_every_item_stays() {
        let parsed = parsed("fn f() {} $\nfn main() {}\n");
        let codes: Vec<_> = parsed.diagnostics.iter().map(Diagnostic::code).collect();
        assert_eq!(codes, [Code::UnexpectedCharacter]);
        assert!(parsed.checkable);
        assert_eq!(parsed.program.broken, []);
        assert_eq!(parsed.program.items.len(), 2);
    }

    #[test]
    fn item_cut_off_by_end_of_file() {
        assert_eq!(
            parse_err("fn main() {}\nfn f() ->"),
            (Code::Expected, 2, 10)
        );
    }
}
