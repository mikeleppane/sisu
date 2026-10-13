//! The registry: every diagnostic code, and the message, labels, help and fixes of each.

use super::{Diagnostic, Edit, Fix, Severity, Span, Suggestion};

/// The code table: each code's variant and number. A number is never reused or changed.
macro_rules! codes {
    ($($variant:ident = $code:literal,)*) => {
        /// A diagnostic code.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[expect(
            clippy::enum_variant_names,
            reason = "each variant names its diagnostic, as `DiagnosticKind`'s do"
        )]
        pub(crate) enum Code {
            $($variant,)*
        }

        impl Code {
            pub(crate) const ALL: &[Code] = &[$(Code::$variant,)*];

            /// The code as written: `E0001`.
            pub(crate) fn as_str(self) -> &'static str {
                match self {
                    $(Code::$variant => $code,)*
                }
            }

            /// The explanation, `docs/diagnostics/<code>.md`.
            pub(crate) fn explanation(self) -> &'static str {
                match self {
                    $(Code::$variant => include_str!(
                        concat!("../../../../docs/diagnostics/", $code, ".md")
                    ),)*
                }
            }
        }
    };
}

codes! {
    AssignToImmutable = "E0001",
    UnreachableCode = "W0001",
    NeverReassigned = "W0002",
    LeadingBinaryMinus = "E0002",
    UnclosedBracket = "E0003",
    BraceOnNewLine = "E0004",
    BracketOnNewLine = "E0005",
    ElseOnNewLine = "E0006",
    UnexpectedClosingBracket = "E0007",
    DigitSeparator = "E0008",
    IntegerTooLarge = "E0009",
    UnexpectedCharacter = "E0010",
    Expected = "E0011",
    Semicolon = "E0012",
    Var = "E0013",
    OptionalNests = "E0014",
    FieldComma = "E0015",
    FieldLet = "E0016",
    MethodNeedsSelf = "E0017",
    MutSelf = "E0018",
    MutParameter = "E0019",
    SelfNotFirst = "E0020",
    AssignToExpression = "E0021",
    AssignThroughSafeAccess = "E0022",
    RangeOutsideFor = "E0023",
    EqualityChain = "E0024",
    ComparisonDirection = "E0025",
    BuiltInTypeName = "E0026",
    NeverConstructed = "E0027",
    NoMain = "E0028",
    MainSignature = "E0029",
    UnknownType = "E0030",
    BodyHasNoValue = "E0031",
    Mismatch = "E0032",
    BindingNotFound = "E0033",
    DefinedTwice = "E0034",
    ForNeedsArray = "E0035",
    ReturnNeedsValue = "E0036",
    UnexpectedReturnValue = "E0037",
    JumpOutsideLoop = "E0038",
    AlreadyDeclared = "E0039",
    IntegerDoesNotFit = "E0040",
    SelfOutsideMethod = "E0041",
    NegatedU8 = "E0042",
    NoValue = "E0043",
    NeverNone = "E0044",
    IsTestsNone = "E0045",
    IsNeedsObjects = "E0046",
    CannotCompare = "E0047",
    CoalesceNotOptional = "E0048",
    SafeAccessNotOptional = "E0049",
    CannotInfer = "E0050",
    UnwrapNeedsOptional = "E0051",
    FunctionNotFound = "E0052",
    ArgumentCount = "E0053",
    AlreadyConverted = "E0054",
    LabeledArgument = "E0055",
    MissingField = "E0056",
    MethodNotField = "E0057",
    FieldNotMethod = "E0058",
    PrintArgument = "E0059",
    PrintArray = "E0060",
    PrintNever = "E0061",
    MayBeNone = "E0062",
    NoMember = "E0063",
    TooManyArguments = "E0064",
    ArgumentNeedsLabel = "E0065",
    FieldOutOfOrder = "E0066",
    SameObject = "E0067",
    CannotIndex = "E0068",
    SumNeedsI64 = "E0069",
    MethodAsFunction = "E0070",
    NestingTooDeep = "E0071",
    InvalidUtf8 = "E0072",
    AssignToSelf = "E0073",
    AssignToFunction = "E0074",
}

impl Code {
    /// The code `text` names, in either case.
    pub(crate) fn parse(text: &str) -> Option<Code> {
        Code::ALL
            .iter()
            .copied()
            .find(|code| code.as_str().eq_ignore_ascii_case(text))
    }

    /// An `E` code is an error, a `W` code a warning.
    pub(crate) fn severity(self) -> Severity {
        if self.as_str().starts_with('E') {
            Severity::Error
        } else {
            Severity::Warning
        }
    }
}

