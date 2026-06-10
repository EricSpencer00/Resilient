---
title: Language Reference
nav_order: 4
has_children: true
permalink: /language-reference
---

# Resilient Language Reference
{: .no_toc }

A formal specification of Resilient's lexical grammar, type system, and
evaluation semantics — written for tool authors, static analysis
developers, and safety auditors. For a tutorial-oriented syntax guide
with worked examples, see [Syntax Reference](syntax).
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Scope and conventions

This document specifies the surface language as implemented by the
reference compiler in `resilient/src/`. Where the informal
[SYNTAX.md](https://github.com/EricSpencer00/Resilient/blob/main/SYNTAX.md)
presents usage-oriented descriptions and examples, this reference
presents the grammar in EBNF, the type system as inference rules, and
runtime behaviour in terms of the operational error variants the VM
surfaces. The two documents are complementary; this one is
authoritative for questions of the form *"is X syntactically legal?"*,
*"what type does expression E have in context Γ?"*, and *"what
runtime errors can operation O produce?"*.

### EBNF notation

Grammar rules use a standard EBNF dialect:

- `::=` — production
- `|`   — alternation
- `[ x ]` — optional (zero or one)
- `{ x }` — repetition (zero or more)
- `( x )` — grouping
- `"x"`  — terminal literal (source text)
- `x*`  — shorthand for `{ x }`
- `x+`  — shorthand for `x { x }`
- Identifiers in `UpperCamel` name non-terminals; lowercase names name
  character classes.

### Source encoding

Source files are UTF-8. The lexer scans by Unicode scalar value
(`char`), but identifier bodies are constrained to ASCII (see
[Identifiers](#identifiers)). Newlines are `\n` (LF); `\r\n` sequences
appear as whitespace. A leading `#!...\n` shebang line is silently
skipped so programs can be made executable.

---

## 1. Lexical grammar

### Whitespace and comments

```ebnf
Whitespace ::= (" " | "\t" | "\n" | "\r")+
LineComment ::= "//" { any-char-except-newline } ("\n" | eof)
BlockComment ::= "/*" { any-char | BlockComment-body } "*/"
```

Block comments **do not nest**; the first `*/` terminates the comment.
An unterminated block comment is a lexical error.

### Identifiers

```ebnf
Identifier ::= (ascii-letter | "_") (ascii-letter | ascii-digit | "_")*

ascii-letter ::= "A" ... "Z" | "a" ... "z"
ascii-digit  ::= "0" ... "9"
```

Identifiers are **ASCII-only** by design. Non-ASCII letters (Cyrillic,
Greek, accented Latin, CJK, etc.) in identifier position are a lexical
error:

```
1:1: identifier contains non-ASCII character 'ф' — Resilient
identifiers are ASCII-only (see SYNTAX.md)
```

The restriction is a homoglyph safety property: two identifiers that
render identically must also compare identical by code point. String
literals, comments, and file contents retain full UTF-8 — only
identifier scanning is tightened.

### Keywords and reserved words

Keywords cannot appear as identifiers. The complete set:

```
fn       let      live     assert   assume   if       else
return   static   while    for      in       requires
ensures  invariant struct  new      match    use
impl     type     default  true     false    _
```

`default` is a reserved alias for `_` in match-arm position
(see [§3, Match expressions](#expression-grammar)); it is otherwise
illegal where an identifier is expected.

### Integer literals

```ebnf
IntLit      ::= DecIntLit | HexIntLit | BinIntLit
DecIntLit   ::= ascii-digit (ascii-digit | "_")*
HexIntLit   ::= "0" ("x" | "X") hex-digit (hex-digit | "_")*
BinIntLit   ::= "0" ("b" | "B") bin-digit (bin-digit | "_")*
hex-digit   ::= ascii-digit | "a" ... "f" | "A" ... "F"
bin-digit   ::= "0" | "1"
```

- The underscore separator is purely visual; `1_000_000` and `1000000`
  tokenize identically.
- A bare `0x` or `0b` with no following digit tokenizes to the integer
  `0`; callers that care about well-formedness should forbid empty
  radix bodies at the grammar level.
- All integer literals have static type `int` (i64). Values outside
  the `i64` range overflow at lex time to `0`; real programs should
  assume this is a follow-up and not rely on the fallback.

### Float literals

```ebnf
FloatLit ::= ascii-digit+ "." ascii-digit+
```

A float literal **must** have at least one digit on both sides of the
decimal point. `1.` and `.5` are not valid float literals: `1.` scans
as the integer `1` followed by a `.` (field-access token), and `.5`
scans as a `.` followed by `5`. There is no exponent syntax
(`1.5e3`), no hex-float syntax, and no suffix form. All floats have
static type `float` (f64, IEEE-754 binary64).

### String literals

```ebnf
StringLit   ::= "\"" { StringChar } "\""
StringChar  ::= any-char-except-quote-and-backslash
              | "\\" EscapeChar
EscapeChar  ::= "n" | "t" | "r" | "\\" | "\""
```

Supported escapes: `\n`, `\t`, `\r`, `\\`, `\"`. Unknown escapes pass
through as the literal two characters `\` + the following character
(lenient recovery — the lexer never halts on a bad escape). There
are no `\x`, `\u{...}`, octal, or continuation escapes in string
literals; use a `bytes` literal for binary data.

### Bytes literals

```ebnf
BytesLit    ::= "b\"" { BytesByte } "\""
BytesByte   ::= any-ascii-byte-except-quote-and-backslash
              | "\\" BytesEscape
BytesEscape ::= "n" | "t" | "r" | "0" | "\\" | "\""
              | "x" hex-digit hex-digit
```

Bytes literals produce a raw `bytes` value. Non-ASCII source bytes
inside a `b"..."` literal are encoded as their UTF-8 bytes (the
lexer does not reject them, but the idiomatic form is `\xNN` for
anything non-printable). Unicode escapes (`\u{...}`) are deliberately
**not** honoured at the bytes level.

### Boolean literals

```ebnf
BoolLit ::= "true" | "false"
```

### Duration literals

Duration literals appear **only** inside the `within` clause of a
`live` block. They are not a general time-library surface.

```ebnf
DurationLit ::= DecIntLit DurationUnit
DurationUnit ::= "ns" | "us" | "ms" | "s"
```

### Operators and punctuation

Complete terminal set:

```
Arithmetic     +  -  *  /  %
Comparison     ==  !=  <  >  <=  >=
Logical        &&  ||  !
Bitwise        &  |  ^  <<  >>
Assignment     =
Arrow          ->   =>
Delimiters     (  )  {  }  [  ]  #{
Separators     ,  ;  :  .
Attribute      @
Other          ?  _
```

The composite token `#{` opens a set literal; the closing brace is an
ordinary `}`. The attribute prefix `@` introduces function
annotations (currently only `@pure`).

### Operator precedence and associativity

Precedence levels, from lowest (1) to highest (10). All binary
operators are left-associative; prefix operators are
right-associative.

| Level | Operator(s)                      | Associativity |
|:-----:|:---------------------------------|:--------------|
| 1     | `\|\|`                           | left          |
| 2     | `&&`                             | left          |
| 3     | `\|` (bitwise-or)                | left          |
| 4     | `^` (bitwise-xor)                | left          |
| 5     | `&` (bitwise-and)                | left          |
| 6     | `==` `!=`                        | left          |
| 7     | `<` `>` `<=` `>=`                | left          |
| 8     | `<<` `>>`                        | left          |
| 9     | `+` `-`                          | left          |
| 10    | `*` `/` `%`                      | left          |
| 11    | unary `-` `!`                    | right         |
| 12    | call `f(...)`, index `a[i]`, field `s.f` | left  |

Call, index, and field-access operators are postfix; they bind tighter
than any prefix operator.

---

## 2. Type system

### Type syntax

```ebnf
Type ::= "int"
       | "float"
       | "string"
       | "bool"
       | "bytes"
       | "void"
       | "any"
       | ArrayType
       | FixedArrayType
       | FunctionType
       | "Result" [ "<" Type ">" ]
       | Identifier                     (* struct name or alias *)
ArrayType      ::= "[" Type "]"
FixedArrayType ::= "[" Type ";" IntLit "]"
FunctionType   ::= "fn" "(" [ Type { "," Type } ] ")" "->" Type
```

### Type universe

```
T ::= int | float | string | bool | bytes | void | any
    | [T]                    -- dynamic array, element T
    | [T; N]                 -- fixed-length array, element T, length N
    | fn(T1,...,Tn) -> T     -- function type
    | Result<T>              -- fallible computation carrying T
    | struct Name            -- nominal record
    | ?α                     -- inference variable (internal)
```

Semantics:

- `int` is 64-bit two's-complement signed (`i64`); overflow in
  arithmetic traps via Rust's checked-arithmetic layer in the
  verifier and saturates / wraps in the interpreter depending on the
  operation.
- `float` is IEEE-754 binary64 (`f64`). NaN and infinities are
  representable; `to_int` rejects them.
- `string` is an owned UTF-8 sequence. `len(s)` returns the **Unicode
  scalar** count, not the byte length.
- `bytes` is a raw byte sequence, distinct from `string`. No implicit
  conversion between the two.
- `bool` is `true` or `false`.
- `void` is the type of expressions with no value (function bodies
  that omit `return`, `println` calls, etc.). It has no literal form
  and cannot appear as a value in user code.
- `any` is a dynamic-type escape hatch used by the typechecker during
  inference and by built-in signatures that accept heterogeneous
  arguments. User code never writes `any` directly; the `compatible`
  relation treats it as unifiable with every type. Values typed `any`
  defer all enforcement to runtime.

### Compatibility relation

Two types `T1` and `T2` are *compatible* (written `T1 ~ T2`) iff:

```
T1 ~ T2  ⟺  T1 = T2  ∨  T1 = any  ∨  T2 = any
```

Compatibility is used at argument passing, assignment, and
return-type checking. It is **not** transitive when `any` is
involved (it is strictly not an equivalence relation); the
typechecker uses it as a "don't reject, defer" signal.

### Numeric coercion rules

Resilient performs **no implicit numeric coercion**. Every
arithmetic and comparison operator `⊕ ∈ {+, -, *, /, %, ==, !=, <,
>, <=, >=}` requires both operands to share a numeric type:

```
Γ ⊢ e1 : int     Γ ⊢ e2 : int
────────────────────────────────  (T-ArithInt)
      Γ ⊢ e1 ⊕ e2 : int

Γ ⊢ e1 : float   Γ ⊢ e2 : float
────────────────────────────────  (T-ArithFloat)
      Γ ⊢ e1 ⊕ e2 : float
```

Mixing `int` and `float` is a static error. Users bridge explicitly:

| Signature                 | Semantics                                          |
|:--------------------------|:---------------------------------------------------|
| `to_float(int) -> float`  | exact widening (faithful for \|x\| < 2<sup>53</sup>) |
| `to_int(float) -> int`    | truncate toward zero; NaN / ±∞ / out-of-range → runtime error |

Comparison and equality operators share the same same-numeric-type
rule and produce `bool`.

### String concatenation coercion

The `+` operator on strings is overloaded: if either operand is
`string`, the other operand may be `int`, `float`, `bool`, or
`string`. The non-string operand is rendered via its `Display` form
and concatenated. This is the **only** implicit conversion in the
language.

```
Γ ⊢ e1 : string   Γ ⊢ e2 : T    T ∈ {string, int, float, bool}
────────────────────────────────────────────────────────────────  (T-Concat-L)
                    Γ ⊢ e1 + e2 : string
```

Symmetric rule `T-Concat-R` with sides swapped.

### Type aliases

```ebnf
TypeAliasDecl ::= "type" Identifier "=" Type ";"
```

Aliases are **structural**, not nominal. `type Meters = int;` makes
`Meters` and `int` interchangeable at every use site — assignment,
argument passing, return types, arithmetic. A value of type `Meters`
flows into an `int` parameter without a cast.

Cycles (`type A = B; type B = A;`) are a static error:

```
type alias cycle: A -> B -> A
```

For a **nominal** distinct-from-int type, wrap in a one-field struct:

```rust
struct Meters { int val, }
```

Alias declarations hoist within a file — forward references work
because the typechecker collects aliases in its first pass.

### Struct types

```ebnf
StructDecl  ::= "struct" Identifier "{" { StructField } "}"
StructField ::= Type Identifier ","
```

Structs are **nominal**: two structs with identical field shape but
different names do not unify. Fields are stored in declaration order
(preserved across `Display` and equality).

### `any` and safety

The `any` type is an escape hatch, not a user-facing construct. It
arises only in:

1. Built-in signatures where argument types are heterogeneous
   (`println`, `abs`, `min`, `max`, the `map_*` family).
2. Inference intermediate states before a concrete type is known.
3. Array element types (MVP — typed arrays are a planned follow-up).

**Safety implications**: any expression typed `any` bypasses static
type checking at that position. Runtime errors on mistyped `any`
values surface through `VmError::TypeMismatch`. Static analysis tools
should treat `any` as an unknown-type node and, if precise type
information is required, reject the program or fall back to runtime
instrumentation.

### Function types

```ebnf
FunctionType ::= "fn" "(" [ Type { "," Type } ] ")" "->" Type
```

A function with an omitted return-type annotation is inferred from
the body. An omitted-return body infers `void`. Parameter types are
always required.

---

## 3. Expression grammar

Expression productions are stratified by precedence level. Each level
delegates to the next-higher level for sub-expressions.

```ebnf
Expression     ::= OrExpr
OrExpr         ::= AndExpr  { "||" AndExpr }
AndExpr        ::= BitOrExpr { "&&" BitOrExpr }
BitOrExpr      ::= BitXorExpr { "|" BitXorExpr }
BitXorExpr     ::= BitAndExpr { "^" BitAndExpr }
BitAndExpr     ::= EqExpr { "&" EqExpr }
EqExpr         ::= CmpExpr { ("==" | "!=") CmpExpr }
CmpExpr        ::= ShiftExpr { ("<" | ">" | "<=" | ">=") ShiftExpr }
ShiftExpr      ::= AddExpr { ("<<" | ">>") AddExpr }
AddExpr        ::= MulExpr { ("+" | "-") MulExpr }
MulExpr        ::= UnaryExpr { ("*" | "/" | "%") UnaryExpr }
UnaryExpr      ::= ("-" | "!") UnaryExpr
                 | PostfixExpr
PostfixExpr    ::= PrimaryExpr { PostfixOp }
PostfixOp      ::= "(" [ ArgList ] ")"                -- call
                 | "[" Expression [ ".." Expression ] "]"   -- index / slice
                 | "." Identifier                     -- field access
ArgList        ::= Expression { "," Expression }

PrimaryExpr    ::= IntLit | FloatLit | StringLit | BytesLit | BoolLit
                 | Identifier
                 | "(" Expression ")"
                 | ArrayLit
                 | SetLit
                 | StructLit
                 | IfExpr
                 | MatchExpr
                 | LiveBlock
                 | Block
ArrayLit       ::= "[" [ Expression { "," Expression } [","] ] "]"
SetLit         ::= "#{" [ Expression { "," Expression } [","] ] "}"
StructLit      ::= "new" Identifier "{" [ FieldInit { "," FieldInit } [","] ] "}"
FieldInit      ::= Identifier [ ":" Expression ]       -- shorthand allowed
Block          ::= "{" { Statement } [ Expression ] "}"
IfExpr         ::= "if" Expression Block [ "else" (IfExpr | Block) ]
MatchExpr      ::= "match" Expression "{" MatchArm { "," MatchArm } [","] "}"
MatchArm       ::= Pattern [ "if" Expression ] "=>" Expression
Pattern        ::= OrPattern
OrPattern      ::= SubPattern { "|" SubPattern }
SubPattern     ::= BindPattern | Literal | Identifier | "_" | "default"
BindPattern    ::= Identifier "@" SubPattern
```

### `if` as expression

`if` is an expression when all branches produce a value of the same
type; otherwise it is a statement producing `void`. The type rule:

```
Γ ⊢ c : bool    Γ ⊢ e1 : T    Γ ⊢ e2 : T
─────────────────────────────────────────   (T-If-Expr)
  Γ ⊢ (if c { e1 } else { e2 }) : T
```

An `if` without an `else` branch, or with mismatched branch types,
has type `void`.

### `match` expressions

Type rule (simplified):

```
Γ ⊢ e : S
for each arm (pᵢ ⇒ bodyᵢ) with optional guard gᵢ:
    Γ, bindings(pᵢ) ⊢ gᵢ : bool         (if present)
    Γ, bindings(pᵢ) ⊢ bodyᵢ : T
arms are exhaustive over S
──────────────────────────────────────────────────   (T-Match)
              Γ ⊢ match e { ... } : T
```

**Exhaustiveness**: guarded arms do not count toward exhaustiveness.
A match with only guarded arms requires an unguarded catch-all (`_`,
`default`, or a bare identifier pattern) or full literal coverage of
a finite type (e.g. both `true` and `false` for `bool`).

**Or-patterns**: every branch of an or-pattern must bind the same
set of names; otherwise `or-pattern branches bind different names`
is reported at typecheck.

### Live blocks

```ebnf
LiveBlock     ::= "live" [ BackoffClause ] [ WithinClause ]
                          [ BackoffClause ] Block
BackoffClause ::= "backoff" "(" BackoffKwarg { "," BackoffKwarg } ")"
BackoffKwarg  ::= ("base_ms" | "factor" | "max_ms") "=" IntLit
WithinClause  ::= "within" DurationLit
```

A `live` block is an expression of type `void`. The order of
`backoff` and `within` clauses is free; each clause may appear at
most once. Semantics are specified in
[§7, Error model](#error-model).

### Array indexing and slicing

```ebnf
IndexOp ::= "[" Expression "]"
SliceOp ::= "[" Expression ".." Expression "]"
```

Indexing: `a[i]` produces the element type when `a : [T]`. Runtime
bounds check: out-of-range indices raise `ArrayIndexOutOfBounds`
([E0009](errors/E0009)).

Slicing: `a[i..j]` produces `[T]`. The range is half-open (`i`
inclusive, `j` exclusive); an inverted or out-of-bounds range raises
a runtime error.

### Field access

`s.f` requires `s : struct Name` and `f` declared in `Name`. The
result type is the field's declared type. Field assignment is a
statement (see [§4](#statement-grammar)).

---

## 4. Statement grammar

```ebnf
Statement        ::= LetStmt
                   | StaticLetStmt
                   | AssignStmt
                   | ReturnStmt
                   | WhileStmt
                   | ForStmt
                   | ExprStmt
                   | FnDecl
                   | StructDecl
                   | TypeAliasDecl
                   | ImplDecl
                   | UseDecl
                   | AssertStmt
                   | ";"                        -- empty

LetStmt          ::= "let" (LetPattern | Identifier [":" Type]) "=" Expression ";"
LetPattern       ::= Identifier "{" FieldPat { "," FieldPat } [ "," ".." ] "}"
FieldPat         ::= Identifier [ ":" Identifier ]
StaticLetStmt    ::= "static" "let" Identifier "=" Expression ";"
AssignStmt       ::= LValue "=" Expression ";"
LValue           ::= Identifier
                   | LValue "." Identifier
                   | LValue "[" Expression "]"
ReturnStmt       ::= "return" [ Expression ] ";"
WhileStmt        ::= "while" Expression Block
ForStmt          ::= "for" Identifier "in" Expression Block
ExprStmt         ::= Expression ";"
AssertStmt       ::= "assert" "(" Expression [ "," Expression ] ")" ";"

UseDecl          ::= "use" StringLit ";"
FnDecl           ::= [ "@" Identifier ] "fn" [ TypeParams ]
                     Identifier "(" [ ParamList ] ")"
                     [ "->" Type ]
                     { Contract }
                     Block
TypeParams       ::= "<" Identifier { "," Identifier } ">"
ParamList        ::= Param { "," Param }
Param            ::= Type Identifier
Contract         ::= "requires" Expression
                   | "ensures"  Expression
                   | "invariant" Expression
ImplDecl         ::= "impl" Identifier "{" { FnDecl } "}"
```

### `let` semantics

```
Γ ⊢ e : T       x ∉ dom(Γ_current_scope)
─────────────────────────────────────────   (T-Let)
       Γ, x:T ⊢ let x = e; : void
```

- `let` creates a new binding in the current scope. Shadowing a name
  in an enclosing scope is permitted; re-binding within the same
  scope is a type error.
- Subsequent `x = expr;` in the same or a child scope reassigns the
  binding. The assigned expression must be compatible with the
  binding's declared or inferred type.
- The optional `:T` annotation is structurally checked against the
  RHS.

### `static let` semantics

A `static let` binding inside a function body persists across
invocations of that function. Its initializer is evaluated exactly
once — on the first call that reaches the declaration. Subsequent
calls observe the value left by the previous call.

### `use` semantics

`use "path/to/file.rz";` is a textual splice performed by the
importer *before* parsing of the importing file completes — the
imported file's top-level declarations become part of the importing
file. Imports are resolved relative to the importing file's
directory. There is no module namespace, no re-export control, and
no symbol visibility modifier; a `use` makes everything in the
imported file available by its original name.

### `for` loops

```
Γ ⊢ e : [T]
Γ, x:T ⊢ body : void
────────────────────────────────────   (T-For-Array)
   Γ ⊢ for x in e { body } : void
```

Iteration over a `Value::Array` binds `x` to successive elements in
index order. Iteration over a set or map is a runtime concern; the
typechecker accepts `any` at the collection position.

### `while` loops

```
Γ ⊢ c : bool    Γ ⊢ body : void
───────────────────────────────────   (T-While)
   Γ ⊢ while c { body } : void
```

While-loops have a built-in **runaway guard**: after 1,000,000
iterations of a single `while` instance, the interpreter raises:

```
while loop exceeded 1000000 iterations (runaway?)
```

This guard exists to preserve the progress property on safety-
critical targets; it is not a user-tunable.

---

## 5. Contract clauses

```ebnf
Contract ::= "requires" Expression
           | "ensures"  Expression
           | "invariant" Expression
```

### `requires` — preconditions

A `requires e;` clause attached to a function is checked **before**
the function body executes. The clause is an arbitrary `bool`
expression in the function's parameter scope.

```
for fn f(T1 p1, ..., Tn pn) requires r1 ... requires rk :
  at each call site f(a1, ..., an):
    evaluate ri[a1/p1, ..., an/pn] in the caller's scope
    if any ri = false, raise a contract violation
```

### `ensures` — postconditions

An `ensures e;` clause is checked **after** the function returns.
Inside `e`, the special identifier `result` is bound to the return
value. The clause may also reference parameters (their values at
function entry).

```
at function exit with return value v:
  evaluate ei with { pᵢ = caller's argᵢ, result = v }
  if any ei = false, raise a contract violation
```

### `invariant` — live-block invariants

An `invariant e;` clause attached to a `live { }` block is checked
**after every iteration** of the block's body. A failed invariant
triggers the same retry path as a body-level error (see
[§7](#error-model)).

### Static discharge (Z3)

When built with `--features z3` and invoked with `--audit`, the
typechecker passes each `requires` clause with known argument values
to the Z3 SMT solver. Outcomes:

- **Unsat** — negation of the clause is unsatisfiable ⇒ the clause
  is **proven** and the runtime check is elided.
- **Sat** — a counter-example exists ⇒ the clause is reported as a
  static contract violation with the counter-example attached.
- **Unknown** (timeout) — the clause is deferred to runtime.

The per-query timeout is `--verifier-timeout-ms N` (default 5000).
Without `--features z3`, every clause is deferred to runtime.

---

## 6. Built-in functions

All built-ins are bound in the global environment at startup. User
functions may shadow built-in names in an inner scope but cannot
redefine a built-in at the top level without a name collision
error.

### I/O

| Name        | Signature           | Errors       | Notes                                  |
|:------------|:--------------------|:-------------|:---------------------------------------|
| `println(x)`| `any -> void`       | none         | prints `x` + `"\n"`; strings print unquoted |
| `print(x)`  | `any -> void`       | none         | no trailing newline; stdout flushed    |
| `input(s)`  | `string -> string`  | I/O error → halt | prompts with `s`, reads one line from stdin (std-only) |

### Math

| Name      | Signature              | Errors | Notes |
|:----------|:-----------------------|:-------|:------|
| `abs(x)`  | `int -> int` / `float -> float` | overflow on `i64::MIN` | single-arg |
| `min(a,b)`| `(T,T) -> T` for T ∈ {int,float} | type mismatch | |
| `max(a,b)`| `(T,T) -> T` for T ∈ {int,float} | type mismatch | |
| `sqrt(x)` | `float -> float` / `int -> float` | `NaN` on negative input | |
| `pow(a,b)`| `(int,int) -> int` / `(float,float) -> float` | `int` overflow → saturate | |
| `floor(x)`| `float -> float`       | —      | toward −∞ |
| `ceil(x)` | `float -> float`       | —      | toward +∞ |
| `sin(x)`  | `float -> float`       | —      | radians |
| `cos(x)`  | `float -> float`       | —      | radians |
| `tan(x)`  | `float -> float`       | —      | radians |
| `ln(x)`   | `float -> float`       | `NaN` on `x ≤ 0` | natural log |
| `log(b,x)`| `(float,float) -> float` | `NaN` on ill-defined inputs | base-`b` log |
| `exp(x)`  | `float -> float`       | —      | eˣ |

### Numeric conversion

| Name          | Signature          | Errors |
|:--------------|:-------------------|:-------|
| `to_float(x)` | `int -> float`     | — (exact for \|x\| < 2<sup>53</sup>) |
| `to_int(x)`   | `float -> int`     | runtime error on NaN, ±∞, or out-of-i64-range |

### Arrays

| Name          | Signature                      | Errors                          |
|:--------------|:-------------------------------|:--------------------------------|
| `len(a)`      | `any -> int`                   | — (Unicode scalar count for strings, element count for arrays / bytes / maps / sets) |
| `push(a, x)`  | `([T], T) -> [T]`              | — (returns new array) |
| `pop(a)`      | `[T] -> [T]`                   | — on empty, returns empty array |
| `slice(a,i,j)`| `([T], int, int) -> [T]`       | bounds → runtime error          |

### Strings

| Name              | Signature                          | Errors |
|:------------------|:-----------------------------------|:-------|
| `split(s, sep)`   | `(string, string) -> [string]`     | — |
| `trim(s)`         | `string -> string`                 | — |
| `contains(s, sub)`| `(string, string) -> bool`         | — |
| `to_upper(s)`     | `string -> string`                 | — |
| `to_lower(s)`     | `string -> string`                 | — |
| `replace(s,a,b)`  | `(string, string, string) -> string` | — |
| `format(tpl, args)` | `(string, [any]) -> string`      | arity / index mismatch → runtime error |

### Bytes

| Name                | Signature                  | Errors |
|:--------------------|:---------------------------|:-------|
| `bytes_len(b)`      | `bytes -> int`             | — |
| `bytes_slice(b,i,j)`| `(bytes, int, int) -> bytes` | bounds → runtime error |
| `byte_at(b, i)`     | `(bytes, int) -> int`      | bounds → runtime error |

### Result

| Name           | Signature               | Errors           |
|:---------------|:------------------------|:-----------------|
| `Ok(x)`        | `T -> Result<T>`        | — |
| `Err(x)`       | `T -> Result<T>`        | — |
| `is_ok(r)`     | `Result -> bool`        | — |
| `is_err(r)`    | `Result -> bool`        | — |
| `unwrap(r)`    | `Result -> any`         | halts on `Err`   |
| `unwrap_err(r)`| `Result -> any`         | halts on `Ok`    |

### Randomness

| Name                | Signature                | Errors | Notes |
|:--------------------|:-------------------------|:-------|:------|
| `random_int(lo, hi)`| `(int, int) -> int`      | `lo >= hi` → runtime error | uniform on `[lo, hi)` |
| `random_float()`    | `() -> float`            | —      | uniform on `[0, 1)` |

Both draw from a global SplitMix64 stream. The seed is either
`--seed <u64>` from the CLI (deterministic) or derived from the
wall clock (reported to stderr so the user can pin the next run).

### Time / clock

| Name        | Signature     | Errors | Notes |
|:------------|:--------------|:-------|:------|
| `clock_ms()`| `() -> int`   | —      | monotonic ms since an unspecified epoch; std-only |
| `clock_now()`| `() -> int`  | —      | current Unix timestamp in seconds; std-only |
| `clock_elapsed(start)`| `(int) -> int` | — | elapsed milliseconds since `start`; std-only |

### Date / time

| Name                | Signature                  | Errors | Notes |
|:--------------------|:---------------------------|:-------|:------|
| `datetime_now()`    | `() -> DateTime`           | system clock before Unix epoch → runtime error | current system time as a `DateTime`; std-only |
| `datetime_from_unix(secs)` | `(int) -> DateTime` | — | convert Unix seconds into `DateTime`; std-only |
| `datetime_to_unix(dt)` | `(DateTime) -> int`      | shape mismatch → runtime error | convert `DateTime` back to Unix seconds; std-only |
| `datetime_format(dt, fmt)` | `(DateTime, string) -> string` | unknown `%` specifier → runtime error | supports `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%%`; std-only |
| `datetime_parse(s, fmt)` | `(string, string) -> Result<DateTime>` | parse failure → `Err(message)` | parses the same format codes accepted by `datetime_format`; std-only |

### Files (std-only)

| Name                | Signature                | Errors              |
|:--------------------|:-------------------------|:--------------------|
| `file_read(path)`   | `string -> string`       | I/O error → halt    |
| `file_write(path,c)`| `(string, string) -> void` | I/O error → halt  |
| `env(name)`         | `string -> Result<string>` | — (absence is `Err`) |

### Maps and sets

| Name              | Signature                           | Errors |
|:------------------|:------------------------------------|:-------|
| `map_new()`       | `() -> map`                         | — |
| `map_insert(m,k,v)` | `(map, K, V) -> map`              | K must be `int`/`string`/`bool` |
| `map_get(m,k)`    | `(map, K) -> Result<V>`             | — (absence is `Err`) |
| `map_remove(m,k)` | `(map, K) -> map`                   | — |
| `map_keys(m)`     | `map -> [K]`                        | — |
| `map_len(m)`      | `map -> int`                        | — |
| `set_new()`       | `() -> set`                         | — |
| `set_insert(s,x)` | `(set, T) -> set`                   | T must be `int`/`string`/`bool` |
| `set_remove(s,x)` | `(set, T) -> set`                   | — |
| `set_has(s,x)`    | `(set, T) -> bool`                  | — |
| `set_len(s)`      | `set -> int`                        | — |
| `set_items(s)`    | `set -> [T]`                        | — (order unspecified) |

### Diagnostics

| Name                        | Signature     | Errors | Notes                     |
|:----------------------------|:--------------|:-------|:--------------------------|
| `assert(cond, msg?)`        | `(bool, string?) -> void` | halts on `cond = false` | second arg optional |
| `assume(cond, msg?)`        | `(bool, string?) -> void` | halts on `cond = false` | verifier treats as axiom; see [Syntax → Runtime assumptions](syntax#runtime-assumptions) |
| `live_retries()`            | `() -> int`   | —      | current retry count of innermost `live` block |
| `live_total_retries()`      | `() -> int`   | —      | process-wide live retry counter |
| `live_total_exhaustions()`  | `() -> int`   | —      | process-wide live exhaustion counter |

---

## 7. Error model

### Runtime error variants

The VM surfaces runtime faults as `VmError` variants. The
interpreter reports an equivalent set through its diagnostics
pipeline. Variants (source: `resilient/src/vm.rs`):

| Variant                     | Trigger                                                       |
|:----------------------------|:--------------------------------------------------------------|
| `EmptyStack`                | operand stack underflow (compiler bug, not user-reachable)    |
| `DivideByZero`              | `/` or `%` with RHS = 0 (int); emitted as [E0008](errors/E0008) |
| `TypeMismatch(what)`        | operator applied to wrong value types at runtime              |
| `LocalOutOfBounds(i)`       | local-slot index invalid (compiler bug)                       |
| `ConstantOutOfBounds(i)`    | constant-pool index invalid (compiler bug)                    |
| `FunctionOutOfBounds(i)`    | call target invalid (compiler bug)                            |
| `CallStackUnderflow`        | `return` at top level (compiler bug)                          |
| `CallStackOverflow`         | call depth exceeds 1024 frames (runaway recursion guard)      |
| `JumpOutOfBounds`           | control-flow target invalid (compiler bug)                    |
| `Unsupported(opcode)`       | opcode reserved but not yet implemented                       |
| `ArrayIndexOutOfBounds`     | index out of range; emitted as [E0009](errors/E0009)          |
| `AtLine { line, kind }`     | wrapper carrying source line of the failing op                |

Compiler-bug variants are defensive checks against malformed
bytecode; user programs cannot cause them without a compiler
defect. The user-visible variants are `DivideByZero`,
`ArrayIndexOutOfBounds`, `TypeMismatch`, and `CallStackOverflow`.

Additionally, contract violations, assertion failures, and
`unwrap`/`unwrap_err` on the wrong variant halt execution with a
diagnostic. These surface through the interpreter rather than
`VmError`.

### Interaction with `live { }` blocks

A `live` block supervises its body. A fault raised inside the body
is classified as **recoverable** or **fatal**:

**Recoverable** (trigger retry):
- `assert(false, ...)` — assertion failure
- contract violations (`requires` / `ensures`)
- failed `invariant` clauses
- runtime errors from built-ins (I/O failures, etc.)
- `DivideByZero`, `ArrayIndexOutOfBounds`, `TypeMismatch`
- `unwrap` on an `Err`, `unwrap_err` on an `Ok`

**Fatal** (escape the block, terminate the program):
- `CallStackOverflow` (runaway recursion)
- exhausting the `while`-loop 1,000,000-iteration guard
- the live block itself exceeds `MAX_RETRIES = 3`

On a recoverable fault, the runtime:

1. Rolls back the block's local state to the last-known-good
   snapshot (captured at block entry and after each successful
   iteration of the body).
2. Increments the retry counter.
3. If `backoff(...)` is set, sleeps `min(max_ms, base_ms *
   factor^retries)` ms.
4. If `within <duration>` is set, checks wall-clock budget; if
   exceeded, escalates as a *timeout* (distinct prefix in
   diagnostics).
5. Re-executes the body from the top.

When a `live` block exhausts its own budget (3 attempts by default),
it raises a `Live block failed after N attempts` error. If that
block is nested inside another `live`, the outer block treats the
failure as one recoverable fault and may itself retry. Retry
budgets at each nesting level are independent — with defaults, two
nested `live` blocks run the inner body up to 3 × 3 = 9 times.

### Propagation without a `live` block

A fault raised outside any `live` block terminates the program with
a formatted diagnostic of the form:

```
<file>:<line>:<col>: <category>: <message>
```

and a non-zero exit code. No default retry, no unwind past `main`.

### Result type

`Result<T>` is the explicit error-propagation alternative to
supervised retries. `Ok(x)` and `Err(x)` construct the two variants;
`is_ok`, `is_err`, `unwrap`, `unwrap_err` inspect them. A program
that wants to model recoverable domain errors without the live-block
machinery uses `Result` end-to-end.

---

## 8. Evaluation order and guarantees

### Argument evaluation order

Function arguments are evaluated **left to right**, fully, before the
callee is entered. There is no argument lazy evaluation and no
compile-time argument reordering.

### Short-circuit evaluation

`&&` and `||` short-circuit:

- `e1 && e2` evaluates `e1`; if `e1` is `false`, `e2` is not
  evaluated and the result is `false`.
- `e1 || e2` evaluates `e1`; if `e1` is `true`, `e2` is not
  evaluated and the result is `true`.

All other operators (including `&`, `|`, `^`) evaluate both operands.

### Evaluation of struct literals and array literals

Fields and elements are evaluated in source order, left to right.

### Determinism

Given the same input source, the same command-line flags, and the
same `--seed <N>` value, program output is **bit-identical** across
runs. Specifically:

- `random_int` / `random_float` are deterministic under `--seed`.
- Map and set iteration orders are unspecified and may vary across
  runs unless the user sorts explicitly; on std builds, `HashMap`
  and `HashSet` provide no order guarantee. (The no_std runtime
  uses sorted containers; programs that need ordered iteration
  across targets should sort at the API boundary.)
- `clock_ms`, `clock_now`, and `datetime_now` are non-determinism
  sources; programs requiring reproducibility must avoid reading them
  or must record them into a trace.

Without `--seed`, the RNG seed is derived from the wall clock and
reported to stderr on first use so the user can pin it on the next
run.

### Runaway guards

Two hard caps on pathological execution:

| Guard                   | Limit              | On trip                              |
|:------------------------|:-------------------|:-------------------------------------|
| `while`-loop iterations | 1,000,000 per loop | `while loop exceeded ... (runaway?)` (fatal) |
| VM call-stack depth     | 1,024 frames       | `CallStackOverflow` (fatal)          |

Both are compile-time constants in the reference implementation.
They are not tunable via CLI flags.

---

## Appendix A: Cross-reference

- [Syntax Reference](syntax) — tutorial-oriented walkthrough of
  the same features, with worked examples.
- [Philosophy](philosophy) — design rationale for the type system,
  `live { }` blocks, and the verifier.
- [Error Reference](errors/) — stable error codes (E0001+)
  corresponding to each user-visible diagnostic.
- [Memory Model](memory-model) — evaluation model, aliasing, and
  the state-snapshot mechanism used by `live` blocks.
- [`no_std` runtime](no-std) — subset of the specification that
  survives the embedded profile.
