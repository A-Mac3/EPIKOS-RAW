//! Optional AI interpretation of "Describe a look" with the user's own API key
//! (Anthropic or OpenAI).
//!
//! The key is kept in the macOS Keychain only. It is read here, in the app's backend,
//! for the request, and is never returned to the web view, written to disk or logged.
//! Only the prompt text is sent (never the photo or file names); the reply is applied
//! through [`epikos_engine::apply_ai_look`], which bounds every value.

use std::time::Duration;

const SERVICE: &str = "EPIKOS RAW AI";

fn account(provider: &str) -> Result<&'static str, String> {
    match provider {
        "anthropic" => Ok("anthropic"),
        "openai" => Ok("openai"),
        other => Err(format!("unknown AI provider \"{other}\"")),
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    pub fn set(service: &str, account: &str, key: &str) -> Result<(), String> {
        set_generic_password(service, account, key.as_bytes()).map_err(|e| format!("Keychain: {e}"))
    }
    pub fn get(service: &str, account: &str) -> Option<String> {
        get_generic_password(service, account).ok().and_then(|b| String::from_utf8(b).ok())
    }
    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        match delete_generic_password(service, account) {
            Ok(()) => Ok(()),
            // Already gone is fine.
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(format!("Keychain: {e}")),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    pub fn set(_: &str, _: &str, _: &str) -> Result<(), String> {
        Err("Saving an API key needs the macOS Keychain".into())
    }
    pub fn get(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn delete(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

pub fn save_key(provider: &str, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.len() < 20 || key.contains(char::is_whitespace) {
        return Err("That doesn't look like an API key".into());
    }
    keychain::set(SERVICE, account(provider)?, key)
}

pub fn has_key(provider: &str) -> Result<bool, String> {
    Ok(keychain::get(SERVICE, account(provider)?).is_some())
}

pub fn delete_key(provider: &str) -> Result<(), String> {
    keychain::delete(SERVICE, account(provider)?)
}

fn agent() -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// The first JSON object in `text` (models sometimes wrap it in a code fence).
fn json_in(text: &str) -> Result<serde_json::Value, String> {
    let start = text.find('{').ok_or("The AI reply had no settings in it")?;
    let end = text.rfind('}').ok_or("The AI reply had no settings in it")?;
    serde_json::from_str(&text[start..=end]).map_err(|_| "The AI reply wasn't valid settings JSON".into())
}

/// Ask `provider`'s `model` to translate `prompt` into setting changes.
pub fn interpret(provider: &str, model: &str, prompt: &str) -> Result<serde_json::Value, String> {
    let key = keychain::get(SERVICE, account(provider)?).ok_or("No API key saved for this provider (Settings)")?;
    let instructions = epikos_engine::AI_LOOK_INSTRUCTIONS;
    let prompt: String = prompt.chars().take(2000).collect();
    let (url, body) = match provider {
        "anthropic" => (
            "https://api.anthropic.com/v1/messages",
            serde_json::json!({
                "model": if model.trim().is_empty() { "claude-sonnet-5" } else { model.trim() },
                "max_tokens": 1024,
                "system": instructions,
                "messages": [{"role": "user", "content": prompt}],
            }),
        ),
        _ => (
            "https://api.openai.com/v1/chat/completions",
            serde_json::json!({
                "model": if model.trim().is_empty() { "gpt-4o" } else { model.trim() },
                "response_format": {"type": "json_object"},
                "messages": [
                    {"role": "system", "content": instructions},
                    {"role": "user", "content": prompt},
                ],
            }),
        ),
    };
    let request = agent().post(url).header("content-type", "application/json");
    let request = if provider == "anthropic" {
        request.header("x-api-key", &key).header("anthropic-version", "2023-06-01")
    } else {
        request.header("authorization", &format!("Bearer {key}"))
    };
    drop(key);
    let mut response = request
        .send(body.to_string())
        .map_err(|e| format!("Couldn't reach the AI service: {}", redact(&e.to_string())))?;
    let status = response.status();
    let text = response.body_mut().read_to_string().map_err(|e| redact(&e.to_string()))?;
    let reply: serde_json::Value = serde_json::from_str(&text).map_err(|_| format!("The AI service answered {status}"))?;
    if !status.is_success() {
        let msg = reply.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("request failed");
        return Err(format!("AI service {}: {}", status.as_u16(), redact(msg)));
    }
    let content = match provider {
        "anthropic" => reply.pointer("/content/0/text"),
        _ => reply.pointer("/choices/0/message/content"),
    }
    .and_then(|c| c.as_str())
    .ok_or("The AI reply was empty")?;
    json_in(content)
}

/// Never echo anything that looks like a key back to the UI.
fn redact(s: &str) -> String {
    s.split_whitespace()
        .map(|w| if w.starts_with("sk-") || w.len() > 40 { "[redacted]" } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_found_in_fences_and_keys_never_echo() {
        let v = json_in("Here you go:\n```json\n{\"changes\": {\"exposure\": 0.3}}\n```").unwrap();
        assert_eq!(v["changes"]["exposure"], 0.3);
        assert!(json_in("no json here").is_err());
        assert_eq!(redact("invalid key sk-ant-abc123 given"), "invalid key [redacted] given");
        assert!(account("other").is_err());
        assert!(save_key("anthropic", "short").is_err(), "obviously not a key");
    }
}