/// What went wrong, and where: the facts each diagnostic code is built from.
pub(crate) enum DiagnosticKind {
    /// An assignment at `at` to `name`, or to an element of it, which cannot change.
    AssignToImmutable {
        name: String,
        at: Span,
        element: bool,
        cause: Immutable,
    },
    /// A statement at `at` after one that never finishes.
    UnreachableCode { at: Span },
    /// The `let mut` binding `name`, at `at`, which nothing reassigns; `keyword` is its `mut`.
    NeverReassigned {
        name: String,
        at: Span,
        keyword: Span,
    },
    /// A binary `-` at `at` that starts a line.
    LeadingBinaryMinus { at: Span },
    /// The `bracket` at `at`, which nothing closes.
    UnclosedBracket { bracket: char, at: Span },
    /// A `{` at `at` that starts a line; `join`, when the line above can take it, moves it up.
    BraceOnNewLine { at: Span, join: Option<Join> },
    /// A `[` at `at` that starts a line inside brackets, after an expression.
    BracketOnNewLine { at: Span },
    /// An `else` at `at` that starts a line; `join`, when a `}` ends the line above, moves it up.
    ElseOnNewLine { at: Span, join: Option<Join> },
    /// A closing `bracket` at `at` with no match; `opened` is the bracket that is open instead.
    UnexpectedClosingBracket {
        bracket: char,
        at: Span,
        opened: Option<(char, Span)>,
    },
    /// A `_` at `at` that is not between digits.
    DigitSeparator { at: Span },
    /// An integer literal at `at` that does not fit `i64`.
    IntegerTooLarge { at: Span },
    /// The character `c` at `at`, which Sisu does not use.
    UnexpectedCharacter { c: char, at: Span },
    /// The token `found` at `at`, where `what` should stand.
    Expected {
        what: String,
        found: String,
        at: Span,
    },
    /// A `;` at `at`.
    Semicolon { at: Span, place: SemicolonPlace },
    /// A `var` at `at`, on a field or on a binding.
    Var { at: Span, field: bool },
    /// The type `ty`, then `?` again, from `at`.
    OptionalNests { ty: String, at: Span },
    /// A `,` at `at` after a field.
    FieldComma { at: Span },
    /// A `let` at `at` in front of a field.
    FieldLet { at: Span, rest: LetField },
    /// A method named at `at` with no `self`.
    MethodNeedsSelf { at: Span },
    /// A `mut` at `keyword` in front of the `self` at `receiver`; removing it is `certain` when
    /// `self` is the whole parameter.
    MutSelf {
        keyword: Span,
        receiver: Span,
        certain: bool,
    },
    /// A `mut` at `at` in front of the parameter `name`.
    MutParameter { name: String, at: Span },
    /// A `self` at `at` that is not a method's first parameter.
    SelfNotFirst { at: Span },
    /// An assignment at `at` to something that is not a place.
    AssignToExpression { at: Span },
    /// An assignment at `at` through a `?.`.
    AssignThroughSafeAccess { at: Span },
    /// A range at `at` outside a `for`.
    RangeOutsideFor { at: Span },
    /// A second `==`, `!=` or `is` at `at` in one chain.
    EqualityChain { at: Span },
    /// A comparison at `at` that turns a chain around.
    ComparisonDirection { at: Span },
    /// The built-in type name `name`, at `at`, taken by a class or function.
    BuiltInTypeName { name: String, at: Span },
    /// The class `class`, which no object can have: its field `field`, at `at`, needs a `target`.
    /// `ty` is the field's type, where a `?` would break the cycle.
    NeverConstructed {
        class: String,
        field: String,
        target: String,
        at: Span,
        ty: Span,
    },
    /// A program with no `main`.
    NoMain,
    /// A `main` at `at` that takes parameters or returns a value.
    MainSignature { at: Span },
    /// The type `name`, at `at`, which is not built in and names no class.
    UnknownType {
        name: String,
        at: Span,
        suggestion: Option<Suggestion>,
    },
    /// The function `function`, whose body, ending at `at`, has no value of type `want`.
    BodyHasNoValue {
        function: String,
        want: String,
        at: Span,
    },
    /// A value of type `got`, at `at`, where `want` is expected; `convert` is the conversion
    /// that would fit.
    Mismatch {
        want: String,
        got: String,
        at: Span,
        convert: Option<Conversion>,
    },
    /// The binding `name`, at `at`, which no scope holds.
    BindingNotFound {
        name: String,
        at: Span,
        suggestion: Option<Suggestion>,
    },
    /// A `kind` definition named `name`, at `at`, after an earlier definition of that name, `first`.
    DefinedTwice {
        kind: Definition,
        name: String,
        at: Span,
        first: Earlier,
    },
    /// A `for` over the value at `at`, of type `found`, which is no array or range.
    ForNeedsArray { found: String, at: Span },
    /// A `return` at `at` with no value, in a function that returns a `want`.
    ReturnNeedsValue { want: String, at: Span },
    /// A `return` with the value at `at`, in a function that returns none.
    UnexpectedReturnValue { at: Span },
    /// A `break` or `continue` (`keyword`) at `at`, outside a loop.
    JumpOutsideLoop { keyword: &'static str, at: Span },
    /// The binding `name`, at `at`, declared before at `first` in the same scope.
    AlreadyDeclared { name: String, at: Span, first: Span },
    /// The integer literal `value`, at `at`, which does not fit the `u8` its position expects.
    IntegerDoesNotFit { value: i64, at: Span },
    /// A `self` at `at`, outside a method.
    SelfOutsideMethod { at: Span },
    /// A unary `-` at `at` on a `u8`.
    NegatedU8 { at: Span },
    /// An expression at `at` that has no value: it is `unit`, or, with `never`, it never
    /// finishes.
    NoValue { at: Span, never: bool },
    /// A comparison at `at` of a value that is not optional with `None`. `value` is that value's
    /// path as written, when it has one.
    NeverNone { value: Option<String>, at: Span },
    /// An `is` at `at` with `None` on one side. `tested`, when `== None` would be right, is the
    /// path of the other side as written.
    IsTestsNone { tested: Option<String>, at: Span },
    /// An `is` whose operand at `at`, of type `found`, is no object.
    IsNeedsObjects { found: String, at: Span },
    /// A comparison at `at` of a `left` with a `right`.
    CannotCompare {
        left: String,
        right: String,
        at: Span,
    },
    /// The left side of a `??`, at `at`, which is not optional.
    CoalesceNotOptional { at: Span },
    /// The receiver at `at` of a `?.`, which is not optional. `value` is its path as written,
    /// when it has one. `access` is everything between the receiver and the member name: the
    /// `?.` and any space or comment around it.
    SafeAccessNotOptional {
        value: Option<String>,
        at: Span,
        access: Span,
    },
    /// A `None` or `[]` (`literal`) at `at` whose type nothing fixes.
    CannotInfer { literal: Literal, at: Span },
    /// The value at `at`, of type `found`, that `keyword` unwraps, and which is not optional.
    UnwrapNeedsOptional {
        keyword: Unwrap,
        found: String,
        at: Span,
    },
    /// A call at `at` of `name`, which names no function.
    FunctionNotFound {
        name: String,
        at: Span,
        suggestion: Option<Suggestion>,
    },
    /// A call at `at` of `name`, which takes `want` arguments, with `found`.
    ArgumentCount {
        name: String,
        want: usize,
        found: usize,
        at: Span,
    },
    /// A conversion to `ty` of the value at `at`, which already has that type. `value` is its
    /// path as written, when it has one.
    AlreadyConverted {
        value: Option<String>,
        ty: String,
        at: Span,
    },
    /// A label at `at` on an argument of a call that is not a constructor.
    LabeledArgument { at: Span },
    /// A constructor at `at` without the field `field`; `fields` are its class's, in order.
    MissingField {
        field: String,
        fields: Vec<String>,
        at: Span,
    },
    /// The method `name` of `owner`, at `at`, read as a field. `call` is whether adding `()`
    /// is certain to fix it, when it can be added.
    MethodNotField {
        name: String,
        owner: String,
        at: Span,
        call: Option<bool>,
    },
    /// The field `name` of `owner`, at `at`, called as a method.
    FieldNotMethod {
        name: String,
        owner: String,
        at: Span,
    },
    /// A `print` at `at` with no argument, several, or one it cannot print. `class` is the class
    /// of an object argument, and `at` is then that argument.
    PrintArgument { at: Span, class: Option<String> },
    /// The argument at `at` of a `print`, of the array type `ty` or an optional of one.
    PrintArray {
        ty: String,
        optional: bool,
        at: Span,
    },
    /// The argument at `at` of a `print`, which never finishes.
    PrintNever { at: Span },
    /// The optional at `at` that a member, an index or a `for` uses as a value. `value` is its
    /// path as written, when it has one. `access` is the `.` before the member, and any space or
    /// comment around it, when a `?.` can take its place.
    MayBeNone {
        value: Option<String>,
        at: Span,
        access: Option<Span>,
    },
    /// The `kind` member `name`, at `at`, which `owner` does not have.
    NoMember {
        owner: String,
        kind: Definition,
        name: String,
        at: Span,
        suggestion: Option<Suggestion>,
    },
    /// The constructor argument at `at`, after the last of the `fields` fields of `class`.
    TooManyArguments {
        class: String,
        fields: usize,
        at: Span,
    },
    /// The constructor argument at `at`, without a label; `field` is the field it sets.
    ArgumentNeedsLabel { field: String, at: Span },
    /// The label `found`, at `at`, where the constructor names `field`; `fields` are the
    /// class's, in order.
    FieldOutOfOrder {
        field: String,
        found: String,
        fields: Vec<String>,
        at: Span,
    },
    /// The value at `at` of an array fill, an object of `class` or an optional of one.
    SameObject { class: String, at: Span },
    /// An index into the value at `at`, of type `ty`, which is no array.
    CannotIndex { ty: String, at: Span },
    /// A `sum` at `at` of an array of type `ty`, which is not `[i64]`.
    SumNeedsI64 { ty: String, at: Span },
    /// A call at `at` of the array method `name` as a function, with the array `receiver` as
    /// its first argument. `next` is the next argument, if any. `wrap` is whether `receiver`
    /// needs parentheses to take a `.name(...)`, and `certain` whether the method call checks.
    MethodAsFunction {
        name: String,
        at: Span,
        receiver: Span,
        next: Option<Span>,
        wrap: bool,
        certain: bool,
    },
    /// The token at `at`, where the tree passes `limit` levels.
    NestingTooDeep { at: Span, limit: usize },
    /// The first byte at `at` that is not UTF-8, which the text shown in its place replaces.
    InvalidUtf8 { at: Span },
    /// An assignment to the `self` at `at`.
    AssignToSelf { at: Span },
    /// An assignment to `name`, at `at`, which is a function.
    AssignToFunction { name: String, at: Span },
}

/// A literal that takes its type from its position.
#[derive(Clone, Copy)]
pub(crate) enum Literal {
    None,
    EmptyArray,
}

/// The statement or expression that unwraps an optional.
#[derive(Clone, Copy)]
pub(crate) enum Unwrap {
    IfLet,
    WhileLet,
}

impl Unwrap {
    fn word(self) -> &'static str {
        match self {
            Unwrap::IfLet => "if let",
            Unwrap::WhileLet => "while let",
        }
    }
}

