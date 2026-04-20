#!/usr/bin/env resilient
// RES-113: the `#!` on line 1 is silently skipped by the lexer so
// Resilient scripts can be made executable. Everything below is
// ordinary source.
fn main() {
    println("ok");
}

main();
