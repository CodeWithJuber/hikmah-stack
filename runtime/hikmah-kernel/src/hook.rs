use serde_json::{json, Value};
use std::io::{Read, Write};

use crate::error::Result;

pub fn run_stop_hook(mut input: impl Read, mut output: impl Write) -> Result<()> {
    let mut buffer = String::new();
    input.read_to_string(&mut buffer)?;
    let payload: Value = match serde_json::from_str(&buffer) {
        Ok(value) => value,
        Err(_) => {
            writeln!(output, "{{}}")?;
            return Ok(());
        }
    };
    if payload
        .get("stop_hook_active")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        writeln!(output, "{{}}")?;
        return Ok(());
    }

    let text = payload
        .get("last_assistant_message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let completion = contains_any(
        &text,
        &[
            "done",
            "complete",
            "completed",
            "finished",
            "ready",
            "shipped",
            "implemented",
            "fixed",
        ],
    );
    let unfinished = contains_any(
        &text,
        &[
            "todo",
            "tbd",
            "fixme",
            "placeholder",
            "coming soon",
            "<insert",
            "[insert",
        ],
    );
    let future_promise = [
        "i'll finish",
        "i will finish",
        "we'll finish",
        "we will finish",
        "i'll upload",
        "i will upload",
        "i'll verify",
        "i will verify",
        "i'll provide",
        "i will provide",
    ]
    .iter()
    .any(|needle| text.contains(needle));

    if completion && (unfinished || future_promise) {
        let response = json!({
            "decision": "block",
            "reason": "Hikmah Truth Gate: the response claims completion while still containing unfinished work or a future-work promise. Resolve it or state the limitation explicitly."
        });
        writeln!(output, "{response}")?;
    } else {
        writeln!(output, "{{}}")?;
    }
    Ok(())
}

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| {
        text.split(|c: char| !c.is_alphanumeric() && c != '<' && c != '[')
            .any(|token| token == *word)
            || text.contains(word)
    })
}