/// What a name is defined as.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Definition {
    Function,
    Class,
    Field,
    Method,
    Binding,
    Type,
}

impl Definition {
    fn word(self) -> &'static str {
        match self {
            Definition::Function => "function",
            Definition::Class => "class",
            Definition::Field => "field",
            Definition::Method => "method",
            Definition::Binding => "binding",
            Definition::Type => "type",
        }
    }
}

/// Where the earlier definition of a name is.
#[derive(Clone)]
pub(crate) enum Earlier {
    /// In the source, at `span`, as a `kind`; `owner` is the class of a field or method.
    At {
        span: Span,
        kind: Definition,
        owner: Option<String>,
    },
    /// In the language: `print`.
    BuiltIn,
}

/// Converting the expression at `operand` to the type `to`: `u8` or `i64`.
#[derive(Clone, Copy)]
pub(crate) struct Conversion {
    pub(crate) to: &'static str,
    pub(crate) operand: Span,
}

/// Moving a token up to the code before it: the `gap` between them becomes a space.
#[derive(Clone, Copy)]
pub(crate) struct Join {
    pub(crate) gap: Span,
    /// Whether the move is always right, not only likely.
    pub(crate) certain: bool,
}

/// What a `;` follows, and so what can be done about it.
#[derive(Clone, Copy)]
pub(crate) enum SemicolonPlace {
    /// It ends a complete statement or member, and a line break or `}` follows.
    EndsLine,
    /// It ends a complete statement or member, and more code follows on the line.
    MidLine,
    /// It ends a complete statement or member, and another `;` follows.
    Repeated,
}

/// What follows the `let` in front of a field.
pub(crate) enum LetField {
    /// A field that ends the line: deleting `remove`, from the `let` to the next token, fixes it.
    Whole { remove: Span },
    /// Anything else; `field` is `name: Type`, or a placeholder when none follows, and `mutable`
    /// whether `mut` follows the `let`.
    Other { mutable: bool, field: String },
}

/// Why a place cannot change; each `declared` is the name where it was declared.
pub(crate) enum Immutable {
    Let {
        declared: Span,
    },
    Param {
        declared: Span,
    },
    /// Bound by `keyword`: `for`, `if let` or `while let`.
    Unwrapped {
        keyword: &'static str,
        declared: Span,
    },
    /// A field declared without `mut`.
    Field {
        declared: Span,
    },
}

