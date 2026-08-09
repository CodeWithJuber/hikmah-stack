use crate::error::{KernelError, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn validate_repo(root: impl AsRef<Path>) -> Result<Vec<String>> {
    let root = root.as_ref();
    let required = [
        ".codex-plugin/plugin.json",
        ".claude-plugin/plugin.json",
        "skills/operator-core/SKILL.md",
        "skills/agent-radar/SKILL.md",
        "skills/decision-forge/SKILL.md",
        "skills/ship-guard/SKILL.md",
        "skills/hikmah-orchestrator/SKILL.md",
        "skills/cognitive-kernel/SKILL.md",
        "docs/COGNITIVE_KERNEL.md",
        "docs/MEMORY.md",
        "runtime/hikmah-kernel/Cargo.toml",
    ];
    let mut notes = Vec::new();
    for path in required {
        let full = root.join(path);
        if !full.is_file() {
            return Err(KernelError::Invalid(format!(
                "missing required file: {path}"
            )));
        }
    }

    for path in [
        ".codex-plugin/plugin.json",
        ".claude-plugin/plugin.json",
        ".agents/plugins/marketplace.json",
        ".claude-plugin/marketplace.json",
        "hooks/codex.json",
        "hooks/hooks.json",
    ] {
        let text = fs::read_to_string(root.join(path))?;
        let _: Value = serde_json::from_str(&text)?;
        notes.push(format!("json ok: {path}"));
    }

    let skills_dir = root.join("skills");
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(&skills_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let skill_path = entry.path().join("SKILL.md");
        if !skill_path.is_file() {
            return Err(KernelError::Invalid(format!(
                "skill directory missing SKILL.md: {}",
                entry.path().display()
            )));
        }
        let text = fs::read_to_string(&skill_path)?;
        let name = parse_frontmatter_field(&text, "name").ok_or_else(|| {
            KernelError::Invalid(format!("skill missing name: {}", skill_path.display()))
        })?;
        if !names.insert(name.clone()) {
            return Err(KernelError::Invalid(format!(
                "duplicate skill name: {name}"
            )));
        }
        if !text.contains("description:") {
            return Err(KernelError::Invalid(format!(
                "skill missing description: {}",
                skill_path.display()
            )));
        }
    }
    notes.push(format!("{} unique skills", names.len()));

    let version = plugin_version(root.join(".codex-plugin/plugin.json"))?;
    let path = ".claude-plugin/plugin.json";
    let other = plugin_version(root.join(path))?;
    if other != version {
        return Err(KernelError::Invalid(format!(
            "manifest version mismatch: {path} has {other}, expected {version}"
        )));
    }
    notes.push(format!("manifest version {version}"));
    Ok(notes)
}

fn plugin_version(path: PathBuf) -> Result<String> {
    let text = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&text)?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| KernelError::Invalid("plugin manifest missing version".into()))
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
