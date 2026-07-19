// RES-3943: cargo-fuzz target for the MCP HTTP wrapper's hand-rolled
// request-line / header / `Content-Length` parser.
//
// Unlike the `parse`/`lex` targets in this crate, the HTTP parser has no
// CLI-boundary equivalent to shell out to — it only exists inside the MCP
// HTTP wrapper (`resilient::mcp_server`), which is a library-only,
// non-wasm32 module. So this target depends on the `resilient` library
// crate directly and calls the narrow, `#[doc(hidden)] pub` seam
// `mcp_server::fuzz_parse_http_request`, which exercises exactly the
// parsing logic used by `handle_http_stream` (request-line extraction,
// `\r\n\r\n` header/body split, `Content-Length` parsing, and the
// request-complete check) without opening a socket or touching tool
// dispatch.
//
// Invariant: for any byte sequence — valid or invalid UTF-8, complete or
// truncated HTTP, negative/overflowing/missing `Content-Length` — the
// parser must never panic.

#![no_main]

use libfuzzer_sys::fuzz_target;
use resilient::mcp_server::fuzz_parse_http_request;

fuzz_target!(|data: &[u8]| {
    fuzz_parse_http_request(data);
});