impl DiagnosticKind {
    pub(crate) fn code(&self) -> Code {
        match self {
            DiagnosticKind::AssignToImmutable { .. } => Code::AssignToImmutable,
            DiagnosticKind::UnreachableCode { .. } => Code::UnreachableCode,
            DiagnosticKind::NeverReassigned { .. } => Code::NeverReassigned,
            DiagnosticKind::LeadingBinaryMinus { .. } => Code::LeadingBinaryMinus,
            DiagnosticKind::UnclosedBracket { .. } => Code::UnclosedBracket,
            DiagnosticKind::BraceOnNewLine { .. } => Code::BraceOnNewLine,
            DiagnosticKind::BracketOnNewLine { .. } => Code::BracketOnNewLine,
            DiagnosticKind::ElseOnNewLine { .. } => Code::ElseOnNewLine,
            DiagnosticKind::UnexpectedClosingBracket { .. } => Code::UnexpectedClosingBracket,
            DiagnosticKind::DigitSeparator { .. } => Code::DigitSeparator,
            DiagnosticKind::IntegerTooLarge { .. } => Code::IntegerTooLarge,
            DiagnosticKind::UnexpectedCharacter { .. } => Code::UnexpectedCharacter,
            DiagnosticKind::Expected { .. } => Code::Expected,
            DiagnosticKind::Semicolon { .. } => Code::Semicolon,
            DiagnosticKind::Var { .. } => Code::Var,
            DiagnosticKind::OptionalNests { .. } => Code::OptionalNests,
            DiagnosticKind::FieldComma { .. } => Code::FieldComma,
            DiagnosticKind::FieldLet { .. } => Code::FieldLet,
            DiagnosticKind::MethodNeedsSelf { .. } => Code::MethodNeedsSelf,
            DiagnosticKind::MutSelf { .. } => Code::MutSelf,
            DiagnosticKind::MutParameter { .. } => Code::MutParameter,
            DiagnosticKind::SelfNotFirst { .. } => Code::SelfNotFirst,
            DiagnosticKind::AssignToExpression { .. } => Code::AssignToExpression,
            DiagnosticKind::AssignThroughSafeAccess { .. } => Code::AssignThroughSafeAccess,
            DiagnosticKind::RangeOutsideFor { .. } => Code::RangeOutsideFor,
            DiagnosticKind::EqualityChain { .. } => Code::EqualityChain,
            DiagnosticKind::ComparisonDirection { .. } => Code::ComparisonDirection,
            DiagnosticKind::BuiltInTypeName { .. } => Code::BuiltInTypeName,
            DiagnosticKind::NeverConstructed { .. } => Code::NeverConstructed,
            DiagnosticKind::NoMain => Code::NoMain,
            DiagnosticKind::MainSignature { .. } => Code::MainSignature,
            DiagnosticKind::UnknownType { .. } => Code::UnknownType,
            DiagnosticKind::BodyHasNoValue { .. } => Code::BodyHasNoValue,
            DiagnosticKind::Mismatch { .. } => Code::Mismatch,
            DiagnosticKind::BindingNotFound { .. } => Code::BindingNotFound,
            DiagnosticKind::DefinedTwice { .. } => Code::DefinedTwice,
            DiagnosticKind::ForNeedsArray { .. } => Code::ForNeedsArray,
            DiagnosticKind::ReturnNeedsValue { .. } => Code::ReturnNeedsValue,
            DiagnosticKind::UnexpectedReturnValue { .. } => Code::UnexpectedReturnValue,
            DiagnosticKind::JumpOutsideLoop { .. } => Code::JumpOutsideLoop,
            DiagnosticKind::AlreadyDeclared { .. } => Code::AlreadyDeclared,
            DiagnosticKind::IntegerDoesNotFit { .. } => Code::IntegerDoesNotFit,
            DiagnosticKind::SelfOutsideMethod { .. } => Code::SelfOutsideMethod,
            DiagnosticKind::NegatedU8 { .. } => Code::NegatedU8,
            DiagnosticKind::NoValue { .. } => Code::NoValue,
            DiagnosticKind::NeverNone { .. } => Code::NeverNone,
            DiagnosticKind::IsTestsNone { .. } => Code::IsTestsNone,
            DiagnosticKind::IsNeedsObjects { .. } => Code::IsNeedsObjects,
            DiagnosticKind::CannotCompare { .. } => Code::CannotCompare,
            DiagnosticKind::CoalesceNotOptional { .. } => Code::CoalesceNotOptional,
            DiagnosticKind::SafeAccessNotOptional { .. } => Code::SafeAccessNotOptional,
            DiagnosticKind::CannotInfer { .. } => Code::CannotInfer,
            DiagnosticKind::UnwrapNeedsOptional { .. } => Code::UnwrapNeedsOptional,
            DiagnosticKind::FunctionNotFound { .. } => Code::FunctionNotFound,
            DiagnosticKind::ArgumentCount { .. } => Code::ArgumentCount,
            DiagnosticKind::AlreadyConverted { .. } => Code::AlreadyConverted,
            DiagnosticKind::LabeledArgument { .. } => Code::LabeledArgument,
            DiagnosticKind::MissingField { .. } => Code::MissingField,
            DiagnosticKind::MethodNotField { .. } => Code::MethodNotField,
            DiagnosticKind::FieldNotMethod { .. } => Code::FieldNotMethod,
            DiagnosticKind::PrintArgument { .. } => Code::PrintArgument,
            DiagnosticKind::PrintArray { .. } => Code::PrintArray,
            DiagnosticKind::PrintNever { .. } => Code::PrintNever,
            DiagnosticKind::MayBeNone { .. } => Code::MayBeNone,
            DiagnosticKind::NoMember { .. } => Code::NoMember,
            DiagnosticKind::TooManyArguments { .. } => Code::TooManyArguments,
            DiagnosticKind::ArgumentNeedsLabel { .. } => Code::ArgumentNeedsLabel,
            DiagnosticKind::FieldOutOfOrder { .. } => Code::FieldOutOfOrder,
            DiagnosticKind::SameObject { .. } => Code::SameObject,
            DiagnosticKind::CannotIndex { .. } => Code::CannotIndex,
            DiagnosticKind::SumNeedsI64 { .. } => Code::SumNeedsI64,
            DiagnosticKind::MethodAsFunction { .. } => Code::MethodAsFunction,
            DiagnosticKind::NestingTooDeep { .. } => Code::NestingTooDeep,
            DiagnosticKind::InvalidUtf8 { .. } => Code::InvalidUtf8,
            DiagnosticKind::AssignToSelf { .. } => Code::AssignToSelf,
            DiagnosticKind::AssignToFunction { .. } => Code::AssignToFunction,
        }
    }
}

