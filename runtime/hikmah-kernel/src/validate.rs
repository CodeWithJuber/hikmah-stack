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
        check_skill_links(&entry.path())?;
    }
    notes.push(format!("{} unique skills", skill_names.len()));
    Ok(notes)
}

/// Skills are installed one directory at a time, so a relative Markdown link must resolve to an
/// existing file inside the same skill directory. Absolute URLs and in-page anchors are allowed.
fn check_skill_links(skill_dir: &Path) -> Result<()> {
    let mut pending = vec![skill_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            for target in markdown_link_targets(&text) {
                if let Some(problem) = skill_link_problem(skill_dir, &dir, target) {
                    return Err(invalid(format!(
                        "{}: link `{target}` {problem}; skills must be self-contained",
                        path.display()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn markdown_link_targets(text: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else { break };
        let target = rest[..end].split_whitespace().next().unwrap_or("");
        targets.push(target);
        rest = &rest[end..];
    }
    targets
}

fn skill_link_problem(skill_dir: &Path, file_dir: &Path, target: &str) -> Option<&'static str> {
    let path_part = target.split('#').next().unwrap_or("");
    if path_part.is_empty() || target.contains("://") || target.starts_with("mailto:") {
        return None;
    }
    if path_part.starts_with('/') {
        return Some("is an absolute path");
    }
    let base = file_dir.strip_prefix(skill_dir).ok()?;
    let mut depth: Vec<&std::ffi::OsStr> = base.iter().collect();
    for part in Path::new(path_part).components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if depth.pop().is_none() {
                    return Some("escapes the skill directory");
                }
            }
            std::path::Component::Normal(name) => depth.push(name),
            _ => return Some("is not a relative path"),
        }
    }
    let resolved = depth
        .iter()
        .fold(skill_dir.to_path_buf(), |acc, part| acc.join(part));
    if resolved.exists() {
        None
    } else {
        Some("does not resolve inside the skill directory")
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_links_must_stay_inside_the_skill() {
        let skill = std::env::temp_dir().join(format!("hikmah-skill-links-{}", std::process::id()));
        fs::create_dir_all(skill.join("references")).unwrap();
        fs::write(skill.join("references/notes.md"), "x").unwrap();
        let refs = skill.join("references");

        assert_eq!(
            skill_link_problem(&skill, &skill, "references/notes.md"),
            None
        );
        assert_eq!(
            skill_link_problem(&skill, &refs, "../references/notes.md#top"),
            None
        );
        assert_eq!(
            skill_link_problem(&skill, &skill, "https://example.com/a.md"),
            None
        );
        assert_eq!(skill_link_problem(&skill, &skill, "#section"), None);
        assert!(skill_link_problem(&skill, &skill, "../../docs/MEMORY.md").is_some());
        assert!(skill_link_problem(&skill, &refs, "../../other/SKILL.md").is_some());
        assert!(skill_link_problem(&skill, &skill, "references/missing.md").is_some());
        assert!(skill_link_problem(&skill, &skill, "/etc/passwd").is_some());

        assert_eq!(
            markdown_link_targets("see [a](x.md \"title\") and [b](https://e.com)"),
            vec!["x.md", "https://e.com"]
        );
        fs::remove_dir_all(&skill).unwrap();
    }
}
