//! Package validator for the repository: manifests, versions, hooks, and skills.
use crate::error::{KernelError, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const PLUGIN_NAME: &str = "hikmah-stack";

fn invalid(message: impl Into<String>) -> KernelError {
    KernelError::Invalid(message.into())
}

pub fn validate_repo(root: impl AsRef<Path>) -> Result<Vec<String>> {
    let root = root.as_ref();
    let required = [
        ".codex-plugin/plugin.json",
        ".claude-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
        ".agents/plugins/marketplace.json",
        "kimi.plugin.json",
        "hooks/hooks.json",
        "hooks/codex.json",
        "hooks/truth_gate.sh",
        "hooks/truth_gate.py",
        "hooks/truth_gate_cases.json",
        "skills/operator-core/SKILL.md",
        "skills/agent-radar/SKILL.md",
        "skills/decision-forge/SKILL.md",
        "skills/ship-guard/SKILL.md",
        "skills/hikmah-orchestrator/SKILL.md",
        "skills/cognitive-kernel/SKILL.md",
        "docs/COGNITIVE_KERNEL.md",
        "docs/MEMORY.md",
        "docs/DECISION_PORT.md",
        "runtime/hikmah-kernel/Cargo.toml",
    ];
    let mut notes = Vec::new();
    for path in required {
        if !root.join(path).is_file() {
            return Err(invalid(format!("missing required file: {path}")));
        }
    }

    let mut json = |path: &str| -> Result<Value> {
        let text = fs::read_to_string(root.join(path))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| invalid(format!("{path} is not valid JSON: {error}")))?;
        notes.push(format!("json ok: {path}"));
        Ok(value)
    };
    let codex = json(".codex-plugin/plugin.json")?;
    let claude = json(".claude-plugin/plugin.json")?;
    let claude_market = json(".claude-plugin/marketplace.json")?;
    let agents_market = json(".agents/plugins/marketplace.json")?;
    let kimi = json("kimi.plugin.json")?;
    let hooks = json("hooks/hooks.json")?;
    let codex_hooks = json("hooks/codex.json")?;
    let cases = json("hooks/truth_gate_cases.json")?;

    // Versions: one release number everywhere.
    let cargo_version = cargo_package_version(&root.join("runtime/hikmah-kernel/Cargo.toml"))?;
    let mut versions = vec![
        (
            "runtime/hikmah-kernel/Cargo.toml".to_string(),
            cargo_version.clone(),
        ),
        (
            ".codex-plugin/plugin.json".into(),
            field(&codex, "version")?,
        ),
        (
            ".claude-plugin/plugin.json".into(),
            field(&claude, "version")?,
        ),
        ("kimi.plugin.json".into(), field(&kimi, "version")?),
    ];
    for (index, plugin) in plugin_entries(&claude_market).iter().enumerate() {
        if let Some(version) = plugin.get("version").and_then(Value::as_str) {
            versions.push((
                format!(".claude-plugin/marketplace.json plugins[{index}]"),
                version.to_string(),
            ));
        }
    }
    for (path, version) in &versions {
        if *version != cargo_version {
            return Err(invalid(format!(
                "version mismatch: {path} has {version}, Cargo.toml has {cargo_version}"
            )));
        }
    }
    notes.push(format!(
        "version {cargo_version} in {} places",
        versions.len()
    ));

    // Names: every host installs the same plugin.
    let mut names = vec![
        (
            ".codex-plugin/plugin.json".to_string(),
            field(&codex, "name")?,
        ),
        (".claude-plugin/plugin.json".into(), field(&claude, "name")?),
        ("kimi.plugin.json".into(), field(&kimi, "name")?),
    ];
    for (label, market) in [
        (".claude-plugin/marketplace.json", &claude_market),
        (".agents/plugins/marketplace.json", &agents_market),
    ] {
        for plugin in plugin_entries(market) {
            names.push((label.to_string(), field(plugin, "name")?));
        }
    }
    for (path, name) in &names {
        if name != PLUGIN_NAME {
            return Err(invalid(format!(
                "plugin name mismatch: {path} has {name}, expected {PLUGIN_NAME}"
            )));
        }
    }
    notes.push(format!(
        "plugin name {PLUGIN_NAME} in {} places",
        names.len()
    ));

    // Hooks: every script a hook command references must exist.
    let script = Regex::new(r"hooks/[A-Za-z0-9_.\-]+").expect("script regex");
    let mut referenced = BTreeSet::new();
    for config in [&hooks, &codex_hooks] {
        collect_commands(config, &mut |command| {
            for m in script.find_iter(command) {
                referenced.insert(m.as_str().to_string());
            }
        });
    }
    for path in &referenced {
        if !root.join(path).is_file() {
            return Err(invalid(format!("hook command references missing {path}")));
        }
    }
    notes.push(format!(
        "{} hook script reference(s) resolve",
        referenced.len()
    ));

    let case_count = cases
        .get("cases")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if case_count == 0 {
        return Err(invalid("hooks/truth_gate_cases.json has no cases"));
    }
    notes.push(format!("{case_count} Truth Gate golden cases"));

    // Skills: frontmatter name equals the directory, description is present.
    let mut skill_names = BTreeSet::new();
    for entry in fs::read_dir(root.join("skills"))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let skill_path: PathBuf = entry.path().join("SKILL.md");
        if !skill_path.is_file() {
            return Err(invalid(format!(
                "skill directory missing SKILL.md: {}",
                entry.path().display()
            )));
        }
        let text = fs::read_to_string(&skill_path)?;
        let name = parse_frontmatter_field(&text, "name").unwrap_or_default();
        if name.is_empty() {
            return Err(invalid(format!(
                "skill missing name: {}",
                skill_path.display()
            )));
        }
        if name != dir_name {
            return Err(invalid(format!(
                "skill name `{name}` does not match its directory `{dir_name}`"
            )));
        }
        let description = parse_frontmatter_field(&text, "description").unwrap_or_default();
        if description.is_empty() {
            return Err(invalid(format!(
                "skill missing frontmatter description: {}",
                skill_path.display()
            )));
        }
        if !skill_names.insert(name.clone()) {
            return Err(invalid(format!("duplicate skill name: {name}")));
        }
    }
    notes.push(format!("{} unique skills", skill_names.len()));
    Ok(notes)
}

fn field(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid(format!("manifest missing `{key}`")))
}

fn plugin_entries(market: &Value) -> Vec<&Value> {
    market
        .get("plugins")
        .and_then(Value::as_array)
        .map(|plugins| plugins.iter().collect())
        .unwrap_or_default()
}

fn collect_commands(value: &Value, visit: &mut dyn FnMut(&str)) {
    match value {
        Value::Object(map) => {
            if let Some(command) = map.get("command").and_then(Value::as_str) {
                visit(command);
            }
            for child in map.values() {
                collect_commands(child, visit);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_commands(item, visit);
            }
        }
        _ => {}
    }
}

fn cargo_package_version(path: &Path) -> Result<String> {
    let text = fs::read_to_string(path)?;
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = trimmed.strip_prefix("version") {
                if let Some(value) = rest.trim_start().strip_prefix('=') {
                    return Ok(value.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    Err(invalid("Cargo.toml has no [package] version"))
}

fn parse_frontmatter_field(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix(&format!("{key}:")) {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}