impl From<DiagnosticKind> for Diagnostic {
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per diagnostic code, so the table reads in one place"
    )]
    fn from(kind: DiagnosticKind) -> Diagnostic {
        let code = kind.code();
        match kind {
            DiagnosticKind::AssignToImmutable {
                name,
                at,
                element,
                cause,
            } => {
                let what = if element { "an element of " } else { "" };
                let d = Diagnostic::coded(code, at, format!("cannot assign to {what}`{name}`"));
                match cause {
                    Immutable::Let { declared } => if element {
                        d
                    } else {
                        d.with_label("cannot assign twice")
                    }
                    .with_secondary(declared, "declared with `let` here")
                    .with_fix(insert_mut("declare it with `let mut`", declared)),
                    Immutable::Param { declared } => d
                        .with_secondary(declared, "declared as a parameter here")
                        .with_help(format!(
                            "copy it into a variable: `let mut {name} = {name}`"
                        )),
                    Immutable::Unwrapped { keyword, declared } => {
                        d.with_secondary(declared, format!("bound by `{keyword}` here"))
                    }
                    Immutable::Field { declared } => d
                        .with_secondary(declared, "declared without `mut` here")
                        .with_fix(insert_mut("mark the field `mut`", declared)),
                }
            }
            DiagnosticKind::UnreachableCode { at } => {
                Diagnostic::coded(code, at, "unreachable code")
            }
            DiagnosticKind::NeverReassigned { name, at, keyword } => {
                Diagnostic::coded(code, at, format!("`{name}` is never reassigned")).with_fix(Fix {
                    message: "declare it with `let`".to_string(),
                    certain: true,
                    edits: vec![Edit {
                        span: Span::new(keyword.start, at.start),
                        replacement: String::new(),
                    }],
                })
            }
            DiagnosticKind::LeadingBinaryMinus { at } => {
                Diagnostic::coded(code, at, "a line cannot start with a binary `-`").with_help(
                    "to continue the previous line, end it with `-`; to negate, write `-x`",
                )
            }
            DiagnosticKind::UnclosedBracket { bracket, at } => {
                Diagnostic::coded(code, at, format!("unclosed `{bracket}`"))
            }
            DiagnosticKind::BraceOnNewLine { at, join } => {
                let d = Diagnostic::coded(
                    code,
                    at,
                    "`{` must be on the same line as `fn`, `class`, `if`, `else`, `while` or `for`",
                );
                match join {
                    Some(join) => d.with_fix(join_fix(
                        "move `{` to the end of the line above",
                        "if this `{` opens the block above, move it to the end of that line",
                        join,
                    )),
                    None => d,
                }
            }
            DiagnosticKind::BracketOnNewLine { at } => {
                Diagnostic::coded(code, at, "`[` cannot start a line here")
                    .with_help("add a comma, or put `[` on the line of its array")
            }
            DiagnosticKind::ElseOnNewLine { at, join } => {
                let d = Diagnostic::coded(
                    code,
                    at,
                    "`else` must be on the same line as the closing `}`",
                );
                match join {
                    Some(join) => d.with_fix(join_fix(
                        "move `else` after the `}`",
                        "if this `else` belongs to the `if` above, move it after the `}`",
                        join,
                    )),
                    None => d,
                }
            }
            DiagnosticKind::UnexpectedClosingBracket {
                bracket,
                at,
                opened,
            } => {
                let d = Diagnostic::coded(code, at, format!("unexpected `{bracket}`"));
                match opened {
                    Some((open, span)) => d.with_secondary(span, format!("`{open}` opened here")),
                    None => d,
                }
            }
            DiagnosticKind::DigitSeparator { at } => {
                Diagnostic::coded(code, at, "`_` must sit between digits")
            }
            DiagnosticKind::IntegerTooLarge { at } => {
                Diagnostic::coded(code, at, "integer literal is too large for `i64`")
            }
            DiagnosticKind::UnexpectedCharacter { c, at } => {
                // A character the user cannot see is named by its code point, not printed.
                let shown = if c.is_whitespace() || c.is_control() || is_format(c) {
                    format!("U+{:04X}", u32::from(c))
                } else {
                    format!("`{c}`")
                };
                let d = Diagnostic::coded(code, at, format!("unexpected character {shown}"));
                match c {
                    '&' => d.with_fix(guess("if you meant logical and, use `&&`", at, "&&")),
                    '|' => d.with_fix(guess("if you meant logical or, use `||`", at, "||")),
                    '\u{FEFF}' if at.start == 0 => d.with_fix(Fix {
                        message: "remove the byte order mark at the start of the file".to_string(),
                        certain: true,
                        edits: vec![Edit {
                            span: at,
                            replacement: String::new(),
                        }],
                    }),
                    _ => d,
                }
            }
            DiagnosticKind::Expected { what, found, at } => {
                Diagnostic::coded(code, at, format!("expected {what}, found {found}"))
            }
            DiagnosticKind::Semicolon { at, place } => {
                let d = Diagnostic::coded(
                    code,
                    at,
                    "a statement ends at the end of its line, not with `;`",
                );
                match place {
                    SemicolonPlace::EndsLine => d.with_fix(Fix {
                        message: "remove the `;`".to_string(),
                        certain: true,
                        edits: vec![Edit {
                            span: at,
                            replacement: String::new(),
                        }],
                    }),
                    SemicolonPlace::MidLine => {
                        d.with_help("put the next statement on its own line")
                    }
                    SemicolonPlace::Repeated => d,
                }
            }
            DiagnosticKind::Var { at, field } => {
                let d = Diagnostic::coded(code, at, "there is no `var` in Sisu");
                if field {
                    d.with_help("a field that can change is marked `mut`")
                } else {
                    d.with_help("Sisu declares a variable with `let mut`")
                }
            }
            DiagnosticKind::OptionalNests { ty, at } => {
                Diagnostic::coded(code, at, format!("`{ty}??` is not a type"))
                    .with_help("optionals do not nest")
            }
            DiagnosticKind::FieldComma { at } => {
                Diagnostic::coded(code, at, "a field ends at the end of the line")
                    .with_help("put each field on its own line")
            }
            DiagnosticKind::FieldLet { at, rest } => {
                let d = Diagnostic::coded(code, at, "a field takes no `let`");
                match rest {
                    LetField::Whole { remove } => d.with_fix(Fix {
                        message: "remove the `let`".to_string(),
                        certain: true,
                        edits: vec![Edit {
                            span: remove,
                            replacement: String::new(),
                        }],
                    }),
                    LetField::Other {
                        mutable: true,
                        field,
                    } => d.with_help(format!("drop the `let`: `mut {field}`")),
                    LetField::Other {
                        mutable: false,
                        field,
                    } => d.with_help(format!("write it bare: `{field}`")),
                }
            }
            DiagnosticKind::MethodNeedsSelf { at } => {
                Diagnostic::coded(code, at, "a method needs `self` as its first parameter")
                    .with_help("static methods are not supported yet")
            }
            DiagnosticKind::MutSelf {
                keyword,
                receiver,
                certain,
            } => Diagnostic::coded(code, keyword, "a class method takes `self`, not `mut self`")
                .with_help("an object is shared: `self` can already write every `mut` field")
                .with_fix(Fix {
                    message: if certain {
                        "remove the `mut`"
                    } else {
                        "try removing the `mut`; the receiver may need another change"
                    }
                    .to_string(),
                    certain,
                    edits: vec![Edit {
                        span: Span::new(keyword.start, receiver.start),
                        replacement: String::new(),
                    }],
                }),
            DiagnosticKind::MutParameter { name, at } => {
                Diagnostic::coded(code, at, "a parameter cannot be `mut`").with_help(format!(
                    "copy it into a variable: `let mut {name} = {name}`"
                ))
            }
            DiagnosticKind::SelfNotFirst { at } => {
                Diagnostic::coded(code, at, "`self` is only the first parameter of a method")
            }
            DiagnosticKind::AssignToExpression { at } => {
                Diagnostic::coded(code, at, "cannot assign to this expression")
            }
            DiagnosticKind::AssignThroughSafeAccess { at } => {
                Diagnostic::coded(code, at, "cannot assign through `?.`")
                    .with_help("unwrap it with `if let`")
            }
            DiagnosticKind::RangeOutsideFor { at } => {
                Diagnostic::coded(code, at, "ranges are only allowed in `for`")
            }
            DiagnosticKind::EqualityChain { at } => {
                Diagnostic::coded(code, at, "`==`, `!=` and `is` do not chain")
                    .with_help("join the comparisons with `&&`")
            }
            DiagnosticKind::ComparisonDirection { at } => {
                Diagnostic::coded(code, at, "a comparison chain must go in one direction")
            }
            DiagnosticKind::BuiltInTypeName { name, at } => {
                Diagnostic::coded(code, at, format!("`{name}` is a built-in type"))
            }
            DiagnosticKind::NeverConstructed {
                class,
                field,
                target,
                at,
                ty,
            } => Diagnostic::coded(
                code,
                at,
                format!("`{class}` can never be constructed: field `{field}` needs a value of type `{target}`"),
            )
            .with_fix(Fix {
                message: format!("if the field can be absent, make it optional: `{target}?`"),
                certain: false,
                edits: vec![Edit {
                    span: Span::new(ty.end, ty.end),
                    replacement: "?".to_string(),
                }],
            }),
            DiagnosticKind::NoMain => {
                Diagnostic::coded(code, Span::new(0, 0), "no `main` function")
            }
            DiagnosticKind::MainSignature { at } => {
                Diagnostic::coded(code, at, "`main` takes no parameters and returns no value")
            }
            DiagnosticKind::UnknownType {
                name,
                at,
                suggestion,
            } => similar(
                Diagnostic::coded(code, at, format!("cannot find type `{name}`")).with_help(
                    "the types are `i64`, `u8`, `bool`, classes and arrays such as `[i64]`",
                ),
                at,
                suggestion,
            ),
            DiagnosticKind::BodyHasNoValue { function, want, at } => Diagnostic::coded(
                code,
                at,
                format!("function `{function}` must return `{want}`, but its body has no value"),
            ),
            DiagnosticKind::Mismatch {
                want,
                got,
                at,
                convert,
            } => {
                let d = Diagnostic::coded(code, at, format!("expected `{want}`, found `{got}`"));
                match convert {
                    Some(Conversion { to, operand }) => d.with_fix(Fix {
                        message: format!("if the types are right, convert it with `{to}(...)`"),
                        certain: false,
                        edits: vec![
                            Edit {
                                span: Span::new(operand.start, operand.start),
                                replacement: format!("{to}("),
                            },
                            Edit {
                                span: Span::new(operand.end, operand.end),
                                replacement: ")".to_string(),
                            },
                        ],
                    }),
                    None => d,
                }
            }
            DiagnosticKind::BindingNotFound {
                name,
                at,
                suggestion,
            } => similar(
                Diagnostic::coded(code, at, format!("cannot find binding `{name}`")),
                at,
                suggestion,
            ),
            DiagnosticKind::DefinedTwice {
                kind,
                name,
                at,
                first,
            } => {
                let (message, span) = match first {
                    Earlier::At {
                        span,
                        kind: first,
                        owner,
                    } if first != kind => {
                        let owner = owner.map_or_else(String::new, |c| format!(" of `{c}`"));
                        (format!("`{name}` is already a {}{owner}", first.word()), Some(span))
                    }
                    Earlier::At { span, .. } => {
                        (format!("{} `{name}` is defined twice", kind.word()), Some(span))
                    }
                    Earlier::BuiltIn => (format!("`{name}` is a built-in function"), None),
                };
                let d = Diagnostic::coded(code, at, message);
                match span {
                    Some(span) => d.with_secondary(span, "first defined here"),
                    None => d,
                }
            }
            DiagnosticKind::ForNeedsArray { found, at } => Diagnostic::coded(
                code,
                at,
                format!("`for` needs an array or a range; this is `{found}`"),
            ),
            DiagnosticKind::ReturnNeedsValue { want, at } => {
                Diagnostic::coded(code, at, format!("`return` needs a value of type `{want}`"))
            }
            DiagnosticKind::UnexpectedReturnValue { at } => {
                Diagnostic::coded(code, at, "this function returns no value")
            }
            DiagnosticKind::JumpOutsideLoop { keyword, at } => {
                Diagnostic::coded(code, at, format!("`{keyword}` outside a loop"))
            }
            DiagnosticKind::AlreadyDeclared { name, at, first } => Diagnostic::coded(
                code,
                at,
                format!("`{name}` is already declared in this scope"),
            )
            .with_secondary(first, "first declared here"),
            DiagnosticKind::IntegerDoesNotFit { value, at } => {
                Diagnostic::coded(code, at, format!("`{value}` does not fit in `u8`"))
            }
            DiagnosticKind::SelfOutsideMethod { at } => {
                Diagnostic::coded(code, at, "`self` is only available in a method")
            }
            DiagnosticKind::NegatedU8 { at } => {
                Diagnostic::coded(code, at, "`u8` cannot be negative")
            }
            DiagnosticKind::NoValue { at, never } => {
                let d = Diagnostic::coded(code, at, "expression has no value");
                if never {
                    d.with_label("never finishes")
                } else {
                    d
                }
            }
            DiagnosticKind::NeverNone { value, at } => Diagnostic::coded(
                code,
                at,
                format!("{} is not optional, so it is never `None`", named(value)),
            ),
            DiagnosticKind::IsTestsNone { tested, at } => {
                let d = Diagnostic::coded(code, at, "`is` tests identity, and `None` is not an object");
                match tested {
                    Some(path) => d.with_help(format!("use `{path} == None`")),
                    None => d,
                }
            }
            DiagnosticKind::IsNeedsObjects { found, at } => {
                Diagnostic::coded(code, at, format!("`is` needs objects; found `{found}`"))
                    .with_help("use `==` to compare values")
            }
            DiagnosticKind::CannotCompare { left, right, at } => {
                Diagnostic::coded(code, at, format!("cannot compare `{left}` with `{right}`"))
            }
            DiagnosticKind::CoalesceNotOptional { at } => {
                Diagnostic::coded(code, at, "the left side of `??` is not optional")
            }
            DiagnosticKind::SafeAccessNotOptional { value, at, access } => {
                Diagnostic::coded(code, at, format!("{} is not optional", named(value))).with_fix(
                    Fix {
                        message: "replace `?.` with `.`".to_string(),
                        certain: true,
                        edits: vec![Edit {
                            span: access,
                            replacement: ".".to_string(),
                        }],
                    },
                )
            }
            DiagnosticKind::CannotInfer { literal, at } => {
                let (literal, example) = match literal {
                    Literal::None => ("None", "let x: Tree? = None"),
                    Literal::EmptyArray => ("[]", "let xs: [i64] = []"),
                };
                Diagnostic::coded(code, at, format!("cannot infer the type of `{literal}`"))
                    .with_help(format!("write the type: `{example}`"))
            }
            DiagnosticKind::UnwrapNeedsOptional { keyword, found, at } => Diagnostic::coded(
                code,
                at,
                format!("`{}` needs an optional; this is `{found}`", keyword.word()),
            ),
            DiagnosticKind::FunctionNotFound {
                name,
                at,
                suggestion,
            } => similar(
                Diagnostic::coded(code, at, format!("cannot find function `{name}`")),
                at,
                suggestion,
            ),
            DiagnosticKind::ArgumentCount {
                name,
                want,
                found,
                at,
            } => {
                let plural = if want == 1 { "" } else { "s" };
                Diagnostic::coded(
                    code,
                    at,
                    format!("`{name}` takes {want} argument{plural}, found {found}"),
                )
            }
            DiagnosticKind::AlreadyConverted { value, ty, at } => {
                Diagnostic::coded(code, at, format!("{} is already `{ty}`", named(value)))
            }
            DiagnosticKind::LabeledArgument { at } => {
                Diagnostic::coded(code, at, "labels on arguments come in milestone 10")
            }
            DiagnosticKind::MissingField { field, fields, at } => {
                Diagnostic::coded(code, at, format!("missing field `{field}`"))
                    .with_help(field_order(&fields))
            }
            DiagnosticKind::MethodNotField {
                name,
                owner,
                at,
                call,
            } => {
                let d = Diagnostic::coded(
                    code,
                    at,
                    format!("`{name}` is a method of `{owner}`, not a field"),
                );
                match call {
                    Some(certain) => d.with_fix(Fix {
                        message: if certain {
                            format!("call it: `{name}()`")
                        } else {
                            format!("if you meant to call `{name}`, add `()`; it also needs its arguments")
                        },
                        certain,
                        edits: vec![Edit {
                            span: Span::new(at.end, at.end),
                            replacement: "()".to_string(),
                        }],
                    }),
                    None => d,
                }
            }
            DiagnosticKind::FieldNotMethod { name, owner, at } => Diagnostic::coded(
                code,
                at,
                format!("`{name}` is a field of `{owner}`, not a method"),
            ),
            DiagnosticKind::PrintArgument { at, class } => {
                let takes = "`print` takes one `i64`, `u8` or `bool`";
                match class {
                    Some(class) => Diagnostic::coded(code, at, format!("cannot print `{class}`"))
                        .with_help(takes),
                    None => Diagnostic::coded(code, at, takes),
                }
            }
            DiagnosticKind::PrintArray { ty, optional, at } => {
                let help = if optional {
                    "unwrap it with `if let`, then print each element in a `for` loop"
                } else {
                    "print each element in a `for` loop"
                };
                Diagnostic::coded(code, at, format!("cannot print `{ty}`")).with_help(help)
            }
            DiagnosticKind::PrintNever { at } => {
                Diagnostic::coded(code, at, "`print` never runs: its argument never finishes")
            }
            DiagnosticKind::MayBeNone { value, at, access } => {
                let d = Diagnostic::coded(code, at, format!("{} may be `None`", named(value)));
                match access {
                    Some(access) => d
                        .with_help("to use the value only when it is present, unwrap it with `if let`")
                        .with_fix(guess("if a `None` result is fine, use `?.`", access, "?.")),
                    None => d.with_help("unwrap it with `if let`"),
                }
            }
            DiagnosticKind::NoMember {
                owner,
                kind,
                name,
                at,
                suggestion,
            } => similar(
                Diagnostic::coded(
                    code,
                    at,
                    format!("`{owner}` has no {} `{name}`", kind.word()),
                ),
                at,
                suggestion,
            ),
            DiagnosticKind::TooManyArguments { class, fields, at } => {
                let plural = if fields == 1 { "" } else { "s" };
                Diagnostic::coded(
                    code,
                    at,
                    format!("too many arguments: `{class}` has {fields} field{plural}"),
                )
            }
            DiagnosticKind::ArgumentNeedsLabel { field, at } => {
                Diagnostic::coded(code, at, "this argument needs a label").with_fix(Fix {
                    message: "label it with its field".to_string(),
                    certain: true,
                    edits: vec![Edit {
                        span: Span::new(at.start, at.start),
                        replacement: format!("{field}: "),
                    }],
                })
            }
            DiagnosticKind::FieldOutOfOrder {
                field,
                found,
                fields,
                at,
            } => Diagnostic::coded(
                code,
                at,
                format!("expected field `{field}` here, found `{found}`"),
            )
            .with_help(field_order(&fields)),
            DiagnosticKind::SameObject { class, at } => Diagnostic::coded(
                code,
                at,
                format!("every element would be the same `{class}` object"),
            )
            .with_help(format!(
                "start from `[None; n]` with type `[{class}?]` and fill it in a loop"
            )),
            DiagnosticKind::CannotIndex { ty, at } => {
                Diagnostic::coded(code, at, format!("cannot index `{ty}`"))
                    .with_help("only arrays can be indexed")
            }
            DiagnosticKind::SumNeedsI64 { ty, at } => {
                Diagnostic::coded(code, at, format!("`sum` needs an `[i64]`, found `{ty}`"))
            }
            DiagnosticKind::MethodAsFunction {
                name,
                at,
                receiver,
                next,
                wrap,
                certain,
            } => {
                let (open, close) = if wrap { ("(", ")") } else { ("", "") };
                Diagnostic::coded(
                    code,
                    at,
                    format!("`{name}` is a method of arrays, not a function"),
                )
                .with_fix(Fix {
                    message: if certain {
                        format!("call `{name}` as a method")
                    } else {
                        format!("try calling `{name}` as a method, then fix its arguments")
                    },
                    certain,
                    edits: vec![
                        Edit {
                            span: Span::new(at.start, receiver.start),
                            replacement: open.to_string(),
                        },
                        Edit {
                            span: Span::new(receiver.end, next.map_or(receiver.end, |n| n.start)),
                            replacement: format!("{close}.{name}("),
                        },
                    ],
                })
            }
            DiagnosticKind::NestingTooDeep { at, limit } => {
                Diagnostic::coded(code, at, "nesting is too deep")
                    .with_help(format!("the limit is {limit} levels"))
            }
            DiagnosticKind::InvalidUtf8 { at } => {
                Diagnostic::coded(code, at, "source is not valid UTF-8")
                    .with_help("save the file as UTF-8")
            }
            DiagnosticKind::AssignToSelf { at } => {
                Diagnostic::coded(code, at, "cannot assign to `self`")
                    .with_help("an object is shared: `self` can already write every `mut` field")
            }
            DiagnosticKind::AssignToFunction { name, at } => {
                Diagnostic::coded(code, at, format!("`{name}` is a function, not a variable"))
            }
        }
    }
}

