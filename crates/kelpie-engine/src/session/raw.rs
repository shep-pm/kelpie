//! What a harness sends beyond ACP, and how kelpie reads it
//!
//! ACP's per-turn usage counts do not carry Claude Code's split between the
//! one-hour and the five-minute prompt cache (ADR 0007). The bridge sends its
//! raw SDK messages as `_claude/sdkMessage` when kelpie asks for them on
//! `session/new`, and the `result` among them carries the split
//! (`docs/acp.md`, Checked by hand).

use kelpie_protocol::CacheLifetime;

use super::Extension;

/// The cache lifetime `params` name for the turn in flight, `None` when they name none
pub(super) fn cache_lifetime(
    extension: Extension,
    params: &serde_json::Value,
) -> Option<CacheLifetime> {
    match extension {
        Extension::ClaudeSdkMessage => claude_sdk_message(params),
    }
}

/// The lifetime the turn's cache write got, from one raw Claude Code SDK message
///
/// Every raw message but the turn's `result` names no lifetime. When one turn
/// wrote caches of both lifetimes, the one holding more of the tokens wins.
fn claude_sdk_message(params: &serde_json::Value) -> Option<CacheLifetime> {
    let message = params.get("message")?;
    if message.get("type")?.as_str()? != "result" {
        return None;
    }
    let creation = message.get("usage")?.get("cache_creation")?;
    let one_hour = count(creation, "ephemeral_1h_input_tokens");
    let five_minutes = count(creation, "ephemeral_5m_input_tokens");
    if one_hour == 0 && five_minutes == 0 {
        return None;
    }
    if five_minutes > one_hour {
        return Some(CacheLifetime::FiveMinutes);
    }
    Some(CacheLifetime::OneHour)
}

/// A token count, `0` when the message does not give one
fn count(json: &serde_json::Value, key: &str) -> u64 {
    json.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}
