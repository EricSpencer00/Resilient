//! Shared MCP JSON-RPC envelope handling for all transports.

use serde_json::Value;

/// Extract the common JSON-RPC fields before invoking the transport's
/// dispatch function. Keeping this boundary independent makes stdio and HTTP
/// callers use identical notification and missing-method semantics.
pub(crate) fn handle_message(
    msg: &Value,
    dispatch: impl FnOnce(&str, &Value, Option<&Value>, bool) -> Option<Value>,
) -> Option<Value> {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let is_notification = msg.get("id").is_none();
    dispatch(method, &id, msg.get("params"), is_notification)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn handler_forwards_json_rpc_envelope_to_dispatch() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "ping",
            "params": { "trace": true }
        });
        let response = handle_message(&message, |method, id, params, is_notification| {
            assert_eq!(method, "ping");
            assert_eq!(id, &json!(7));
            assert_eq!(params, Some(&json!({ "trace": true })));
            assert!(!is_notification);
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} }))
        })
        .expect("handler should return the dispatch response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 7);
        assert!(response["result"].is_object(), "got: {response}");
    }
}