/// The help that lists a class's `fields` in declaration order.
fn field_order(fields: &[String]) -> String {
    let names: Vec<_> = fields.iter().map(|f| format!("`{f}`")).collect();
    format!("name the fields in declaration order: {}", names.join(", "))
}

/// A value as a message names it: its `path` in backticks, or "this value" when it has none.
fn named(path: Option<String>) -> String {
    path.map_or_else(|| "this value".to_string(), |path| format!("`{path}`"))
}

/// The fix that replaces the gap of `join` with a space: `message`, or `guess` when `join` is
/// not certain.
fn join_fix(message: &str, guess: &str, join: Join) -> Fix {
    Fix {
        message: if join.certain { message } else { guess }.to_string(),
        certain: join.certain,
        edits: vec![Edit {
            span: join.gap,
            replacement: " ".to_string(),
        }],
    }
}

/// `d`, with the guess that the name at `at` meant `suggestion`, if there is one.
fn similar(d: Diagnostic, at: Span, suggestion: Option<Suggestion>) -> Diagnostic {
    let Some(Suggestion { kind, name }) = suggestion else {
        return d;
    };
    d.with_fix(guess(
        &format!("a {} with a similar name exists: `{name}`", kind.word()),
        at,
        &name,
    ))
}

/// Whether `c` is in Unicode category Cf (format). Neither the standard library nor a dependency
/// has the category, so these are the Cf ranges of Unicode 17.0 (`DerivedGeneralCategory.txt`),
/// the `char::UNICODE_VERSION` of the pinned toolchain. Recheck the list on a toolchain upgrade;
/// a new Cf character prints raw until it is extended. `core` has a private
/// `char::is_format_control`; replace the list with it if it is ever stabilized.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{AD}' | '\u{600}'..='\u{605}' | '\u{61C}' | '\u{6DD}' | '\u{70F}'
        | '\u{890}'..='\u{891}' | '\u{8E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{110CD}'
        | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0001}' | '\u{E0020}'..='\u{E007F}')
}

