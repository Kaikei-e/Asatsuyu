# Asatsuyu Grammar Specification

## Status

Frozen as of Issue 88 (Phase 3-0). Changes to this grammar require a new issue.

---

## Keywords (17 keywords)

All keywords are lexed unconditionally as keyword tokens. They are classified into four categories (see `KeywordClass` in `asatsuyu-syntax/src/keyword.rs`).

### Hard keywords (10)

Always reserved. Cannot be used as identifiers in any context.

| Keyword  | Token       | Purpose                          |
|----------|-------------|----------------------------------|
| `fn`     | `FnKw`      | Function / lambda definition     |
| `pub`    | `PubKw`     | Visibility modifier              |
| `let`    | `LetKw`     | Variable binding                 |
| `type`   | `TypeKw`    | Type definition                  |
| `match`  | `MatchKw`   | Pattern matching                 |
| `if`     | `IfKw`      | Conditional expression           |
| `else`   | `ElseKw`    | Else branch                      |
| `import` | `ImportKw`  | Module import                    |
| `from`   | `FromKw`    | Python FFI import prefix         |
| `try`    | `TryKw`     | Exception-to-Result boundary     |

### Literal keywords (2)

Boolean literals that are keywords syntactically.

| Keyword  | Token       | Purpose                          |
|----------|-------------|----------------------------------|
| `True`   | `TrueKw`    | Boolean literal                  |
| `False`  | `FalseKw`   | Boolean literal                  |

### Contextual keywords (2)

Only meaningful in specific syntactic positions (`from python import ... as ...`).

| Keyword  | Token       | Purpose                          |
|----------|-------------|----------------------------------|
| `python` | `PythonKw`  | Python FFI marker                |
| `as`     | `AsKw`      | Import alias                     |

### Reserved keywords (3)

Reserved for future language features. Using them is an error today.

| Keyword  | Token       | Planned use                      |
|----------|-------------|----------------------------------|
| `mut`    | `MutKw`     | Scoped mutability (Phase 3-1)    |
| `async`  | `AsyncKw`   | Async functions (Phase 3-2)      |
| `await`  | `AwaitKw`   | Await expressions (Phase 3-2)    |

---

## Productions

```ebnf
SourceFile         = TopLevel*

TopLevel           = FnDef | TypeDef | ImportStmt | FromPythonImportStmt

FnDef              = Visibility? 'fn' IDENT ParamList ReturnType? BlockExpr
Visibility         = 'pub'
ParamList          = '(' (Param (',' Param)* ','?)? ')'
Param              = IDENT ':' TypeExpr
ReturnType         = '->' TypeExpr

TypeDef            = Visibility? 'type' IDENT TypeDefParams? '{' TypeBody '}'
TypeDefParams      = '(' IDENT (',' IDENT)* ','? ')'
TypeBody           = Field* | Variant*
Field              = IDENT ':' TypeExpr
Variant            = IDENT ('(' VarField (',' VarField)* ','? ')')?
VarField           = (IDENT ':')? TypeExpr

TypeExpr           = IDENT ('(' TypeExpr (',' TypeExpr)* ','? ')')?

ImportStmt         = 'import' Path ('as' IDENT)?
FromPythonImportStmt = 'from' 'python' 'import' IDENT ('as' IDENT)?
Path               = IDENT ('.' IDENT)*

BlockExpr          = '{' (LetStmt | Expr)* '}'
LetStmt            = 'let' IDENT '=' Expr

Expr               = PrefixExpr | InfixExpr | PostfixExpr | Atom
PrefixExpr         = ('-' | '!') Expr
InfixExpr          = Expr BinOp Expr
PostfixExpr        = Expr '(' ArgList ')' | Expr '.' IDENT
Atom               = LiteralExpr | IdentExpr | ParenExpr | IfExpr
                   | MatchExpr | LambdaExpr | TryExpr | ListExpr

LiteralExpr        = INT_LIT | FLOAT_LIT | STRING_LIT | 'True' | 'False'
IdentExpr          = IDENT
ParenExpr          = '(' Expr ')'
IfExpr             = 'if' Expr BlockExpr ('else' (IfExpr | BlockExpr))?
MatchExpr          = 'match' Expr '{' MatchArm* '}'
MatchArm           = Pattern Guard? '->' Expr
Guard              = 'if' Expr
LambdaExpr         = 'fn' LambdaParamList ReturnType? BlockExpr
LambdaParamList    = '(' (LambdaParam (',' LambdaParam)* ','?)? ')'
LambdaParam        = IDENT (':' TypeExpr)?
TryExpr            = 'try' Expr
ListExpr           = '[' (Expr (',' Expr)* ','?)? ']'
ArgList            = '(' (Expr (',' Expr)* ','?)? ')'

Pattern            = WildcardPat | LiteralPat | ConstructorPat | IdentPat | ListPat
WildcardPat        = '_'
LiteralPat         = INT_LIT | FLOAT_LIT | STRING_LIT | 'True' | 'False'
IdentPat           = IDENT                          (lowercase initial)
ConstructorPat     = IDENT ('(' Pattern (',' Pattern)* ','? ')')? (uppercase initial)
ListPat            = '[' (Pattern (',' Pattern)*)? ('..' IDENT?)? ']'
```

