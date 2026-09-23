//! Append-only, hash-chained TraceWeave ledger.
//!
//! Format history:
//! - v1 (≤ 3.0.0): `hash = blake3(json{seq, prev_hash, payload})` where the payload is the
//!   *re-serialized* Rust struct. Still verified for backward compatibility.
//! - v2 (3.1.0+): `hash = blake3("hikmah-ledger-v2\n" seq "\n" prev_hash "\n" payload_bytes)`
//!   over the exact payload bytes stored on disk, so schema growth never breaks old hashes and
//!   any injected byte inside the payload is detected.
//!
//! Writes validate every event against current state *before* anything touches disk, hold an
//! exclusive lock on a `<store>.lock` sidecar (never on the ledger itself: Windows locks are
//! mandatory and would block lock-free readers), re-read records other processes appended since this store was opened,
//! and write each batch with a single `write_all`. A torn final line (crash mid-write) is ignored
//! on read and truncated by the next writer. A `<store>.head` file records the latest
//! `{seq, hash}`: it is read before the ledger (so a concurrent writer cannot cause a false
//! alarm), checked under the lock before every write (so a write cannot paper over a truncation, a
//! rewrite, or records appended without a head update), and can be pinned externally through
//! `verify_report(Some(head))`. `accept_tail` acknowledges appended records after inspection;
//! `reset_head` accepts the current ledger after a deliberate repair. Both are a person's
//! decision, and the CLI refuses them inside a detected AI agent session (see [`crate::principal`]).
//!
//! The chain is unkeyed: anyone who can write the ledger can compute valid hashes. The chain alone
//! detects a record edited, reordered, or injected inside it. The head file adds truncation,
//! rewrites, and records appended without a head update. Neither detects an append or a re-chain
//! by someone who can also update or delete the head file; only a head pinned outside the writer's
//! reach (`verify_report(Some(head))`) does.
use crate::claims::{detect_conflicts, ClaimConflict};
use crate::error::{KernelError, Result};
use crate::policy::KernelPolicy;
use crate::trace::{PrivacyClass, Trace, TraceKind, TraceStatus};
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const GENESIS: &str = "GENESIS";
const LEDGER_VERSION: u8 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerPayload {
    Remember { trace: Box<Trace> },
    Supersede { old_id: String, new_id: String },
    Fulfill { id: String },
    Purge { id: String, reason: String },
}

/// One persisted record as it sits on disk (payload kept as exact bytes).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecord<'a> {
    seq: u64,
    prev_hash: String,
    #[serde(default)]
    v: Option<u8>,
    #[serde(borrow)]
    payload: &'a RawValue,
    hash: String,
}

#[derive(Serialize)]
struct HashMaterialV1<'a> {
    seq: u64,
    prev_hash: &'a str,
    payload: &'a LedgerPayload,
}

/// In-memory view of a verified record.
#[derive(Debug, Clone)]
pub struct LedgerRecord {
    pub seq: u64,
    pub prev_hash: String,
    pub hash: String,
    pub version: u8,
    payload_json: String,
}

impl LedgerRecord {
    pub fn payload(&self) -> Result<LedgerPayload> {
        Ok(serde_json::from_str(&self.payload_json)?)
    }
}

#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub trace: Trace,
    pub status: TraceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LedgerHead {
    pub seq: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    /// Chain, sequence numbers, hashes, and head checks all passed (`errors` is empty).
    pub ok: bool,
    pub records: usize,
    pub head: Option<LedgerHead>,
    /// `true` when a `<store>.head` file exists and matched the chain.
    pub head_file_checked: bool,
    /// Integrity findings; any entry makes `ok` false. Records removed or rewritten, an unreadable
    /// head file, records past the head that no write acknowledged, a pinned head mismatch.
    pub errors: Vec<String>,
    /// Non-integrity problems: events that could not be applied, a torn tail, no head file.
    pub warnings: Vec<String>,
    /// Records past the head file that no hikmah write acknowledged (appended by another program,
    /// an older binary, or left by a crash before the head update). Writes are refused until they
    /// are inspected and accepted with [`MemoryStore::accept_tail`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unacknowledged: Vec<RecordSummary>,
}