/// The uncertain fix `message` that replaces `span` with `replacement`.
fn guess(message: &str, span: Span, replacement: &str) -> Fix {
    Fix {
        message: message.to_string(),
        certain: false,
        edits: vec![Edit {
            span,
            replacement: replacement.to_string(),
        }],
    }
}

/// The certain fix `message` that writes `mut ` before the name at `declared`.
fn insert_mut(message: &str, declared: Span) -> Fix {
    Fix {
        message: message.to_string(),
        certain: true,
        edits: vec![Edit {
            span: Span::new(declared.start, declared.start),
            replacement: "mut ".to_string(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_parses_both_cases() {
        assert_eq!(Code::parse("E0001"), Some(Code::AssignToImmutable));
        assert_eq!(Code::parse("e0001"), Some(Code::AssignToImmutable));
        assert_eq!(Code::parse("E9999"), None);
    }

    /// The ends of the list and a range inside it, and neighbours outside it.
    #[test]
    fn format_characters_are_category_cf() {
        for c in [
            '\u{AD}',
            '\u{200B}',
            '\u{200F}',
            '\u{202E}',
            '\u{FEFF}',
            '\u{E0001}',
            '\u{E007F}',
        ] {
            assert!(is_format(c), "{c:?}");
        }
        for c in [
            'a',
            '\u{AC}',
            '\u{AE}',
            '\u{2010}',
            '\u{E0002}',
            '\u{E0080}',
            '\u{3042}',
        ] {
            assert!(!is_format(c), "{c:?}");
        }
    }

    #[test]
    fn codes_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for &code in Code::ALL {
            let text = code.as_str();
            let (series, digits) = text.split_at(1);
            assert!(
                matches!(series, "E" | "W")
                    && digits.len() == 4
                    && digits.bytes().all(|b| b.is_ascii_digit()),
                "{text}"
            );
            assert_eq!(Code::parse(text), Some(code));
            assert!(seen.insert(text), "{text} is used twice");
        }
    }
}
