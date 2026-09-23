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

/// Words that make a lowercase word chain read as a description of where or how a secret is
/// kept (`server-only`, `hashed_with_argon2id`, `configured-in-env`, `quarterly-via-vault`,
/// `hostlelo-whmcs-creds`) rather than as the secret itself. A chain with none of them
/// (`correct-horse-battery-staple`, `qwerty_asdf`, `admin_pass`) is a passphrase: refused.
const DESCRIBING_WORDS: &[&str] = &[
    "as",
    "at",
    "by",
    "for",
    "from",
    "in",
    "into",
    "of",
    "on",
    "only",
    "via",
    "with",
    "without",
    "not",
    "never",
    "no",
    "see",
    "set",
    "stored",
    "hashed",
    "hash",
    "encrypted",
    "env",
    "environment",
    "vault",
    "kms",
    "managed",
    "manager",
    "rotated",
    "rotation",
    "redacted",
    "masked",
    "configured",
    "mounted",
    "provided",
    "loaded",
    "injected",
    "server",
    "client",
    "side",
    "runtime",
    "config",
    "settings",
    "dashboard",
    "creds",
    "credentials",
    "placeholder",
    "unset",
    "none",
    "empty",
    "required",
    "daily",
    "weekly",
    "monthly",
    "quarterly",
    "yearly",
    "annually",
];

/// An assignment value that describes a secret instead of being one:
/// - a chain of two or more lowercase words joined by `-` or `_` that contains a describing
///   word (`server-only`, `hashed_with_argon2id`, `hostlelo-whmcs-creds`; see
///   [`DESCRIBING_WORDS`]);
/// - words ending in an event word and an ISO year-month or date (`rotated-2026-09`,
///   `key-issued-2026-09-01`): the word just before the date must end in `ed`;
/// - an environment variable *name* (`TYPESAFE_API_KEY`).
///
/// A word never ends in a digit: at most one run of digits sits between its letters
/// (`argon2id`). So `pass123`, `secret1`, `hunter2` or `PASS1` anywhere in the value makes it
/// count as a credential, and so do `sha256` and `oauth2`. The first word is letters only, at
/// least two of them, so random tokens (`ts_live_f9a8b7c6`) and `p_assw0rd` still count as
/// values. So do a single word (`changeme123`, `princess`), a word plus a bare number
/// (`summer-2024`), words plus a date with no event word (`admin-pass-2024-09`), and a word
/// chain with no describing word (`correct-horse-battery-staple`, `qwerty_asdf`, `admin_pass`),
/// because common human passwords and passphrases look like that.
fn is_description(value: &str) -> bool {
    static CHAIN: OnceLock<Regex> = OnceLock::new();
    static OTHER: OnceLock<Regex> = OnceLock::new();
    let first = "[a-z]{2,}";
    let word = "[a-z]+(?:[0-9]+[a-z]+)?";
    let chain = CHAIN.get_or_init(|| {
        Regex::new(&format!("^{first}(?:[-_]{word})+$")).expect("description chain regex")
    });
    let other = OTHER.get_or_init(|| {
        let date = "[0-9]{4}-[0-9]{2}(?:-[0-9]{2})?";
        let name_first = "[A-Z]{2,}(?:[0-9]+[A-Z]+)?";
        let name_word = "[A-Z]+(?:[0-9]+[A-Z]+)?";
        Regex::new(&format!(
            "^(?:(?:{first}(?:[-_]{word})*[-_])?[a-z]+ed[-_]{date}|{name_first}(?:_{name_word})+)$"
        ))
        .expect("description regex")
    });
    // Sentence punctuation after the value is not part of it.
    let v = value.trim_end_matches(['.', '!', '?', ':', ')']);
    other.is_match(v)
        || (chain.is_match(v) && v.split(['-', '_']).any(|w| DESCRIBING_WORDS.contains(&w)))
}

/// True when `text` contains something shaped like a credential.
pub fn contains_secret(text: &str) -> bool {
    patterns().is_match(text)
        || sk_key()
            .find_iter(text)
            .any(|m| m.as_str().bytes().any(|b| b.is_ascii_digit()))
        || assignment()
            .captures_iter(text)
            .any(|c| !is_reference_or_placeholder(&c[1]) && !is_description(&c[1]))
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
            // Values still count when they look like human passwords or random tokens, even
            // next to a description on the same line.
            "wifi password=summer-2024",
            "DB_PASSWORD=princess1",
            "admin password: princesses",
            "client_secret=Rotated-2026-09",
            "client_secret=x9K2pQ7vR4mT8wZ1",
            "TYPESAFE_API_KEY=server-only DB_PASSWORD=hunter2hunter2",
            "API_KEY=ts_live_4f9a8b7c6d5e4f3a",
            "API_KEY=ts_live_f9a8b7c6d5e4f3a1",
            "DB_PASSWORD=X9K2P0QZ7TRM_AB12CD34",
            // Words plus digits, and words plus a date, are human passwords, not descriptions.
            // Each was stored by an earlier version of the description rule (review, 2026-09).
            "DB_PASSWORD=admin_pass123",
            "password=hunter_hunter2",
            "wifi password=welcome-home1",
            "mysql root password: super-secret1",
            "password=summer_fun2024",
            "password=p_assw0rd",
            "api_key=ADMIN_PASS1",
            "ADMIN_PASSWORD=admin-pass-2024-09",
            "client_secret=x_rotated-2026-09",
        ] {
            assert!(contains_secret(sample), "missed: {sample}");
        }
    }

    #[test]
    fn passphrase_like_word_chains_are_credentials() {
        // A lowercase word chain with no describing word is a passphrase, not a description.
        for sample in [
            "db_password=qwerty_asdf",
            "password=correct-horse-battery-staple",
            "admin password: admin_pass",
            "password=my_p4ssw0rd",
            "wifi password=blue-elephant-sunrise",
        ] {
            assert!(contains_secret(sample), "missed: {sample}");
        }
    }

    #[test]
    fn configuration_notes_are_not_credentials() {
        // Refused by `remember` before descriptions were recognised (HostLelo review, 2026-09).
        for sample in [
            "TYPESAFE_API_KEY=server-only, never shipped to the client",
            "Kubernetes secret: hostlelo-whmcs-creds is mounted into the pod",
            "password=hashed_with_argon2id before storage",
            "The WHMCS api_key: configured-in-env",
            "client_secret=rotated-2026-09 in the vault",
            // Environment variable names are names, not values.
            "Set TYPESAFE_API_KEY in .env; the hook reads it server-side only.",
            "apiKeyEnv: TYPESAFE_API_KEY",
            "\"secret_name\": \"WHMCS_API_SECRET\"",
            "Key rotation note: api_key_rotation=quarterly-via-vault, last rotated 2026-09-01.",
            "The WHMCS api_key: configured-in-env.",
            "client_secret=key-issued-2026-09-01 by the platform team",
            "password=stored_as_argon2id_hash",
        ] {
            assert!(!contains_secret(sample), "false positive: {sample}");
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
