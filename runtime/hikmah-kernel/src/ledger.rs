use crate::claims::{detect_conflicts, ClaimConflict};
use crate::error::{KernelError, Result};
use crate::policy::KernelPolicy;
use crate::trace::{Trace, TraceStatus};
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerPayload {
    Remember { trace: Trace },
    Supersede { old_id: String, new_id: String },
    Fulfill { id: String },
    Purge { id: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub seq: u64,
    pub prev_hash: String,
    pub payload: LedgerPayload,
    pub hash: String,
}

#[derive(Serialize)]
struct HashMaterial<'a> {
    seq: u64,
    prev_hash: &'a str,
    payload: &'a LedgerPayload,
}

#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub trace: Trace,
    pub status: TraceStatus,
}

#[derive(Debug)]
pub struct MemoryStore {
    path: PathBuf,
    policy: KernelPolicy,
    records: Vec<LedgerRecord>,
    traces: BTreeMap<String, TraceEntry>,
}

impl MemoryStore {
    pub fn open(path: impl AsRef<Path>, policy: KernelPolicy) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        if !path.exists() {
            File::create(&path)?;
        }

        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        let mut traces = BTreeMap::new();
        let mut expected_prev = String::from("GENESIS");
        let mut expected_seq = 1_u64;

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record: LedgerRecord = serde_json::from_str(&line)?;
            if record.seq != expected_seq {
                return Err(KernelError::Integrity {
                    seq: record.seq,
                    message: format!("expected sequence {expected_seq}"),
                });
            }
            if record.prev_hash != expected_prev {
                return Err(KernelError::Integrity {
                    seq: record.seq,
                    message: "previous hash does not match".into(),
                });
            }
            let calculated = hash_record(record.seq, &record.prev_hash, &record.payload)?;
            if calculated != record.hash {
                return Err(KernelError::Integrity {
                    seq: record.seq,
                    message: "record hash does not match payload".into(),
                });
            }
            apply_payload(&mut traces, &record.payload)?;
            expected_prev = record.hash.clone();
            expected_seq += 1;
            records.push(record);
        }

        Ok(Self {
            path,
            policy,
            records,
            traces,
        })
    }

    pub fn remember(&mut self, mut trace: Trace) -> Result<(Trace, Vec<ClaimConflict>)> {
        trace.validate()?;
        if trace.privacy == crate::trace::PrivacyClass::Sensitive
            && !self.policy.allow_sensitive_persistence
        {
            return Err(KernelError::Invalid(
                "sensitive persistence is disabled; use an encrypted vault adapter or lower the privacy class explicitly"
                    .into(),
            ));
        }
        if trace.id.is_empty() {
            trace.id = self.next_trace_id(&trace);
        }
        if self.traces.contains_key(&trace.id) {
            return Err(KernelError::Invalid(format!(
                "trace id already exists: {}",
                trace.id
            )));
        }

        let conflicts = detect_conflicts(&trace, self.active_traces());
        let payload = LedgerPayload::Remember {
            trace: trace.clone(),
        };
        self.append(payload)?;
        if let Some(old_id) = trace.supersedes.clone() {
            self.append(LedgerPayload::Supersede {
                old_id,
                new_id: trace.id.clone(),
            })?;
        }
        Ok((trace, conflicts))
    }

    pub fn fulfill(&mut self, id: impl Into<String>) -> Result<()> {
        let id = id.into();
        if !self.traces.contains_key(&id) {
            return Err(KernelError::NotFound(id));
        }
        self.append(LedgerPayload::Fulfill { id })
    }

    pub fn purge(&mut self, id: impl Into<String>, reason: impl Into<String>) -> Result<()> {
        let id = id.into();
        if !self.traces.contains_key(&id) {
            return Err(KernelError::NotFound(id));
        }
        self.append(LedgerPayload::Purge {
            id,
            reason: reason.into(),
        })
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

    pub fn verify(&self) -> Result<()> {
        let mut prev = String::from("GENESIS");
        for record in &self.records {
            if record.prev_hash != prev {
                return Err(KernelError::Integrity {
                    seq: record.seq,
                    message: "previous hash mismatch".into(),
                });
            }
            let calculated = hash_record(record.seq, &record.prev_hash, &record.payload)?;
            if calculated != record.hash {
                return Err(KernelError::Integrity {
                    seq: record.seq,
                    message: "hash mismatch".into(),
                });
            }
            prev = record.hash.clone();
        }
        Ok(())
    }

    fn append(&mut self, payload: LedgerPayload) -> Result<()> {
        let seq = self.records.len() as u64 + 1;
        let prev_hash = self
            .records
            .last()
            .map(|r| r.hash.clone())
            .unwrap_or_else(|| "GENESIS".to_string());
        let hash = hash_record(seq, &prev_hash, &payload)?;
        let record = LedgerRecord {
            seq,
            prev_hash,
            payload,
            hash,
        };
        let encoded = serde_json::to_string(&record)?;
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{encoded}")?;
        file.sync_data()?;
        apply_payload(&mut self.traces, &record.payload)?;
        self.records.push(record);
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

fn hash_record(seq: u64, prev_hash: &str, payload: &LedgerPayload) -> Result<String> {
    let material = HashMaterial {
        seq,
        prev_hash,
        payload,
    };
    let bytes = serde_json::to_vec(&material)?;
    let mut hasher = Hasher::new();
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn apply_payload(traces: &mut BTreeMap<String, TraceEntry>, payload: &LedgerPayload) -> Result<()> {
    match payload {
        LedgerPayload::Remember { trace } => {
            traces.insert(
                trace.id.clone(),
                TraceEntry {
                    trace: trace.clone(),
                    status: TraceStatus::Active,
                },
            );
        }
        LedgerPayload::Supersede { old_id, .. } => {
            let entry = traces
                .get_mut(old_id)
                .ok_or_else(|| KernelError::NotFound(old_id.clone()))?;
            entry.status = TraceStatus::Superseded;
        }
        LedgerPayload::Fulfill { id } => {
            let entry = traces
                .get_mut(id)
                .ok_or_else(|| KernelError::NotFound(id.clone()))?;
            entry.status = TraceStatus::Fulfilled;
        }
        LedgerPayload::Purge { id, .. } => {
            let entry = traces
                .get_mut(id)
                .ok_or_else(|| KernelError::NotFound(id.clone()))?;
            entry.status = TraceStatus::Purged;
        }
    }
    Ok(())
}
