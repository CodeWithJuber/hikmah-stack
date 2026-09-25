//! JSONL bridge for paired, offline replay of public-dataset engine responses.
//! Uses the actual kernel; it never calls a network service or reads an API key.
use hikmah_kernel::decision_port::{
    ask, DecisionRequest, EngineDescriptor, NoEngine, RawAnswer, StaticEngine,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::time::Instant;

fn process(value: Value) -> Result<Value, String> {
    let request: DecisionRequest =
        serde_json::from_value(value["request"].clone()).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let baseline = ask(&NoEngine, &request).map_err(|e| e.to_string())?;
    let baseline_us = started.elapsed().as_micros();
    let mut output = json!({"baseline": baseline, "baseline_us": baseline_us});
    if let Some(response) = value.get("response") {
        let values = response["answers"]
            .as_object()
            .ok_or("response has no answers object")?;
        let answers: BTreeMap<String, RawAnswer> = values
            .iter()
            .map(|(id, answer)| {
                let parsed = serde_json::from_value(answer.clone()).unwrap_or_else(|e| {
                    RawAnswer::Malformed {
                        detail: e.to_string(),
                    }
                });
                (id.clone(), parsed)
            })
            .collect();
        let engine = StaticEngine {
            descriptor: EngineDescriptor {
                name: "jev-paired-replay".into(),
                version: response["model"].as_str().unwrap_or("unknown").into(),
            },
            answers,
        };
        let started = Instant::now();
        let admitted = ask(&engine, &request).map_err(|e| e.to_string())?;
        output["admission_us"] = json!(started.elapsed().as_micros());
        output["combined"] = json!(admitted);
    }
    Ok(output)
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let result = line
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
            .and_then(process);
        let output = result.unwrap_or_else(|error| json!({"error": error}));
        writeln!(stdout, "{}", output)?;
        stdout.flush()?;
    }
    Ok(())
}