---

## Match Arm Notation

Match arms use `->` (Arrow). This is final.

```
match value {
  Some(n) if n > 0 -> "positive"
  Some(n) -> "non-positive"
  None -> "nothing"
}
```

`=>` (FatArrow) is not part of the Asatsuyu grammar.

---

## Start Sets

These are defined as `TokenSet` constants in `asatsuyu-parser/src/parser.rs`.

| Set                  | Tokens                                                                  |
|----------------------|-------------------------------------------------------------------------|
| `TOP_LEVEL_RECOVERY` | `fn` `pub` `type` `let` `import` `from`                                |
| `EXPR_START`         | `-` `!` `(` `if` `match` `fn` `try` `[` INT FLOAT STRING `True` `False` IDENT |
| `PATTERN_START`      | `_` INT FLOAT STRING `True` `False` IDENT `[`                          |
| `CLOSING_DELIMITERS` | `)` `}` `]`                                                            |

---

## Operator Precedence (low to high)

| Level | Operators                   | Associativity | Binding Power |
|-------|-----------------------------|---------------|---------------|
| 1     | `\|\|`                      | Left          | (1, 2)        |
| 2     | `&&`                        | Left          | (3, 4)        |
| 3     | `==` `!=` `<` `<=` `>` `>=` | Left          | (5, 6)        |
| 4     | `\|>` `+` `-` `<>`         | Left          | (7, 8)        |
| 5     | `*` `/` `%`                 | Left          | (9, 10)       |
| 6     | `-` `!` (prefix)            | Right         | (_, 11)       |
| 7     | `f()` (call)                | Left          | (13, _)       |
| 8     | `.field` (access)           | Left          | (15, _)       |

---

## Tokens

### Operators

| Token | Symbol |
|-------|--------|
| Plus  | `+`    |
| Minus | `-`    |
| Star  | `*`    |
| Slash | `/`    |
| Percent | `%`  |
| Eq    | `=`    |
| EqEq  | `==`   |
| BangEq | `!=`  |
| Lt    | `<`    |
| LtEq  | `<=`   |
| Gt    | `>`    |
| GtEq  | `>=`   |
| Bang  | `!`    |
| Ampersand | `&` |
| PipeSingle | `\|` |
| AmpAmp | `&&`  |
| PipePipe | `\|\|` |
| Pipe  | `\|>`  |
| StringConcat | `<>` |

### Delimiters and Punctuation

| Token     | Symbol |
|-----------|--------|
| LParen    | `(`    |
| RParen    | `)`    |
| LBrace    | `{`    |
| RBrace    | `}`    |
| LBracket  | `[`    |
| RBracket  | `]`    |
| Comma     | `,`    |
| Colon     | `:`    |
| Semicolon | `;`    |
| Dot       | `.`    |
| DotDot    | `..`   |
| Arrow     | `->`   |
| Underscore | `_`   |

### Trivia

| Token      | Description      |
|------------|------------------|
| Whitespace | Spaces, tabs     |
| Newline    | Line breaks      |
| Comment    | `// ...`         |
