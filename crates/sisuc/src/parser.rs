//! The parser: tokens to the syntax tree.

use crate::ast::{
    Block, Expr, ExprKind, Function, Ident, Param, Program, Stmt, StmtKind, TypeExpr,
};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Token, TokenKind};

/// Parses `tokens`, which must end with `Eof`. Stops at the first error.
pub(crate) fn parse(tokens: &[Token]) -> Result<Program, Diagnostic> {
    Parser { tokens, pos: 0 }.program()
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

    /// "expected <what>, found <the next token>", pointing at the next token.
    fn expected(&self, what: &str) -> Diagnostic {
        let found = self.peek();
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
        let ident = self.ident("a type")?;
        Ok(TypeExpr {
            name: ident.name,
            span: ident.span,
        })
    }

    fn program(&mut self) -> Result<Program, Diagnostic> {
        let mut functions = Vec::new();
        self.skip_newlines();
        while !self.at(&TokenKind::Eof) {
            functions.push(self.function()?);
            self.skip_newlines();
        }
        Ok(Program { functions })
    }

    fn function(&mut self) -> Result<Function, Diagnostic> {
        let fn_token = self.expect(&TokenKind::Fn)?;
        let name = self.ident("a function name")?;
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
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
                return Err(self.expected("end of line"));
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
            TokenKind::Let | TokenKind::Var => self.let_stmt()?,
            TokenKind::While => {
                self.bump();
                let cond = self.expr()?;
                StmtKind::While {
                    cond,
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
            TokenKind::Else => {
                return Err(Diagnostic::error(
                    start,
                    "`else` must be on the same line as the closing `}`",
                ));
            }
            _ => StmtKind::Expr(self.expr()?),
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

    fn let_stmt(&mut self) -> Result<StmtKind, Diagnostic> {
        let mutable = self.bump().kind == TokenKind::Var;
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

    // Task 7 turns this into the Pratt loop; for now an expression is a primary.
    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        self.primary()
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
            TokenKind::Ident(name) => {
                self.bump();
                if self.at(&TokenKind::LParen) {
                    return self.call(Ident {
                        name,
                        span: token.span,
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
            TokenKind::If => return self.if_expr(),
            _ => return Err(self.expected("an expression")),
        };
        Ok(Expr {
            kind,
            span: token.span,
        })
    }

    fn call(&mut self, callee: Ident) -> Result<Expr, Diagnostic> {
        self.expect(&TokenKind::LParen)?;
        let mut args = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                args.push(self.expr()?);
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect(&TokenKind::RParen)?;
        Ok(Expr {
            span: callee.span.to(close.span),
            kind: ExprKind::Call { callee, args },
        })
    }

    fn if_expr(&mut self) -> Result<Expr, Diagnostic> {
        let start = self.expect(&TokenKind::If)?.span;
        let cond = self.expr()?;
        let then_block = self.block()?;
        let mut end = then_block.span;
        let else_block = if self.eat(&TokenKind::Else).is_some() {
            let block = if self.at(&TokenKind::If) {
                // `else if` becomes a block holding the inner `if`.
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
        Ok(Expr {
            span: start.to(end),
            kind: ExprKind::If {
                cond: Box::new(cond),
                then_block,
                else_block,
            },
        })
    }
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

    fn body(src: &str) -> String {
        parsed(src).functions[0].body.to_string()
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
    fn typed_let_and_var() {
        assert_eq!(
            body("fn main() {\n    let x: i64 = 1\n    var y = 2\n}"),
            "(block (let x i64 1) (var y 2))"
        );
    }

    #[test]
    fn function_span() {
        assert_eq!(
            parsed("fn main() {\n}\n").functions[0].span,
            Span::new(0, 13)
        );
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
            ("expected `fn`, found `let`".to_string(), 1, 1)
        );
    }
}