/// One ledger record, summarized for a person to inspect before accepting it. Free text is cut to
/// a short preview, and text that looks like a credential is withheld.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecordSummary {
    pub seq: u64,
    /// `remember`, `supersede`, `fulfill`, or `purge`.
    pub event: String,
    /// The trace the event writes or changes.
    pub trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    /// `claim_key=claim_value`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<String>,
    /// Content preview, purge reason, or the superseding trace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl RecordSummary {
    fn of(record: &LedgerRecord) -> Self {
        let mut summary = Self {
            seq: record.seq,
            event: "unreadable".into(),
            trace_id: String::new(),
            kind: None,
            source: None,
            verified: None,
            claim: None,
            detail: None,
        };
        let Ok(payload) = record.payload() else {
            return summary;
        };
        match payload {
            LedgerPayload::Remember { trace } => {
                summary.event = "remember".into();
                summary.trace_id = preview(&trace.id);
                summary.kind = Some(trace.kind.to_string());
                summary.source = Some(preview(&trace.provenance.source));
                summary.verified = Some(trace.provenance.verified);
                summary.claim = trace
                    .claim_key
                    .as_ref()
                    .zip(trace.claim_value.as_ref())
                    .map(|(key, value)| preview(&format!("{key}={value}")));
                summary.detail = Some(preview(&trace.content));
            }
            LedgerPayload::Supersede { old_id, new_id } => {
                summary.event = "supersede".into();
                summary.trace_id = preview(&old_id);
                summary.detail = Some(format!("superseded by {}", preview(&new_id)));
            }
            LedgerPayload::Fulfill { id } => {
                summary.event = "fulfill".into();
                summary.trace_id = preview(&id);
            }
            LedgerPayload::Purge { id, reason } => {
                summary.event = "purge".into();
                summary.trace_id = preview(&id);
                summary.detail = Some(preview(&reason));
            }
        }
        summary
    }
}

impl std::fmt::Display for RecordSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "seq {} {} {}", self.seq, self.event, self.trace_id)?;
        let mut facts = Vec::new();
        facts.extend(self.kind.clone());
        facts.extend(self.source.as_ref().map(|s| format!("source {s}")));
        if self.verified == Some(true) {
            facts.push("verified".into());
        }
        facts.extend(self.claim.as_ref().map(|c| format!("claim {c}")));
        if !facts.is_empty() {
            write!(f, " ({})", facts.join(", "))?;
        }
        if let Some(detail) = &self.detail {
            write!(f, ": {detail:?}")?;
        }
        Ok(())
    }
}

/// What [`MemoryStore::accept_tail`] acknowledged.
#[derive(Debug, Clone, Serialize)]
pub struct TailAcceptance {
    pub accepted: Vec<RecordSummary>,
    pub head: Option<LedgerHead>,
}

const PREVIEW_CHARS: usize = 80;
/// At most this many unacknowledged records are listed in a refusal message.
const LISTED_IN_ERRORS: usize = 10;

/// A single-line, bounded preview of free text that never echoes something shaped like a credential.
fn preview(text: &str) -> String {
    if crate::secrets::contains_secret(text) {
        return "[withheld: looks like a credential]".into();
    }
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    match flat.char_indices().nth(PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}...", &flat[..cut]),
        None => flat,
    }
}

fn list_records(summaries: &[RecordSummary]) -> String {
    let mut listed: Vec<String> = summaries
        .iter()
        .take(LISTED_IN_ERRORS)
        .map(ToString::to_string)
        .collect();
    if summaries.len() > LISTED_IN_ERRORS {
        listed.push(format!("and {} more", summaries.len() - LISTED_IN_ERRORS));
    }
    listed.join("; ")
}

/// The `<store>.head` file as read *before* the ledger (so a concurrent writer can never make
/// the snapshot look ahead of the records it is compared with).
#[derive(Debug, Clone, PartialEq)]
enum HeadState {
    Missing,
    Present(LedgerHead),
    Unreadable(String),
}

#[derive(Debug)]
pub struct MemoryStore {
    path: PathBuf,
    policy: KernelPolicy,
    records: Vec<LedgerRecord>,
    traces: BTreeMap<String, TraceEntry>,
    /// Bytes of complete, replayed lines.
    offset: u64,
    torn_tail_bytes: u64,
    missing_final_newline: bool,
    replay_issues: Vec<String>,
    head_at_load: HeadState,
    /// When set, a write fails at once if another process holds the writer lock instead of
    /// waiting for it (see [`Self::set_nonblocking_writes`]).
    nonblocking_writes: bool,
}

