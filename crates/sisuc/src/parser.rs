//! The parser: tokens to the syntax tree.

use crate::ast::{
    Arg, BinaryOp, Block, Class, CompareOp, Expr, ExprKind, FieldDecl, ForSource, Function, Ident,
    Item, Member, Param, Program, Stmt, StmtKind, TypeExpr, TypeKind, UnaryOp,
};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Token, TokenKind};

/// Parses `tokens`, which must end with `Eof`. Stops at the first error.
pub(crate) fn parse(tokens: &[Token]) -> Result<Program, Diagnostic> {
    Parser { tokens, pos: 0 }.program()
}

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
}

impl Parser<'_> {
    fn peek(&self) -> &Token {
        // `Eof` is last and never consumed, so `pos` stays in range.
        &self.tokens[self.pos]
    }

    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if token.kind != TokenKind::Eof {
            self.pos += 1;
        }
        token
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek().kind == *kind
    }

    /// Consumes the next token if it is `kind`.
    fn eat(&mut self, kind: &TokenKind) -> Option<Token> {
        self.at(kind).then(|| self.bump())
    }

    fn skip_newlines(&mut self) {
        while self.eat(&TokenKind::Newline).is_some() {}
    }

    /// What must follow a statement or a class member: a line break or the closing `}`.
    fn end_of_line(&self) -> Diagnostic {
        self.expected("end of line")
    }

    /// "expected <what>, found <the next token>", pointing at the next token.
    /// A stray `;` or `var` always gets its one message, wherever it appears.
    fn expected(&self, what: &str) -> Diagnostic {
        let found = self.peek();
        match found.kind {
            TokenKind::Semicolon => {
                return Diagnostic::error(
                    found.span,
                    "Sisu has no `;`: statements end at the end of the line",
                );
            }
            TokenKind::Var => {
                return Diagnostic::error(found.span, "Sisu has no `var`").help("use `let mut`");
            }
            _ => {}
        }
        Diagnostic::error(
            found.span,
            format!("expected {what}, found {}", found.kind.describe()),
        )
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<Token, Diagnostic> {
        self.eat(kind)
            .ok_or_else(|| self.expected(&kind.describe()))
    }

    fn ident(&mut self, what: &str) -> Result<Ident, Diagnostic> {
        match self.peek().kind.clone() {
            TokenKind::Ident(name) => Ok(Ident {
                name,
                span: self.bump().span,
            }),
            _ => Err(self.expected(what)),
        }
    }

    /// The only place that reads a type; later milestones extend it.
    fn parse_type(&mut self) -> Result<TypeExpr, Diagnostic> {
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
            return Err(Diagnostic::error(
                start.to(self.peek().span),
                format!("`{base}??` is not a type"),
            )
            .help("optionals do not nest"));
        }
        Ok(TypeExpr {
            kind,
            optional,
            span,
        })
    }

    fn program(&mut self) -> Result<Program, Diagnostic> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.at(&TokenKind::Eof) {
            items.push(match self.peek().kind {
                TokenKind::Fn => Item::Function(self.function(false)?),
                TokenKind::Class => Item::Class(self.class()?),
                _ => return Err(self.expected("`fn` or `class`")),
            });
            self.skip_newlines();
        }
        Ok(Program { items })
    }

    fn class(&mut self) -> Result<Class, Diagnostic> {
        self.expect(&TokenKind::Class)?;
        let name = self.ident("a class name")?;
        self.expect(&TokenKind::LBrace)?;
        let mut members = Vec::new();
        self.skip_newlines();
        while !self.at(&TokenKind::RBrace) {
            let member = self.member()?;
            if matches!(member, Member::Field(_)) && self.at(&TokenKind::Comma) {
                return Err(Diagnostic::error(
                    self.peek().span,
                    "a field ends at the end of the line",
                )
                .help("put each field on its own line"));
            }
            members.push(member);
            if !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Newline) {
                return Err(self.end_of_line());
            }
            self.skip_newlines();
        }
        self.bump();
        Ok(Class { name, members })
    }

    fn member(&mut self) -> Result<Member, Diagnostic> {
        match self.peek().kind {
            TokenKind::Ident(_) | TokenKind::Mut => {
                let mutable = self.eat(&TokenKind::Mut).is_some();
                let name = self.ident("a field name")?;
                self.expect(&TokenKind::Colon)?;
                Ok(Member::Field(FieldDecl {
                    mutable,
                    name,
                    ty: self.parse_type()?,
                }))
            }
            TokenKind::Let => Err(
                Diagnostic::error(self.peek().span, "a field takes no `let`")
                    .help(format!("write it bare: `{}`", self.field_after_keyword())),
            ),
            TokenKind::Var => Err(
                Diagnostic::error(self.peek().span, "Sisu has no `var`").help(format!(
                    "mark the field `mut`: `mut {}`",
                    self.field_after_keyword()
                )),
            ),
            TokenKind::Fn => {
                let method = self.function(true)?;
                if method.self_param.is_none() {
                    return Err(Diagnostic::error(
                        method.name.span,
                        "a method needs `self` as its first parameter",
                    )
                    .help("static methods are not supported yet"));
                }
                Ok(Member::Method(method))
            }
            _ => Err(self.expected("a field or `fn`")),
        }
    }

    /// The field after the `let` or `var` at the next token, as `name: Type`, for a help
    /// line; the placeholder `name: Type` when no field follows.
    fn field_after_keyword(&self) -> String {
        let mut rest = Parser {
            tokens: self.tokens,
            pos: self.pos + 1,
        };
        let mut field = || -> Result<String, Diagnostic> {
            let name = rest.ident("a field name")?;
            rest.expect(&TokenKind::Colon)?;
            Ok(format!("{}: {}", name.name, rest.parse_type()?))
        };
        field().unwrap_or_else(|_| "name: Type".to_string())
    }

    /// `method` is true inside a class body, where `self` may be the first parameter.
    fn function(&mut self, method: bool) -> Result<Function, Diagnostic> {
        let fn_token = self.expect(&TokenKind::Fn)?;
        let name = self.ident("a function name")?;
        self.expect(&TokenKind::LParen)?;
        let mut self_param = None;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                if self.at(&TokenKind::Mut) {
                    let receiver = method && self_param.is_none() && params.is_empty();
                    // `mut` is never `Eof`, so the token after it exists.
                    match &self.tokens[self.pos + 1].kind {
                        TokenKind::SelfKw if receiver => {
                            return Err(Diagnostic::error(
                                self.peek().span,
                                "a class method takes `self`, not `mut self`",
                            )
                            .help(
                                "an object is shared: `self` can already write every `mut` field",
                            ));
                        }
                        TokenKind::SelfKw => {
                            self.bump();
                        }
                        TokenKind::Ident(name) => {
                            return Err(Diagnostic::error(
                                self.peek().span,
                                "a parameter cannot be `mut`",
                            )
                            .help(format!(
                                "copy it into a variable: `let mut {name} = {name}`"
                            )));
                        }
                        _ => {}
                    }
                }
                if self.at(&TokenKind::SelfKw) {
                    if !(method && self_param.is_none() && params.is_empty()) {
                        return Err(Diagnostic::error(
                            self.peek().span,
                            "`self` is only the first parameter of a method",
                        ));
                    }
                    self_param = Some(self.bump().span);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                    continue;
                }
                let name = self.ident("a parameter name")?;
                self.expect(&TokenKind::Colon)?;
                params.push(Param {
                    name,
                    ty: self.parse_type()?,
                });
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
        let open = self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.at(&TokenKind::RBrace) {
            stmts.push(self.stmt()?);
            if !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Newline) {
                return Err(self.end_of_line());
            }
            self.skip_newlines();
        }
        let close = self.bump();
        Ok(Block {
            stmts,
            span: open.span.to(close.span),
        })
    }

    fn stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.peek().span;
        let kind = match self.peek().kind {
            TokenKind::Let => self.let_stmt()?,
            TokenKind::While => {
                self.bump();
                if let Some((name, value)) = self.let_binding()? {
                    StmtKind::WhileLet {
                        name,
                        value,
                        body: self.block()?,
                    }
                } else {
                    let cond = self.expr()?;
                    StmtKind::While {
                        cond,
                        body: self.block()?,
                    }
                }
            }
            TokenKind::For => {
                self.bump();
                let name = self.ident("a variable name")?;
                self.expect(&TokenKind::In)?;
                // A range is only valid here, so the header reads its two sides itself.
                let first = self.binary(1)?;
                let source = if self.eat(&TokenKind::DotDot).is_some() {
                    ForSource::Range {
                        start: first,
                        end: self.binary(1)?,
                    }
                } else {
                    ForSource::Array(first)
                };
                StmtKind::For {
                    name,
                    source,
                    body: self.block()?,
                }
            }
            TokenKind::Return => {
                self.bump();
                if self.at(&TokenKind::Newline) || self.at(&TokenKind::RBrace) {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.expr()?))
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
            TokenKind::Else => {
                return Err(Diagnostic::error(
                    start,
                    "`else` must be on the same line as the closing `}`",
                ));
            }
            _ => self.expr_or_assign()?,
        };
        Ok(Stmt {
            kind,
            span: start.to(self.last_span()),
        })
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
        let name = self.ident("a variable name")?;
        self.expect(&TokenKind::Eq)?;
        Ok(Some((name, self.expr()?)))
    }

    fn let_stmt(&mut self) -> Result<StmtKind, Diagnostic> {
        self.bump();
        let mutable = self.eat(&TokenKind::Mut).is_some();
        let name = self.ident("a variable name")?;
        let ty = match self.eat(&TokenKind::Colon) {
            Some(_) => Some(self.parse_type()?),
            None => None,
        };
        self.expect(&TokenKind::Eq)?;
        Ok(StmtKind::Let {
            mutable,
            name,
            ty,
            init: self.expr()?,
        })
    }

    /// An expression statement, or an assignment when an assignment operator follows.
    fn expr_or_assign(&mut self) -> Result<StmtKind, Diagnostic> {
        let target = self.expr()?;
        let op = match self.peek().kind {
            TokenKind::Eq => None,
            TokenKind::PlusEq => Some(BinaryOp::Add),
            TokenKind::MinusEq => Some(BinaryOp::Sub),
            TokenKind::StarEq => Some(BinaryOp::Mul),
            TokenKind::SlashEq => Some(BinaryOp::Div),
            TokenKind::PercentEq => Some(BinaryOp::Rem),
            _ => return Ok(StmtKind::Expr(target)),
        };
        if !is_place(&target) {
            return Err(Diagnostic::error(
                target.span,
                "cannot assign to this expression",
            ));
        }
        if goes_through_safe_access(&target) {
            return Err(Diagnostic::error(target.span, "cannot assign through `?.`")
                .help("unwrap it with `if let`"));
        }
        self.bump();
        let value = self.expr()?;
        Ok(match op {
            None => StmtKind::Assign { target, value },
            Some(op) => StmtKind::CompoundAssign { op, target, value },
        })
    }

    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        let e = self.binary(1)?;
        if self.at(&TokenKind::DotDot) {
            return Err(Diagnostic::error(
                self.peek().span,
                "ranges are only allowed in `for`",
            ));
        }
        Ok(e)
    }

    /// Precedence climbing over levels 1 to 7; `min` is the lowest level to accept.
    fn binary(&mut self, min: u8) -> Result<Expr, Diagnostic> {
        let mut lhs = self.unary()?;
        loop {
            let kind = &self.peek().kind;
            if compare_op(kind).is_some() {
                if min > 4 {
                    return Ok(lhs);
                }
                lhs = self.comparison_chain(lhs)?;
            } else if let Some((level, op)) = infix_op(kind) {
                if level < min {
                    return Ok(lhs);
                }
                self.bump();
                // `??` is right-associative: its right side starts at its own level.
                let rhs = self.binary(if matches!(op, Infix::Coalesce) {
                    level
                } else {
                    level + 1
                })?;
                let span = lhs.span.to(rhs.span);
                let (lhs_box, rhs_box) = (Box::new(lhs), Box::new(rhs));
                lhs = Expr {
                    span,
                    kind: match op {
                        Infix::Binary(op) => ExprKind::Binary {
                            op,
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
                if level == 3 {
                    let message = match self.peek().kind {
                        TokenKind::Is => "`is` does not chain",
                        TokenKind::EqEq | TokenKind::NotEq => "`==` and `!=` do not chain",
                        _ => continue,
                    };
                    return Err(Diagnostic::error(self.peek().span, message)
                        .help("join the comparisons with `&&`"));
                }
            } else {
                return Ok(lhs);
            }
        }
    }

    /// Level 4: `first` and every `<`/`<=`/`>`/`>=` operand after it, as one node.
    fn comparison_chain(&mut self, first: Expr) -> Result<Expr, Diagnostic> {
        let start = first.span;
        let mut operands = vec![first];
        let mut ops: Vec<CompareOp> = Vec::new();
        while let Some(op) = compare_op(&self.peek().kind) {
            if ops
                .first()
                .is_some_and(|first| first.is_less() != op.is_less())
            {
                return Err(Diagnostic::error(
                    self.peek().span,
                    "a comparison chain must go in one direction",
                ));
            }
            self.bump();
            ops.push(op);
            operands.push(self.binary(5)?);
        }
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
        let start = self.bump().span;
        let operand = self.unary()?;
        Ok(Expr {
            span: start.to(operand.span),
            kind: ExprKind::Unary {
                op,
                operand: Box::new(operand),
            },
        })
    }

    /// `primary` followed by any number of `.field`, `.method(args)`, `?.field`,
    /// `?.method(args)` and `[index]`.
    fn postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.primary()?;
        loop {
            if self.eat(&TokenKind::LBracket).is_some() {
                let index = self.expr()?;
                let close = self.expect(&TokenKind::RBracket)?;
                expr = Expr {
                    span: expr.span.to(close.span),
                    kind: ExprKind::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                    },
                };
                continue;
            }
            let Some(access) = self
                .eat(&TokenKind::Dot)
                .or_else(|| self.eat(&TokenKind::QuestionDot))
            else {
                return Ok(expr);
            };
            let safe = access.kind == TokenKind::QuestionDot;
            let name = self.ident("a field or method name")?;
            expr = if self.at(&TokenKind::LParen) {
                let (args, close) = self.args()?;
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
        let mut elements = Vec::new();
        let mut fill = None;
        while !self.at(&TokenKind::RBracket) {
            elements.push(self.expr()?);
            if elements.len() == 1 && self.eat(&TokenKind::Semicolon).is_some() {
                fill = Some(self.expr()?);
                break;
            }
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        let close = self.expect(&TokenKind::RBracket)?;
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
            TokenKind::Int(n) => {
                self.bump();
                ExprKind::Int(n)
            }
            TokenKind::True | TokenKind::False => {
                self.bump();
                ExprKind::Bool(token.kind == TokenKind::True)
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
            _ => return Err(self.expected("an expression")),
        };
        Ok(Expr {
            kind,
            span: token.span,
        })
    }

    /// `(` arguments `)`, and the span of the `)`.
    fn args(&mut self) -> Result<(Vec<Arg>, Span), Diagnostic> {
        self.expect(&TokenKind::LParen)?;
        let mut args = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                args.push(self.arg()?);
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect(&TokenKind::RParen)?;
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
        let head = match self.let_binding()? {
            Some((name, value)) => Head::Let(name, value),
            None => Head::Cond(self.expr()?),
        };
        let then_block = self.block()?;
        let mut end = then_block.span;
        let else_block = if self.eat(&TokenKind::Else).is_some() {
            let block = if self.at(&TokenKind::If) {
                // `else if` and `else if let` become a block holding the inner `if`.
                let inner = self.if_expr()?;
                Block {
                    span: inner.span,
                    stmts: vec![Stmt {
                        span: inner.span,
                        kind: StmtKind::Expr(inner),
                    }],
                }
            } else {
                self.block()?
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
    use crate::diagnostic::line_col;
    use crate::lexer::lex;

    fn parsed(src: &str) -> Program {
        parse(&lex(src).expect("source lexes")).expect("source parses")
    }

    fn program(src: &str) -> String {
        parsed(src).to_string()
    }

    /// The first item, which must be a function.
    fn first_fn(src: &str) -> Function {
        match parsed(src).items.into_iter().next() {
            Some(Item::Function(f)) => f,
            other => panic!("expected a function, found {other:?}"),
        }
    }

    fn body(src: &str) -> String {
        first_fn(src).body.to_string()
    }

    fn parse_diagnostic(src: &str) -> Diagnostic {
        parse(&lex(src).expect("source lexes")).expect_err("source does not parse")
    }

    fn parse_err(src: &str) -> (String, usize, usize) {
        let d = parse_diagnostic(src);
        let (line, col) = line_col(src, d.span.start);
        (d.message, line, col)
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
    fn else_on_new_line() {
        assert_eq!(
            parse_err("fn main() {\n    if a {\n    }\n    else {\n    }\n}"),
            (
                "`else` must be on the same line as the closing `}`".to_string(),
                4,
                5
            )
        );
    }

    #[test]
    fn two_statements_on_one_line() {
        assert_eq!(
            parse_err("fn main() { let x = 1 2 }"),
            ("expected end of line, found `2`".to_string(), 1, 23)
        );
    }

    #[test]
    fn unexpected_eof() {
        let src = "fn\n";
        assert_eq!(
            parse_err(src),
            (
                "expected a function name, found end of file".to_string(),
                1,
                3
            )
        );
        let rendered = parse_diagnostic(src).render("a.sisu", src);
        assert!(rendered.contains("--> a.sisu:1:3"), "{rendered}");
    }

    #[test]
    fn top_level_statement() {
        assert_eq!(
            parse_err("let x = 1"),
            ("expected `fn` or `class`, found `let`".to_string(), 1, 1)
        );
    }

    /// Parses `fn main() { <src> }` and prints the first statement.
    fn expr(src: &str) -> String {
        first_fn(&format!("fn main() {{ {src} }}")).body.stmts[0].to_string()
    }

    /// Error message, line and column for `fn main() { <src> }`; the columns start at 13.
    fn expr_err(src: &str) -> (String, usize, usize) {
        parse_err(&format!("fn main() {{ {src} }}"))
    }

    #[test]
    fn new_tokens_are_quoted_in_errors() {
        // `self` is an expression now, so its row hits a different error.
        let rows = [
            ("class", "class", "an expression"),
            ("is", "is", "an expression"),
            ("let self = 1", "self", "a variable name"),
            ("let None = 1", "None", "a variable name"),
            (".", ".", "an expression"),
            ("?", "?", "an expression"),
            ("?.", "?.", "an expression"),
            ("??", "??", "an expression"),
        ];
        for (src, tok, what) in rows {
            assert_eq!(expr_err(src).0, format!("expected {what}, found `{tok}`"));
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
        assert_eq!(
            expr_err("a < b > c"),
            (
                "a comparison chain must go in one direction".to_string(),
                1,
                19
            )
        );
    }

    #[test]
    fn mixed_chain_directions_report_at_the_second_operator() {
        assert_eq!(
            expr_err("a > b <= c"),
            (
                "a comparison chain must go in one direction".to_string(),
                1,
                19
            )
        );
    }

    #[test]
    fn mixed_equality_operators_do_not_chain() {
        assert_eq!(
            expr_err("a == b != c"),
            ("`==` and `!=` do not chain".to_string(), 1, 20)
        );
    }

    #[test]
    fn equality_does_not_chain() {
        let src = "fn main() { a == b == c }";
        let d = parse_diagnostic(src);
        assert_eq!(d.message, "`==` and `!=` do not chain");
        assert_eq!(d.help.as_deref(), Some("join the comparisons with `&&`"));
        assert_eq!(line_col(src, d.span.start), (1, 20));
    }

    #[test]
    fn is_does_not_chain() {
        let src = "fn main() { print(a is b is c) }";
        let d = parse_diagnostic(src);
        assert_eq!(d.message, "`is` does not chain");
        assert_eq!(d.help.as_deref(), Some("join the comparisons with `&&`"));
        assert_eq!(line_col(src, d.span.start), (1, 26));
    }

    #[test]
    fn mixed_is_and_equality_do_not_chain() {
        assert_eq!(
            expr_err("a is b == c"),
            ("`==` and `!=` do not chain".to_string(), 1, 20)
        );
        assert_eq!(
            expr_err("a == b is c"),
            ("`is` does not chain".to_string(), 1, 20)
        );
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
            let d = parse_diagnostic(&full);
            assert_eq!(d.message, "cannot assign through `?.`", "{src}");
            assert_eq!(d.help.as_deref(), Some("unwrap it with `if let`"), "{src}");
            assert_eq!(line_col(&full, d.span.start), (1, 13), "{src}");
        }
    }

    #[test]
    fn cannot_assign_to_expression() {
        for src in ["1 + 2 = 3", "f()[0] = 1"] {
            assert_eq!(
                expr_err(src),
                ("cannot assign to this expression".to_string(), 1, 13),
                "{src}"
            );
        }
    }

    #[test]
    fn missing_initializer_at_end_of_line() {
        assert_eq!(
            parse_err("fn main() {\n    let x = 1\n    let y\n}"),
            ("expected `=`, found end of line".to_string(), 3, 10)
        );
    }

    #[test]
    fn end_of_line_renders_one_caret() {
        let src = "fn main() {\n    let y\n}";
        let expected = "\
error: expected `=`, found end of line
 --> a.sisu:2:10
  |
2 |     let y
  |          ^";
        assert_eq!(parse_diagnostic(src).render("a.sisu", src), expected);
    }

    #[test]
    fn crlf_end_of_line_column_matches_lf() {
        assert_eq!(
            parse_err("fn main() {\r\n    let y\r\n}\r\n"),
            ("expected `=`, found end of line".to_string(), 2, 10)
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
        let src = "class A {\n    fn f() {}\n}";
        let d = parse_diagnostic(src);
        assert_eq!(d.message, "a method needs `self` as its first parameter");
        assert_eq!(
            d.help.as_deref(),
            Some("static methods are not supported yet")
        );
        assert_eq!(line_col(src, d.span.start), (2, 8));
    }

    #[test]
    fn self_not_first() {
        assert_eq!(
            parse_err("class A {\n    fn f(x: i64, self) {}\n}"),
            (
                "`self` is only the first parameter of a method".to_string(),
                2,
                18
            )
        );
    }

    #[test]
    fn self_repeated() {
        assert_eq!(
            parse_err("class A {\n    fn f(self, self) {}\n}"),
            (
                "`self` is only the first parameter of a method".to_string(),
                2,
                16
            )
        );
    }

    #[test]
    fn self_outside_class() {
        assert_eq!(
            parse_err("fn f(self) {}"),
            (
                "`self` is only the first parameter of a method".to_string(),
                1,
                6
            )
        );
    }

    #[test]
    fn call_of_a_call() {
        assert_eq!(
            expr_err("f()()"),
            ("expected end of line, found `(`".to_string(), 1, 16)
        );
    }

    #[test]
    fn assign_to_method_call() {
        assert_eq!(
            expr_err("t.m() = 1"),
            ("cannot assign to this expression".to_string(), 1, 13)
        );
    }

    #[test]
    fn bad_class_member() {
        assert_eq!(
            parse_err("class A { 1 }"),
            ("expected a field or `fn`, found `1`".to_string(), 1, 11)
        );
    }

    #[test]
    fn class_members_need_a_line_break() {
        assert_eq!(
            parse_err("class A { x: i64 y: i64 }"),
            ("expected end of line, found `y`".to_string(), 1, 18)
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
            ("if let 1 = x {}", "expected a variable name, found `1`", 20),
            (
                "while let 1 = x {}",
                "expected a variable name, found `1`",
                23,
            ),
            ("if let n x {}", "expected `=`, found `x`", 22),
            ("if let = n {}", "expected a variable name, found `=`", 20),
        ];
        for (src, message, col) in rows {
            assert_eq!(expr_err(src), (message.to_owned(), 1, col), "{src}");
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
            assert_eq!(d.message, "`Tree??` is not a type", "{src}");
            assert_eq!(d.help.as_deref(), Some("optionals do not nest"), "{src}");
            assert_eq!(d.span, span, "{src}");
            assert_eq!(line_col(src, d.span.start), position, "{src}");
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
        const M: &str = "Sisu has no `;`: statements end at the end of the line";
        let rows = [
            (
                "let x = 1; 2",
                "Sisu has no `;`: statements end at the end of the line",
                22,
            ),
            ("let r = 0..3", "ranges are only allowed in `for`", 22),
            ("f(0..3)", "ranges are only allowed in `for`", 16),
            ("for i in 0..1..2 {}", "expected `{`, found `..`", 26),
            ("[1; 2, 3]", "expected `]`, found `,`", 18),
            ("[1; 2; 3]", M, 18),
            ("f(1; 2)", M, 16),
            ("(1; 2)", M, 15),
            (";", M, 13),
            ("a[]", "expected an expression, found `]`", 15),
        ];
        for (src, message, col) in rows {
            assert_eq!(expr_err(src), (message.to_owned(), 1, col), "{src}");
        }
    }

    #[test]
    fn stray_semicolon_in_a_class_body() {
        assert_eq!(
            parse_err("class A {\n    x: i64; y: i64\n}"),
            (
                "Sisu has no `;`: statements end at the end of the line".to_owned(),
                2,
                11
            )
        );
    }

    #[test]
    fn optional_array_types_do_not_nest() {
        let src = "fn f(x: [Node]??) {}";
        let d = parse_diagnostic(src);
        assert_eq!(d.message, "`[Node]??` is not a type");
        assert_eq!(d.help.as_deref(), Some("optionals do not nest"));
        assert_eq!(line_col(src, d.span.start), (1, 9));
    }

    /// The message, help and `line:col` of the parse error in `src`.
    fn parse_help(src: &str) -> (String, Option<String>, usize, usize) {
        let d = parse_diagnostic(src);
        let (line, col) = line_col(src, d.span.start);
        (d.message, d.help, line, col)
    }

    fn no_var(line: usize, col: usize) -> (String, Option<String>, usize, usize) {
        (
            "Sisu has no `var`".to_owned(),
            Some("use `let mut`".to_owned()),
            line,
            col,
        )
    }

    #[test]
    fn var_is_reserved_everywhere() {
        assert_eq!(parse_help("fn main() {\n    var x = 1\n}"), no_var(2, 5));
        assert_eq!(parse_help("fn var() {}"), no_var(1, 4));
        assert_eq!(parse_help("fn main() { let var = 1 }"), no_var(1, 17));
    }

    #[test]
    fn mut_is_a_keyword() {
        assert_eq!(
            parse_err("fn main() { let mut = 1 }"),
            ("expected a variable name, found `=`".to_owned(), 1, 21)
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
    fn var_field() {
        assert_eq!(
            parse_help("class P {\n    var hp: i64\n}"),
            (
                "Sisu has no `var`".to_owned(),
                Some("mark the field `mut`: `mut hp: i64`".to_owned()),
                2,
                5
            )
        );
    }

    #[test]
    fn let_field() {
        assert_eq!(
            parse_help("class P {\n    let hp: [Tree?]\n}"),
            (
                "a field takes no `let`".to_owned(),
                Some("write it bare: `hp: [Tree?]`".to_owned()),
                2,
                5
            )
        );
        // With no field after the keyword, the help shows its shape.
        assert_eq!(
            parse_help("class P {\n    let hp = 1\n}").1.as_deref(),
            Some("write it bare: `name: Type`")
        );
    }

    #[test]
    fn comma_after_a_field() {
        assert_eq!(
            parse_help("class P {\n    x: i64, y: i64\n}"),
            (
                "a field ends at the end of the line".to_owned(),
                Some("put each field on its own line".to_owned()),
                2,
                11
            )
        );
    }

    #[test]
    fn comma_after_a_method_is_not_a_field_comma() {
        assert_eq!(
            parse_err("class C {\n fn f(self) {},\n}"),
            ("expected end of line, found `,`".to_owned(), 2, 15)
        );
    }

    #[test]
    fn member_fallback() {
        assert_eq!(
            parse_err("class C {\n    1\n}"),
            ("expected a field or `fn`, found `1`".to_owned(), 2, 5)
        );
    }

    #[test]
    fn mut_self_on_a_class_method() {
        assert_eq!(
            parse_help("class C {\n    fn f(mut self) {}\n}"),
            (
                "a class method takes `self`, not `mut self`".to_owned(),
                Some("an object is shared: `self` can already write every `mut` field".to_owned()),
                2,
                10
            )
        );
    }

    #[test]
    fn mut_self_outside_the_receiver_position() {
        let message = "`self` is only the first parameter of a method";
        assert_eq!(
            parse_help("fn f(mut self) {}"),
            (message.to_owned(), None, 1, 10)
        );
        assert_eq!(
            parse_help("class C {\n    fn f(self, n: i64, mut self) {}\n}"),
            (message.to_owned(), None, 2, 28)
        );
    }

    #[test]
    fn mut_parameter() {
        assert_eq!(
            parse_help("fn f(a: i64, mut n: i64) {}"),
            (
                "a parameter cannot be `mut`".to_owned(),
                Some("copy it into a variable: `let mut n = n`".to_owned()),
                1,
                14
            )
        );
    }

    #[test]
    fn mut_before_neither_self_nor_a_name() {
        assert_eq!(
            parse_err("fn f(mut 1) {}"),
            ("expected a parameter name, found `mut`".to_owned(), 1, 6)
        );
    }
}
