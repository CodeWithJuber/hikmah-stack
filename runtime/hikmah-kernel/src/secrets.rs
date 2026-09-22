//! Conservative credential detection for text about to leave the machine (decision engines) and
//! for every trace before it is written to memory (`Trace::validate`).
//!
//! Uses the `regex` crate, whose matching time is linear in the input, so a crafted input cannot
//! stall a hook. This is a guard rail for well-known credential shapes, not a DLP system.
use regex::{Regex, RegexSet};
use std::sync::OnceLock;

/// Shapes that are credentials whenever they match.
fn patterns() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| {
        RegexSet::new([
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
            r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
            r"\bgh[pousr]_[A-Za-z0-9]{30,}\b",
            r"\bgithub_pat_[A-Za-z0-9_]{40,}\b",
            r"\bglpat-[A-Za-z0-9_\-]{20,}\b",
            r"\bsk-ant-[A-Za-z0-9_\-]{20,}\b",
            r"\b[sr]k_live_[A-Za-z0-9]{16,}\b",
            r"\bxox[abprs]-[A-Za-z0-9\-]{10,}\b",
            r"\bAIza[0-9A-Za-z_\-]{35}\b",
            r"\bapikey_[0-9a-f]{32,}_[0-9a-f]{32,}\b",
            r"\beyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
            // scheme://user:password@host
            r"\b[a-z][a-z0-9+.\-]{1,20}://[^\s:/@]{1,64}:[^\s@/]{3,128}@[^\s/]+",
        ])
        .expect("secret patterns compile")
    })
}

/// `sk-...` keys. Real keys are random, so they contain a digit; hyphenated prose such as
/// `sk-learn-model-selection-cross-validation` does not.
fn sk_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{32,}\b").expect("sk regex"))
}

/// KEY=value / "key": "value" assignments of secret-looking names. Group 1 is the value.
fn assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)\b[a-z0-9_\-]{0,40}(?:password|passwd|secret|api[_\-]?key|access[_\-]?token|auth[_\-]?token|private[_\-]?key)[a-z0-9_\-]{0,20}["']?\s*[:=]\s*["']?([^\s"',;]{8,})"#)
            .expect("assignment regex")
    })
}

/// A value that names where a secret lives, or stands in for one, rather than being one.
/// Recording a reference is exactly what the memory error message asks people to do.
fn is_reference_or_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.starts_with(['$', '<', '%', '{', '*'])
        || ["vault:", "env:", "redacted", "[redacted"]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
}

/// True when `text` contains something shaped like a credential.
pub fn contains_secret(text: &str) -> bool {
    patterns().is_match(text)
        || sk_key()
            .find_iter(text)
            .any(|m| m.as_str().bytes().any(|b| b.is_ascii_digit()))
        || assignment()
            .captures_iter(text)
            .any(|c| !is_reference_or_placeholder(&c[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_credentials() {
        for sample in [
            "token ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8",
            "postgres://app:Pr0dPassw0rd@db.internal/app",
            "AWS key ASIAABCDEFGHIJKLMNOP",
            "DB_PASSWORD=hunter2hunter2",
            "Bearer apikey_0123456789abcdef0123456789abcdef01234567_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "sk-proj-a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8",
            // A placeholder earlier on the line does not hide a real value later on it.
            "API_KEY=${KEY} DB_PASSWORD=hunter2hunter2",
            "password: changeme123",
        ] {
            assert!(contains_secret(sample), "missed: {sample}");
        }
    }

    #[test]
    fn ignores_ordinary_text() {
        for sample in [
            "commit 3ce98c1a8b7f2d4e5c6b7a8d9e0f1a2b3c4d5e6f",
            "the password field should be hashed with argon2",
            "https://example.com/docs/page",
            "uuid 123e4567-e89b-12d3-a456-426614174000",
            // References and placeholders name where a secret lives; they are not secrets.
            "DB_PASSWORD=vault:secret/db/prod",
            "API_KEY=${TYPESAFE_API_KEY}",
            "api_key: <redacted>",
            "DB_PASSWORD=$DB_PASSWORD_FROM_ENV",
            "access_token: ********",
            "task sk-learn-model-selection-cross-validation-guide",
        ] {
            assert!(!contains_secret(sample), "false positive: {sample}");
        }
    }

    #[test]
    fn linear_time_on_adversarial_input() {
        let text = "token-".repeat(20_000);
        let started = std::time::Instant::now();
        let _ = contains_secret(&text);
        assert!(started.elapsed().as_secs() < 2);
    }
}