impl MemoryStore {
    /// Open a store, creating the file (and parent directories) when it does not exist.
    /// Use this for commands that write. Read-only commands should use [`Self::open_existing`].
    pub fn open(path: impl AsRef<Path>, policy: KernelPolicy) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        // Never truncate: `File::create` would race with a concurrent creator and erase its record.
        OpenOptions::new().create(true).append(true).open(&path)?;
        Self::load(path, policy)
    }

    /// Open a store that must already exist. A mistyped path is an error, not an empty store.
    pub fn open_existing(path: impl AsRef<Path>, policy: KernelPolicy) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(KernelError::NotFound(format!(
                "memory store {} (run `hikmah init` first)",
                path.display()
            )));
        }
        Self::load(path, policy)
    }

    fn load(path: PathBuf, policy: KernelPolicy) -> Result<Self> {
        let mut store = Self {
            path,
            policy,
            records: Vec::new(),
            traces: BTreeMap::new(),
            offset: 0,
            torn_tail_bytes: 0,
            missing_final_newline: false,
            replay_issues: Vec::new(),
            head_at_load: HeadState::Missing,
            nonblocking_writes: false,
        };
        // Read the head before the ledger: heads are written only after their records are
        // synced, so this snapshot can never be ahead of what we are about to read.
        store.head_at_load = read_head(&store.head_path());
        let bytes = fs::read(&store.path)?;
        store.replay_bytes(&bytes)?;
        Ok(store)
    }

    /// Replay complete lines of `chunk` (which starts at `self.offset`). Works on bytes so a
    /// torn final line that ends inside a multi-byte character is still treated as a torn tail.
    fn replay_bytes(&mut self, chunk: &[u8]) -> Result<()> {
        let mut consumed = 0_usize;
        let mut rest = chunk;
        while let Some(pos) = rest.iter().position(|&b| b == b'\n') {
            let line = std::str::from_utf8(&rest[..pos]).map_err(|_| KernelError::Integrity {
                seq: self.records.len() as u64 + 1,
                message: "ledger record is not valid UTF-8".into(),
            })?;
            self.replay_line(line)?;
            consumed += pos + 1;
            rest = &rest[pos + 1..];
        }
        self.offset += consumed as u64;
        self.torn_tail_bytes = 0;
        self.missing_final_newline = false;
        if !rest.is_empty() {
            // A final line without '\n'. Either a complete record whose newline never landed
            // (older binaries wrote record and newline in two syscalls) or a torn write.
            match std::str::from_utf8(rest) {
                Ok(text) if serde_json::from_str::<RawRecord>(text).is_ok() => {
                    self.replay_line(text)?;
                    self.offset += rest.len() as u64;
                    self.missing_final_newline = true;
                }
                // Only JSON whitespace may be kept in front of the next record.
                Ok(text) if text.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r')) => {
                    self.offset += rest.len() as u64;
                }
                _ => self.torn_tail_bytes = rest.len() as u64,
            }
        }
        Ok(())
    }

    fn replay_line(&mut self, line: &str) -> Result<()> {
        if line.trim().is_empty() {
            return Ok(());
        }
        let expected_seq = self.records.len() as u64 + 1;
        let expected_prev = self
            .records
            .last()
            .map(|r| r.hash.clone())
            .unwrap_or_else(|| GENESIS.to_string());
        let raw: RawRecord =
            serde_json::from_str(line).map_err(|error| KernelError::Integrity {
                seq: expected_seq,
                message: format!("unparseable ledger record: {error}"),
            })?;
        if raw.seq != expected_seq {
            return Err(KernelError::Integrity {
                seq: raw.seq,
                message: format!("expected sequence {expected_seq}"),
            });
        }
        if raw.prev_hash != expected_prev {
            return Err(KernelError::Integrity {
                seq: raw.seq,
                message: "previous hash does not match".into(),
            });
        }
        let version = raw.v.unwrap_or(1);
        let payload_json = raw.payload.get().to_string();
        let payload: LedgerPayload =
            serde_json::from_str(&payload_json).map_err(|error| KernelError::Integrity {
                seq: raw.seq,
                message: format!("unparseable payload: {error}"),
            })?;
        let calculated = match version {
            1 => hash_v1(raw.seq, &raw.prev_hash, &payload)?,
            2 => hash_v2(raw.seq, &raw.prev_hash, &payload_json),
            other => {
                return Err(KernelError::Integrity {
                    seq: raw.seq,
                    message: format!("unsupported ledger record version {other}"),
                })
            }
        };
        if calculated != raw.hash {
            return Err(KernelError::Integrity {
                seq: raw.seq,
                message: "record hash does not match payload".into(),
            });
        }
        // A hash-valid record that cannot be applied (for example a supersede of an unknown id
        // written by an older binary) is a logic problem, not tampering: keep the store usable.
        if let Err(issue) = apply_payload(&mut self.traces, &payload) {
            self.replay_issues
                .push(format!("seq {}: {issue} (event skipped)", raw.seq));
        }
        self.records.push(LedgerRecord {
            seq: raw.seq,
            prev_hash: raw.prev_hash,
            hash: raw.hash,
            version,
            payload_json,
        });
        Ok(())
    }

    pub fn remember(&mut self, mut trace: Trace) -> Result<(Trace, Vec<ClaimConflict>)> {
        trace.validate()?;
        if trace.privacy == PrivacyClass::Sensitive && !self.policy.allow_sensitive_persistence {
            return Err(KernelError::Invalid(
                "sensitive persistence is disabled; use an encrypted vault adapter or lower the privacy class explicitly"
                    .into(),
            ));
        }
        if trace.id.is_empty() {
            trace.id = self.next_trace_id(&trace);
        }
        trace.validate()?;
        if self.traces.contains_key(&trace.id) {
            return Err(KernelError::Invalid(format!(
                "trace id already exists: {}",
                trace.id
            )));
        }
        if let Some(old_id) = &trace.supersedes {
            let target = self
                .traces
                .get(old_id)
                .ok_or_else(|| KernelError::NotFound(old_id.clone()))?;
            if target.status != TraceStatus::Active {
                return Err(KernelError::Invalid(format!(
                    "cannot supersede {old_id}: it is {:?}, not active",
                    target.status
                )));
            }
            if trace.is_model_authored() {
                return Err(KernelError::Invalid(
                    "model-authored traces cannot supersede other traces".into(),
                ));
            }
            if target.trace.provenance.verified && !trace.provenance.verified {
                return Err(KernelError::Invalid(format!(
                    "cannot supersede verified trace {old_id} with an unverified one; record a conflicting claim instead"
                )));
            }
        }
        if let Some(outcome) = &trace.outcome {
            let prediction = self
                .traces
                .get(&outcome.prediction_id)
                .ok_or_else(|| KernelError::NotFound(outcome.prediction_id.clone()))?;
            if prediction.trace.kind != TraceKind::Prediction {
                return Err(KernelError::Invalid(format!(
                    "{} is not a prediction trace",
                    outcome.prediction_id
                )));
            }
            if prediction.status != TraceStatus::Active {
                return Err(KernelError::Invalid(format!(
                    "cannot record an outcome for {}: the prediction is {:?}, not active, and no longer counts toward calibration",
                    outcome.prediction_id, prediction.status
                )));
            }
            if let Some(record) = &prediction.trace.prediction {
                let observed = outcome.observed.trim();
                if !record.answer_space.is_empty()
                    && !record.answer_space.iter().any(|v| v == observed)
                {
                    return Err(KernelError::Invalid(format!(
                        "observed value `{observed}` is not one of {:?}",
                        record.answer_space
                    )));
                }
            }
        }

        let superseded = trace.supersedes.clone();
        let conflicts = detect_conflicts(
            &trace,
            self.active_traces()
                .filter(|existing| Some(&existing.id) != superseded.as_ref()),
        );
        let mut batch = vec![LedgerPayload::Remember {
            trace: Box::new(trace.clone()),
        }];
        if let Some(old_id) = superseded {
            batch.push(LedgerPayload::Supersede {
                old_id,
                new_id: trace.id.clone(),
            });
        }
        self.append_batch(batch)?;
        Ok((trace, conflicts))
    }

    pub fn fulfill(&mut self, id: impl Into<String>) -> Result<()> {
        self.append_batch(vec![LedgerPayload::Fulfill { id: id.into() }])
    }

    /// Tombstone a trace: it leaves recall, consolidation, and commitments. The content stays in
    /// the append-only ledger; this is not erasure.
    pub fn purge(&mut self, id: impl Into<String>, reason: impl Into<String>) -> Result<()> {
        self.append_batch(vec![LedgerPayload::Purge {
            id: id.into(),
            reason: reason.into(),
        }])
    }

    pub fn get(&self, id: &str) -> Option<&TraceEntry> {
        self.traces.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &TraceEntry> {
        self.traces.values()
    }

    pub fn active_traces(&self) -> impl Iterator<Item = &Trace> {
        self.traces
            .values()
            .filter(|entry| entry.status == TraceStatus::Active)
            .map(|entry| &entry.trace)
    }

    pub fn policy(&self) -> &KernelPolicy {
        &self.policy
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    pub fn records(&self) -> &[LedgerRecord] {
        &self.records
    }

    pub fn head(&self) -> Option<LedgerHead> {
        self.records.last().map(|r| LedgerHead {
            seq: r.seq,
            hash: r.hash.clone(),
        })
    }

    pub fn head_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".head");
        PathBuf::from(name)
    }

    /// Integrity check. Errors on any chain break; returns `Ok(())` only when the report is ok.
    pub fn verify(&self) -> Result<()> {
        let report = self.verify_report(None)?;
        if report.ok {
            Ok(())
        } else {
            Err(KernelError::Integrity {
                seq: report.records as u64,
                message: report.errors.join("; "),
            })
        }
    }

    /// Full verification report. `expected_head` pins the latest hash (for example a value kept
    /// in CI or in git) so a wholesale re-chain by someone with write access is detected. When the
    /// loaded ledger holds records past its head snapshot, this waits for in-flight writes (a
    /// shared lock on the `<store>.lock` sidecar) before reporting them as unacknowledged.
    pub fn verify_report(&self, expected_head: Option<&str>) -> Result<VerifyReport> {
        let mut prev = GENESIS.to_string();
        for record in &self.records {
            if record.prev_hash != prev {
                return Err(KernelError::Integrity {
                    seq: record.seq,
                    message: "previous hash mismatch".into(),
                });
            }
            let calculated = match record.version {
                1 => hash_v1(record.seq, &record.prev_hash, &record.payload()?)?,
                _ => hash_v2(record.seq, &record.prev_hash, &record.payload_json),
            };
            if calculated != record.hash {
                return Err(KernelError::Integrity {
                    seq: record.seq,
                    message: "hash mismatch".into(),
                });
            }
            prev = record.hash.clone();
        }

        let mut warnings = self.replay_issues.clone();
        if self.torn_tail_bytes > 0 {
            warnings.push(format!(
                "torn final line of {} bytes ignored (the next write repairs it)",
                self.torn_tail_bytes
            ));
        }
        let head = self.head();
        let mut errors = Vec::new();
        let mut unacknowledged = Vec::new();
        let mut head_file_checked = false;
        match &self.head_at_load {
            HeadState::Missing => {
                if !self.records.is_empty() {
                    warnings.push(
                        "no head file: truncation and appended records cannot be detected until a write (or `hikmah verify-ledger --reset-head`) creates one".into(),
                    );
                }
            }
            HeadState::Unreadable(error) => {
                errors.push(format!(
                    "head file is unreadable ({error}); a person must inspect it, then run `hikmah verify-ledger --reset-head` from their own terminal to accept the current ledger"
                ));
            }
            HeadState::Present(stored) => {
                head_file_checked = true;
                let count = self.records.len() as u64;
                if stored.seq > count {
                    errors.push(format!(
                        "ledger has {count} records but its head file records seq {}: records were removed",
                        stored.seq
                    ));
                } else if stored.seq > 0
                    && self.records[(stored.seq - 1) as usize].hash != stored.hash
                {
                    errors.push(format!(
                        "record {} does not match the head file hash: the ledger was rewritten",
                        stored.seq
                    ));
                } else if stored.seq < count {
                    let acknowledged = self.acknowledged_now(stored.seq);
                    if acknowledged < count {
                        unacknowledged = self.summaries_after(acknowledged);
                        errors.push(format!(
                            "{} record(s) after the recorded head (seq {acknowledged}) were never acknowledged by a hikmah write: appended by another program, written by an older binary, or left by a crash before the head update. Writes are refused until a person inspects them and runs `hikmah verify-ledger --accept-tail` from their own terminal",
                            count - acknowledged
                        ));
                    }
                }
            }
        }
        if let Some(expected) = expected_head {
            let actual = head.as_ref().map(|h| h.hash.as_str()).unwrap_or(GENESIS);
            if actual != expected {
                errors.push(format!(
                    "head hash {actual} does not match the pinned head {expected}"
                ));
            }
        }
        Ok(VerifyReport {
            ok: errors.is_empty(),
            records: self.records.len(),
            head,
            head_file_checked,
            errors,
            warnings,
            unacknowledged,
        })
    }

    /// Summaries of the records after `seq` (for inspection before [`Self::accept_tail`]).
    fn summaries_after(&self, seq: u64) -> Vec<RecordSummary> {
        self.records
            .iter()
            .skip(seq as usize)
            .map(RecordSummary::of)
            .collect()
    }

    /// A lock-free reader can load the ledger while a write is in flight: the writer syncs its
    /// records before it updates the head, so this snapshot may hold records its head snapshot
    /// (`loaded_seq`) does not cover yet. Only in that case, wait for in-flight writes to finish (a
    /// shared lock on the `<store>.lock` sidecar, which every writer holds exclusively until its
    /// head update has landed), then re-read the head and, if it has moved past this snapshot, the
    /// ledger. Returns how many leading records of this snapshot the head acknowledges now: all of
    /// them when every write completed, the new head's seq when it moved only part of the way (so
    /// a legitimate record is never listed beside a forged one), and `loaded_seq` when the head
    /// cannot be confirmed.
    fn acknowledged_now(&self, loaded_seq: u64) -> u64 {
        let n = self.records.len() as u64;
        let Some(last) = self.records.last() else {
            return n;
        };
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path())
            .or_else(|_| File::open(self.lock_path()));
        // Without the lock an in-flight write cannot be told apart from an unacknowledged tail;
        // report the tail rather than guess.
        let Ok(lock) = lock else {
            return loaded_seq;
        };
        if lock.lock_shared().is_err() {
            return loaded_seq;
        }
        match read_head(&self.head_path()) {
            HeadState::Present(now) if now.seq == n && now.hash == last.hash => n,
            HeadState::Present(now) if now.seq > n => {
                // Confirm this snapshot is a prefix of the ledger that the newer head covers.
                let Ok(fresh) = Self::load(self.path.clone(), self.policy.clone()) else {
                    return loaded_seq;
                };
                let covered = match &fresh.head_at_load {
                    HeadState::Present(head) => {
                        head.seq >= n
                            && head.seq <= fresh.records.len() as u64
                            && fresh.records[(head.seq - 1) as usize].hash == head.hash
                            && fresh.records[(n - 1) as usize].hash == last.hash
                    }
                    _ => false,
                };
                if covered {
                    n
                } else {
                    loaded_seq
                }
            }
            HeadState::Present(now)
                if now.seq > loaded_seq
                    && now.seq < n
                    && self.records[(now.seq - 1) as usize].hash == now.hash =>
            {
                now.seq
            }
            _ => loaded_seq,
        }
    }

    fn append_batch(&mut self, payloads: Vec<LedgerPayload>) -> Result<()> {
        if payloads.is_empty() {
            return Ok(());
        }
        let (_lock, mut file) = self.open_locked()?;
        self.sync_tail(&mut file)?;
        self.check_head_before_write()?;
        validate_batch(&self.traces, &payloads)?;

        let mut buffer = String::new();
        let mut staged = Vec::with_capacity(payloads.len());
        let mut prev = self
            .records
            .last()
            .map(|r| r.hash.clone())
            .unwrap_or_else(|| GENESIS.to_string());
        let mut seq = self.records.len() as u64;
        for payload in &payloads {
            seq += 1;
            let payload_json = serde_json::to_string(payload)?;
            let hash = hash_v2(seq, &prev, &payload_json);
            buffer.push_str(&format!(
                "{{\"seq\":{seq},\"prev_hash\":{},\"v\":{LEDGER_VERSION},\"payload\":{payload_json},\"hash\":\"{hash}\"}}\n",
                serde_json::to_string(&prev)?
            ));
            staged.push(LedgerRecord {
                seq,
                prev_hash: prev.clone(),
                hash: hash.clone(),
                version: LEDGER_VERSION,
                payload_json,
            });
            prev = hash;
        }
        file.seek(SeekFrom::End(0))?;
        file.write_all(buffer.as_bytes())?;
        file.sync_data()?;
        for payload in &payloads {
            apply_payload(&mut self.traces, payload).map_err(KernelError::Invalid)?;
        }
        self.records.extend(staged);
        self.offset += buffer.len() as u64;
        self.write_head()?;
        Ok(())
    }

    /// Under the write lock: pick up records other processes appended, repair a torn tail.
    fn sync_tail(&mut self, file: &mut File) -> Result<()> {
        let len = file.metadata()?.len();
        if len < self.offset {
            return Err(KernelError::Integrity {
                seq: self.records.len() as u64,
                message: "ledger is shorter than when it was opened".into(),
            });
        }
        if len > self.offset {
            file.seek(SeekFrom::Start(self.offset))?;
            let mut tail = Vec::new();
            file.read_to_end(&mut tail)?;
            self.replay_bytes(&tail)?;
        }
        if self.torn_tail_bytes > 0 {
            // Truncating a torn record is a repair; truncating a file that was never a ledger is
            // data loss. With no valid record, only cut bytes that begin like a ledger record.
            if self.records.is_empty() {
                let mut start = [0_u8; 7];
                file.seek(SeekFrom::Start(self.offset))?;
                let read = file.read(&mut start)?;
                let signature: &[u8] = b"{\"seq\"";
                let begins_like_record =
                    start[..read].starts_with(signature) || signature.starts_with(&start[..read]);
                if !begins_like_record {
                    return Err(KernelError::Integrity {
                        seq: 0,
                        message: format!(
                            "{} does not look like a Hikmah ledger (no valid record, and its content does not start like one); refusing to write to it",
                            self.path.display()
                        ),
                    });
                }
            }
            file.set_len(self.offset)?;
            self.replay_issues.push(format!(
                "repaired a torn final line of {} bytes",
                self.torn_tail_bytes
            ));
            self.torn_tail_bytes = 0;
        }
        if self.missing_final_newline {
            file.seek(SeekFrom::End(0))?;
            file.write_all(b"\n")?;
            self.offset += 1;
            self.missing_final_newline = false;
        }
        Ok(())
    }

    /// Under the write lock: refuse to append when the head file shows the ledger was truncated or
    /// rewritten, or holds records no hikmah write acknowledged, so an ordinary write can never
    /// paper over tamper evidence or silently approve a forged append. Every completed write
    /// updates the head before it releases the lock, so a head behind the ledger here is never
    /// another writer in flight.
    fn check_head_before_write(&self) -> Result<()> {
        let count = self.records.len() as u64;
        match read_head(&self.head_path()) {
            HeadState::Missing => Ok(()),
            HeadState::Unreadable(error) => Err(KernelError::Integrity {
                seq: count,
                message: format!(
                    "head file is unreadable ({error}); a person must inspect the ledger, then run `hikmah verify-ledger --reset-head` from their own terminal"
                ),
            }),
            HeadState::Present(head) => {
                let rewritten = head.seq > count
                    || (head.seq > 0 && self.records[(head.seq - 1) as usize].hash != head.hash);
                if rewritten {
                    Err(KernelError::Integrity {
                        seq: head.seq,
                        message: "the ledger no longer matches its head file (records removed or rewritten); refusing to write. A person must inspect it, then run `hikmah verify-ledger --reset-head` from their own terminal to accept the current ledger".into(),
                    })
                } else if head.seq < count {
                    Err(KernelError::Integrity {
                        seq: head.seq + 1,
                        message: format!(
                            "{} record(s) after the recorded head (seq {}) were never acknowledged by a hikmah write (appended by another program, written by an older binary, or left by a crash before the head update); refusing to write so they are not silently accepted. Unacknowledged: {}. A person must inspect them, then run `hikmah verify-ledger --accept-tail` from their own terminal to accept them (it is refused inside an AI agent session)",
                            count - head.seq,
                            head.seq,
                            list_records(&self.summaries_after(head.seq))
                        ),
                    })
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Acknowledge records appended after the head file (after inspecting them): the head moves
    /// to the end of the ledger and writes are allowed again. Unlike [`Self::reset_head`] this
    /// accepts only appended records. It refuses when records were removed or rewritten, when the
    /// head file is unreadable, and when there is no head file to extend.
    pub fn accept_tail(&mut self) -> Result<TailAcceptance> {
        let (_lock, mut file) = self.open_locked()?;
        self.sync_tail(&mut file)?;
        let count = self.records.len() as u64;
        let head = match read_head(&self.head_path()) {
            HeadState::Present(head) => head,
            HeadState::Missing => {
                return Err(KernelError::Invalid(
                    "there is no head file, so no records are marked unacknowledged; the next write creates one (or run `hikmah verify-ledger --reset-head`)".into(),
                ))
            }
            HeadState::Unreadable(error) => {
                return Err(KernelError::Integrity {
                    seq: count,
                    message: format!(
                        "head file is unreadable ({error}); a person must inspect the ledger, then run `hikmah verify-ledger --reset-head` from their own terminal"
                    ),
                })
            }
        };
        if head.seq > count
            || (head.seq > 0 && self.records[(head.seq - 1) as usize].hash != head.hash)
        {
            return Err(KernelError::Integrity {
                seq: head.seq,
                message: "records before the head were removed or rewritten; accepting the tail cannot repair that. A person must inspect the ledger, then run `hikmah verify-ledger --reset-head` from their own terminal".into(),
            });
        }
        let accepted = self.summaries_after(head.seq);
        if !accepted.is_empty() {
            self.write_head()?;
        }
        Ok(TailAcceptance {
            accepted,
            head: self.head(),
        })
    }

    /// Explicitly accept the current ledger as the new head (after a deliberate repair).
    pub fn reset_head(&mut self) -> Result<Option<LedgerHead>> {
        let (_lock, mut file) = self.open_locked()?;
        self.sync_tail(&mut file)?;
        let path = self.head_path();
        if self.records.is_empty() {
            if path.exists() {
                fs::remove_file(&path)?;
            }
            self.head_at_load = HeadState::Missing;
            return Ok(None);
        }
        self.write_head()?;
        Ok(self.head())
    }

    /// Make writes fail immediately when another process holds the writer lock, instead of
    /// waiting. For callers such as the Stop hook that must never stall on a busy store.
    pub fn set_nonblocking_writes(&mut self, nonblocking: bool) {
        self.nonblocking_writes = nonblocking;
    }

    pub fn lock_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    /// Take the exclusive writer lock, then open the ledger read+write. The ledger is not opened
    /// in append mode: on Windows an append-only handle cannot `set_len` to repair a torn tail,
    /// so every write seeks to the end explicitly while the lock is held.
    fn open_locked(&self) -> Result<(File, File)> {
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path())?;
        if self.nonblocking_writes {
            match lock.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(KernelError::Invalid(format!(
                        "{} is locked by another writer",
                        self.path.display()
                    )))
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        } else {
            lock.lock()?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)?;
        Ok((lock, file))
    }

    fn write_head(&mut self) -> Result<()> {
        let Some(head) = self.head() else {
            return Ok(());
        };
        let path = self.head_path();
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        {
            let mut file = File::create(&tmp)?;
            file.write_all(&serde_json::to_vec(&head)?)?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        self.head_at_load = HeadState::Present(head);
        Ok(())
    }

    fn next_trace_id(&self, trace: &Trace) -> String {
        let seed = format!(
            "{}:{}:{}:{}",
            self.records.len() + 1,
            trace.created_at_ms,
            trace.kind,
            trace.content
        );
        let hash = blake3::hash(seed.as_bytes()).to_hex().to_string();
        format!("tr_{}", &hash[..16])
    }
}

fn read_head(path: &Path) -> HeadState {
    match fs::read(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HeadState::Missing,
        Err(error) => HeadState::Unreadable(error.to_string()),
        Ok(bytes) => match serde_json::from_slice::<LedgerHead>(&bytes) {
            Ok(head) => HeadState::Present(head),
            Err(error) => HeadState::Unreadable(error.to_string()),
        },
    }
}

fn hash_v1(seq: u64, prev_hash: &str, payload: &LedgerPayload) -> Result<String> {
    let material = HashMaterialV1 {
        seq,
        prev_hash,
        payload,
    };
    let bytes = serde_json::to_vec(&material)?;
    let mut hasher = Hasher::new();
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_v2(seq: u64, prev_hash: &str, payload_json: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"hikmah-ledger-v2\n");
    hasher.update(seq.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(prev_hash.as_bytes());
    hasher.update(b"\n");
    hasher.update(payload_json.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Check a batch against current state (plus earlier events in the same batch) before writing.
fn validate_batch(traces: &BTreeMap<String, TraceEntry>, payloads: &[LedgerPayload]) -> Result<()> {
    let mut added: BTreeMap<String, (TraceKind, TraceStatus)> = BTreeMap::new();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let lookup = |id: &str,
                  added: &BTreeMap<String, (TraceKind, TraceStatus)>|
     -> Option<(TraceKind, TraceStatus)> {
        added
            .get(id)
            .copied()
            .or_else(|| traces.get(id).map(|e| (e.trace.kind, e.status)))
    };
    for payload in payloads {
        match payload {
            LedgerPayload::Remember { trace } => {
                trace.validate()?;
                if lookup(&trace.id, &added).is_some() {
                    return Err(KernelError::Invalid(format!(
                        "trace id already exists: {}",
                        trace.id
                    )));
                }
                // Re-checked under the lock: another process may have purged the prediction
                // since `remember` looked at it.
                if let Some(outcome) = &trace.outcome {
                    let id = &outcome.prediction_id;
                    let (kind, status) =
                        lookup(id, &added).ok_or_else(|| KernelError::NotFound(id.clone()))?;
                    if kind != TraceKind::Prediction {
                        return Err(KernelError::Invalid(format!(
                            "{id} is not a prediction trace"
                        )));
                    }
                    if status != TraceStatus::Active || changed.contains(id) {
                        return Err(KernelError::Invalid(format!(
                            "cannot record an outcome for {id}: the prediction is not active"
                        )));
                    }
                }
                added.insert(trace.id.clone(), (trace.kind, TraceStatus::Active));
            }
            LedgerPayload::Supersede { old_id, new_id } => {
                let (_, status) =
                    lookup(old_id, &added).ok_or_else(|| KernelError::NotFound(old_id.clone()))?;
                if status != TraceStatus::Active || changed.contains(old_id) {
                    return Err(KernelError::Invalid(format!(
                        "cannot supersede {old_id}: it is not active"
                    )));
                }
                if lookup(new_id, &added).is_none() {
                    return Err(KernelError::NotFound(new_id.clone()));
                }
                changed.insert(old_id.clone());
            }
            LedgerPayload::Fulfill { id } => {
                let (kind, status) =
                    lookup(id, &added).ok_or_else(|| KernelError::NotFound(id.clone()))?;
                if kind != TraceKind::Commitment {
                    return Err(KernelError::Invalid(format!(
                        "only commitments can be fulfilled; {id} is a {kind}"
                    )));
                }
                if status != TraceStatus::Active || changed.contains(id) {
                    return Err(KernelError::Invalid(format!(
                        "cannot fulfill {id}: it is not active"
                    )));
                }
                changed.insert(id.clone());
            }
            LedgerPayload::Purge { id, reason } => {
                let (_, status) =
                    lookup(id, &added).ok_or_else(|| KernelError::NotFound(id.clone()))?;
                if status == TraceStatus::Purged {
                    return Err(KernelError::Invalid(format!("{id} is already purged")));
                }
                if reason.trim().is_empty() {
                    return Err(KernelError::Invalid("purge needs a reason".into()));
                }
                // Purging is what a user does after a leak; the reason must not re-leak it.
                if crate::secrets::contains_secret(reason) {
                    return Err(KernelError::Invalid(
                        "purge reason appears to contain a credential; describe the leak without quoting it".into(),
                    ));
                }
                changed.insert(id.clone());
            }
        }
    }
    Ok(())
}

fn apply_payload(
    traces: &mut BTreeMap<String, TraceEntry>,
    payload: &LedgerPayload,
) -> std::result::Result<(), String> {
    let mut set_status = |id: &str, status: TraceStatus| -> std::result::Result<(), String> {
        let entry = traces
            .get_mut(id)
            .ok_or_else(|| format!("trace not found: {id}"))?;
        entry.status = status;
        Ok(())
    };
    match payload {
        LedgerPayload::Remember { trace } => {
            traces.insert(
                trace.id.clone(),
                TraceEntry {
                    trace: (**trace).clone(),
                    status: TraceStatus::Active,
                },
            );
            Ok(())
        }
        LedgerPayload::Supersede { old_id, .. } => set_status(old_id, TraceStatus::Superseded),
        LedgerPayload::Fulfill { id } => set_status(id, TraceStatus::Fulfilled),
        LedgerPayload::Purge { id, .. } => set_status(id, TraceStatus::Purged),
    }
}
