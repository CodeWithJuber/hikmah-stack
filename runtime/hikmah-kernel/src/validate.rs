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

/// Validate the repository without checking the `hikmah` commands skills name; the `hikmah
/// validate` command uses [`validate_repo_with_cli`].
pub fn validate_repo(root: impl AsRef<Path>) -> Result<Vec<String>> {
    validate(root.as_ref(), None)
}

/// Validate the repository, and also check that every `hikmah <subcommand> --flag` a skill
/// names exists in `cli` (the binary's clap definition), so a skill cannot cite a command or
/// flag the binary rejects.
pub fn validate_repo_with_cli(root: impl AsRef<Path>, cli: &clap::Command) -> Result<Vec<String>> {
    validate(root.as_ref(), Some(cli))
}

fn validate(root: &Path, cli: Option<&clap::Command>) -> Result<Vec<String>> {
    // Built once, so help flags and global options are visible on every subcommand.
    let cli = cli.map(|cli| {
        let mut cli = cli.clone();
        cli.build();
        cli
    });
    let mut command_refs = 0;
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
        // A skill is doctrine plus the commands that execute it. Every skill must name at
        // least one `hikmah` command it wires to, so the pack cannot drift back into
        // instruction-only text with no executable path.
        if !skill_wires_to_kernel(&text) {
            return Err(invalid(format!(
                "skill `{name}` names no `hikmah` command: add an \"Execute with the kernel\" section wiring it to the binary"
            )));
        }
        if let Some(cli) = &cli {
            let problems = skill_command_problems(&text, cli);
            if !problems.is_empty() {
                return Err(invalid(format!(
                    "skill `{name}` names commands the `hikmah` binary rejects: {}",
                    problems.join("; ")
                )));
            }
            command_refs += skill_command_refs(&text).len();
        }
        check_skill_links(&entry.path())?;
    }
    notes.push(format!("{} unique skills", skill_names.len()));
    if cli.is_some() {
        notes.push(format!(
            "{command_refs} skill command reference(s) match the CLI"
        ));
    }
    Ok(notes)
}

/// The `hikmah ...` commands a skill names: inline code spans that start with `hikmah`, and lines
/// that start with `hikmah` (or `$ hikmah`) inside fenced code blocks.
fn skill_command_refs(text: &str) -> Vec<&str> {
    let is_command = |code: &str| code == "hikmah" || code.starts_with("hikmah ");
    let mut refs = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            let code = trimmed.strip_prefix("$ ").unwrap_or(trimmed);
            if is_command(code) {
                refs.push(code);
            }
            continue;
        }
        // Between backticks: every odd piece of the line is inline code.
        refs.extend(
            line.split('`')
                .skip(1)
                .step_by(2)
                .filter(|code| is_command(code)),
        );
    }
    refs
}

/// Problems with the `hikmah` commands a skill names: an unknown subcommand, or a `--flag` the
/// subcommand (or the global options) does not have. `cli` must be built (`Command::build`).
/// Placeholders such as `<trace-id>`, `...`, or `[--record]`'s brackets are not checked.
pub fn skill_command_problems(text: &str, cli: &clap::Command) -> Vec<String> {
    let mut problems = Vec::new();
    for reference in skill_command_refs(text) {
        let mut command = cli;
        let mut path = String::from("hikmah");
        let mut tokens = reference.split_whitespace().skip(1);
        while let Some(token) = tokens.next() {
            let token = token
                .trim_start_matches(['[', '('])
                .trim_end_matches([']', ')', ',', ';', '.']);
            if let Some(flag) = token.strip_prefix("--") {
                let (name, inline_value) = match flag.split_once('=') {
                    Some((name, value)) => (name, Some(value)),
                    None => (flag, None),
                };
                if name.is_empty() {
                    continue;
                }
                let arg = command
                    .get_arguments()
                    .find(|arg| arg.get_long() == Some(name));
                match arg {
                    None => problems.push(format!("`{path}` has no `--{name}` (in `{reference}`)")),
                    // Before the subcommand, a flag's value must not be read as the subcommand.
                    Some(arg)
                        if command.has_subcommands()
                            && inline_value.is_none()
                            && arg.get_action().takes_values() =>
                    {
                        tokens.next();
                    }
                    Some(_) => {}
                }
                continue;
            }
            if !command.has_subcommands() {
                continue;
            }
            if !token.starts_with(|c: char| c.is_ascii_alphabetic()) {
                break; // a placeholder such as `<command>`: nothing more to check
            }
            match command.find_subcommand(token) {
                Some(sub) => {
                    command = sub;
                    path = format!("{path} {token}");
                }
                None => {
                    problems.push(format!("`{path}` has no subcommand `{token}`"));
                    break;
                }
            }
        }
    }
    problems
}

/// A skill is doctrine plus the commands that execute it: its SKILL.md must name at least
/// one `hikmah` command it wires to, so the pack cannot drift back into instruction-only
/// text with no executable path.
fn skill_wires_to_kernel(text: &str) -> bool {
    text.contains("`hikmah")
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

    #[test]
    fn skill_commands_must_exist_in_the_cli() {
        use clap::{Arg, ArgAction, Command};
        let mut cli = Command::new("hikmah")
            .version("1.0.0")
            .arg(Arg::new("policy").long("policy").global(true))
            .subcommand(
                Command::new("decide")
                    .arg(Arg::new("frame").long("frame").required(true))
                    .arg(Arg::new("engine").long("engine"))
                    .arg(Arg::new("record").long("record").action(ArgAction::SetTrue)),
            )
            .subcommand(Command::new("get").arg(Arg::new("id")));
        cli.build();
        let problems = |text: &str| skill_command_problems(text, &cli);

        for ok in [
            "- `hikmah decide --frame frame.json [--engine jev --record]` ranks options.",
            "`hikmah get <trace-id>` / `hikmah` on PATH / `cargo install --git x hikmah-kernel`",
            "`hikmah --policy p.json decide --frame f.json`",
            "`hikmah decide --policy=p.json --frame f.json --help`",
            "`hikmah --version`",
            "```bash\nhikmah decide --frame f.json\n$ hikmah get tr_1\n```",
            "the hikmah binary, never `hikmah-kernel` alone",
        ] {
            assert_eq!(problems(ok), Vec::<String>::new(), "{ok}");
        }

        // The regression this check exists for: flags the binary rejects.
        let wrong = problems("- `hikmah decide --criteria ... --options ...` ranks options.");
        assert_eq!(wrong.len(), 2, "{wrong:?}");
        assert!(wrong[0].contains("`hikmah decide` has no `--criteria`"));
        assert!(wrong[1].contains("`--options`"));
        assert_eq!(problems("`hikmah decid --frame f`").len(), 1);
        assert_eq!(problems("`hikmah --verbose decide --frame f`").len(), 1);
        assert_eq!(problems("```\nhikmah nope\n```").len(), 1);
        // A fence closes: text after it is prose again, and inline spans are checked.
        assert_eq!(problems("```\nx\n```\n`hikmah get --bogus`").len(), 1);
    }

    #[test]
    fn every_skill_must_wire_to_the_kernel() {
        assert!(skill_wires_to_kernel(
            "## Execute with the kernel\n\n- `hikmah recall --query ...`"
        ));
        assert!(skill_wires_to_kernel("run `hikmah verify-ledger` first"));
        assert!(!skill_wires_to_kernel(
            "# Pure doctrine\n\nTwelve rules for builders. No commands here."
        ));
        // A bare mention without backticks is prose, not a command reference.
        assert!(!skill_wires_to_kernel("the hikmah binary does things"));
    }
}
