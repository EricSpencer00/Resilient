// RES-3943: fuzz the hand-rolled MCP HTTP request parser.
//
// The compiler exposes a doc-hidden, native-only seam that keeps this target
// in-process and on the exact byte-to-response path used by the HTTP server.
// Invalid requests are expected to become HTTP error responses; none may
// panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _response = resilient::fuzz_http_request(data);
});
