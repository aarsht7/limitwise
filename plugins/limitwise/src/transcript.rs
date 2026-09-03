use serde_json::Value;

pub fn token_usage(content: &str) -> Option<i64> {
    let mut total = 0_i64;
    let mut found = false;
    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("turn.completed") {
            continue;
        }
        let Some(usage) = value.get("usage") else {
            continue;
        };
        let tokens = usage
            .get("total_tokens")
            .and_then(Value::as_i64)
            .or_else(|| {
                Some(usage.get("input_tokens")?.as_i64()? + usage.get("output_tokens")?.as_i64()?)
            });
        if let Some(tokens) = tokens.filter(|tokens| *tokens >= 0) {
            total = total.saturating_add(tokens);
            found = true;
        }
    }
    found.then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_turn_token_usage() {
        let transcript = concat!(
            "not json\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"output_tokens\":20,\"reasoning_output_tokens\":5}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"total_tokens\":30}}\n"
        );
        assert_eq!(token_usage(transcript), Some(150));
        assert_eq!(token_usage("{\"type\":\"turn.completed\"}"), None);
    }
}
