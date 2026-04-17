---
id: RES-052
title: Typed declarations (let and fn return type annotations)
state: DONE
priority: P1
goalpost: G7
created: 2026-04-16
owner: executor
---

## Summary
Phase 3 kickoff. Before the typechecker can reject bad programs, the
grammar needs to be able to express types. Today parameters already
use a `type_name name` form; variables and return types don't.

This ticket lays the syntactic groundwork. The typechecker
continues to be permissive (RES-053 is where it starts rejecting).

## Acceptance criteria

    let x: int = 42;
    let name: string = "Resilient";

    fn add(int a, int b) -> int {
        return a + b;
    }

    fn void_fn() -> void {
        println("hi");
    }

- `let NAME: TYPE = EXPR;` — optional `: TYPE` part (existing `let x = ...` still works)
- `fn NAME(params) -> TYPE { ... }` — optional `-> TYPE` part (existing `fn f() { ... }` still works)
- Type names: `int`, `float`, `string`, `bool`, `void`, or a user-defined struct name
- AST records the type on `LetStatement` and `Function` (and `FunctionLiteral`)
- Tests parse typed forms and confirm the type flows into the AST
- Runtime behavior unchanged — types are advisory until RES-053

## Notes
- New Token::Arrow (`->`) already exists? Check; if not, add.
- LetStatement gains `type_annot: Option<String>`
- Function / FunctionLiteral gain `return_type: Option<String>`
- Typechecker: for this ticket, just accept the new shape; no
  enforcement. Enforcement is RES-053.

## Log
- 2026-04-16 created and claimed
