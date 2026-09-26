//! Public labeled-retrieval adapter. The kernel and BM25 never receive relevance judgments.
use clap::{Parser, Subcommand};
use hikmah_kernel::policy::KernelPolicy;
use hikmah_kernel::recall::RecallQuery;
use hikmah_kernel::trace::{Trace, TraceKind};
use hikmah_kernel::MemoryStore;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CLOCK: u64 = 1_700_000_000_000;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    mode: Mode,
}
#[derive(Subcommand)]
enum Mode {
    Retrieve {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        queries: PathBuf,
        #[arg(long)]
        store_dir: PathBuf,
        #[arg(long)]
        dataset: String,
        #[arg(long, default_value_t = 10)]
        k: usize,
    },
    Corrections {
        #[arg(long)]
        store_dir: PathBuf,
        #[arg(long, default_value_t = 100)]
        cases: usize,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    id: String,
    content: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    id: String,
    text: String,
}

fn emit(value: Value) -> Result<()> {
    let mut out = io::stdout().lock();
    writeln!(out, "{value}")?;
    out.flush()?;
    Ok(())
}
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn fresh(dir: &Path) -> Result<PathBuf> {
    fs::create_dir(dir)?;
    Ok(dir.join("memory.jsonl"))
}
fn make_trace(id: &str, content: &str, source: &str, kind: TraceKind) -> Trace {
    let mut t = Trace::new(kind, content, source);
    t.id = id.into();
    t.created_at_ms = CLOCK;
    t.provenance.observed_at_ms = CLOCK;
    t.provenance.locator = Some(format!("{source}:{id}"));
    t
}
fn terms(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

// Transparent lexical baseline: positive Robertson IDF, k1=1.2, b=0.75;
// lowercase Unicode alphanumeric terms, no stemming/stopword removal, unique query terms.
#[derive(Default)]
struct Bm25 {
    ids: Vec<String>,
    lengths: Vec<usize>,
    postings: HashMap<String, Vec<(usize, usize)>>,
    total_terms: usize,
}
impl Bm25 {
    fn add(&mut self, doc: &Document) {
        let words = terms(&doc.content);
        let mut frequencies = HashMap::new();
        for word in &words {
            *frequencies.entry(word.clone()).or_insert(0) += 1;
        }
        let index = self.ids.len();
        self.ids.push(doc.id.clone());
        self.lengths.push(words.len());
        self.total_terms += words.len();
        for (term, frequency) in frequencies {
            self.postings
                .entry(term)
                .or_default()
                .push((index, frequency));
        }
    }
    fn search(&self, query: &str, k: usize) -> Vec<Value> {
        if self.ids.is_empty() || self.total_terms == 0 {
            return vec![];
        }
        let n = self.ids.len() as f64;
        let avg = self.total_terms as f64 / n;
        let mut scores: HashMap<usize, f64> = HashMap::new();
        for word in terms(query).into_iter().collect::<BTreeSet<_>>() {
            if let Some(postings) = self.postings.get(&word) {
                let df = postings.len() as f64;
                let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                for &(index, frequency) in postings {
                    let tf = frequency as f64;
                    let norm = 1.2 * (0.25 + 0.75 * self.lengths[index] as f64 / avg);
                    *scores.entry(index).or_default() += idf * tf * 2.2 / (tf + norm);
                }
            }
        }
        let mut ranked: Vec<_> = scores.into_iter().collect();
        ranked
            .sort_by(|(a, x), (b, y)| y.total_cmp(x).then_with(|| self.ids[*a].cmp(&self.ids[*b])));
        ranked
            .into_iter()
            .take(k)
            .map(|(i, score)| json!({"id": self.ids[i], "score": score}))
            .collect()
    }
}

fn retrieve(corpus: &Path, queries: &Path, dir: &Path, dataset: &str, k: usize) -> Result<()> {
    if !(1..=100).contains(&k) || dataset.trim().is_empty() {
        return Err("k must be 1..100 and dataset must be nonempty".into());
    }
    let policy = KernelPolicy {
        recall_limit: k,
        ..KernelPolicy::default()
    };
    let path = fresh(dir)?;
    let source = format!("beir:{dataset}");
    let mut store = MemoryStore::open(&path, policy.clone())?;
    let mut baseline = Bm25::default();
    let mut ids = HashSet::new();
    let mut rejected = Vec::new();
    let mut batch = Vec::new();
    let mut baseline_index_ms = 0.0;
    let ingest = Instant::now();
    for line in BufReader::new(File::open(corpus)?).lines() {
        let doc: Document = serde_json::from_str(&line?)?;
        if doc.id.trim().is_empty() || !ids.insert(doc.id.clone()) {
            return Err("empty or duplicate document id".into());
        }
        let trace = make_trace(&doc.id, &doc.content, &source, TraceKind::Observation);
        if let Err(reason) = trace.validate() {
            rejected.push(json!({"id": doc.id, "reason": reason.to_string()}));
            continue;
        }
        let timer = Instant::now();
        baseline.add(&doc);
        baseline_index_ms += ms(timer);
        batch.push(trace);
        if batch.len() == 1000 {
            store.remember_many(std::mem::take(&mut batch))?;
        }
    }
    if !batch.is_empty() {
        store.remember_many(batch)?;
    }
    let ingest_ms = ms(ingest);
    store.verify()?;
    let head_before = serde_json::to_value(store.head())?;
    drop(store);
    let start = Instant::now();
    let store = MemoryStore::open_existing(&path, policy)?;
    let reopen_ms = ms(start);
    store.verify()?;
    let provenance_retained = store
        .all()
        .filter(|entry| {
            let t = &entry.trace;
            t.provenance.source == source
                && !t.provenance.verified
                && t.created_at_ms == CLOCK
                && t.provenance.observed_at_ms == CLOCK
                && t.provenance.locator.as_deref() == Some(format!("{source}:{}", t.id).as_str())
        })
        .count();
    if store.record_count() != baseline.ids.len()
        || provenance_retained != baseline.ids.len()
        || serde_json::to_value(store.head())? != head_before
    {
        return Err("replay, head or provenance invariant failed".into());
    }
    emit(
        json!({"kind": "loaded", "corpus_total": ids.len(), "admitted": baseline.ids.len(),
        "rejected": rejected, "provenance_retained": provenance_retained,
        "ingest_both_ms": ingest_ms, "bm25_index_ms": baseline_index_ms, "reopen_ms": reopen_ms,
        "policy": store.policy(), "bm25": {"k1": 1.2, "b": 0.75}, "query_clock_ms": CLOCK}),
    )?;
    let mut query_ids = HashSet::new();
    for line in BufReader::new(File::open(queries)?).lines() {
        let q: Query = serde_json::from_str(&line?)?;
        if q.id.trim().is_empty() || !query_ids.insert(q.id.clone()) {
            return Err("invalid query id".into());
        }
        let mut query = RecallQuery::new(&q.text);
        query.now_ms = CLOCK;
        query.limit = k;
        let start = Instant::now();
        let recalled = store.recall(&query);
        let hikmah_ms = ms(start);
        let start = Instant::now();
        let bm25 = baseline.search(&q.text, k);
        let bm25_ms = ms(start);
        let hikmah: Vec<_> = recalled.into_iter().map(|r| json!({
            "id": r.trace.id, "score": r.score, "duplicates_folded": r.duplicates,
            "provenance_source": r.trace.provenance.source, "verified": r.trace.provenance.verified,
        })).collect();
        emit(
            json!({"kind": "query", "id": q.id, "hikmah": hikmah, "bm25": bm25,
                    "hikmah_ms": hikmah_ms, "bm25_ms": bm25_ms}),
        )?;
    }
    emit(json!({"kind": "finished", "queries": query_ids.len()}))
}

fn corrections(dir: &Path, cases: usize) -> Result<()> {
    if !(1..=10000).contains(&cases) {
        return Err("cases must be 1..10000".into());
    }
    let path = fresh(dir)?;
    let mut store = MemoryStore::open(&path, KernelPolicy::default())?;
    let mut stale_hits = 0;
    let mut latest_hits = 0;
    let mut model_rejections = 0;
    let mut link_hits = 0;
    let mut expected = Vec::new();
    for i in 0..cases {
        let subject = format!("service{i}topic");
        let key = format!("{subject}.region");
        let mut predecessor = None;
        for (version, value) in ["oldregion", "middlezone", "newlocation"]
            .iter()
            .enumerate()
        {
            let id = format!("{i}-v{version}");
            let mut trace = make_trace(
                &id,
                &format!("{subject} {value}"),
                "synthetic:correction",
                if version == 0 {
                    TraceKind::Belief
                } else {
                    TraceKind::Correction
                },
            );
            trace.claim_key = Some(key.clone());
            trace.claim_value = Some((*value).into());
            trace.supersedes = predecessor;
            store.remember(trace)?;
            predecessor = Some(id);
        }
        let mut malicious = make_trace(
            &format!("{i}-model"),
            "invented overwrite",
            "model:fixture",
            TraceKind::Correction,
        );
        malicious.supersedes = predecessor;
        model_rejections += usize::from(store.remember(malicious).is_err());
        let results = store.recall(&RecallQuery::new(&subject));
        stale_hits += results
            .iter()
            .filter(|r| r.trace.id == format!("{i}-v0") || r.trace.id == format!("{i}-v1"))
            .count();
        latest_hits += usize::from(results.iter().any(|r| r.trace.id == format!("{i}-v2")));
        link_hits +=
            usize::from(results.iter().any(|r| {
                r.trace.id == format!("{i}-v2") && r.supersedes == Some(format!("{i}-v1"))
            }));
        expected.push((subject, format!("{i}-v2")));
    }
    let old_only_results = store
        .recall(&RecallQuery::new("oldregion middlezone"))
        .len();
    drop(store);
    let store = MemoryStore::open_existing(&path, KernelPolicy::default())?;
    store.verify()?;
    let restart_hits = expected
        .iter()
        .filter(|(q, id)| {
            store
                .recall(&RecallQuery::new(q))
                .iter()
                .any(|r| &r.trace.id == id)
        })
        .count();
    let provenance = store
        .all()
        .filter(|e| {
            e.trace.provenance.source == "synthetic:correction" && !e.trace.provenance.verified
        })
        .count();
    let ok = stale_hits == 0
        && latest_hits == cases
        && model_rejections == cases
        && link_hits == cases
        && old_only_results == 0
        && restart_hits == cases
        && provenance == 3 * cases;
    emit(
        json!({"kind": "synthetic_corrections", "cases": cases, "ok": ok,
        "stale_hits": stale_hits, "latest_hits": latest_hits, "supersession_link_hits": link_hits,
        "model_overwrite_rejections": model_rejections, "old_only_query_results": old_only_results,
        "latest_hits_after_restart": restart_hits, "provenance_retained": provenance,
        "limits": ["Generated structured histories; not extracted real incident corrections.",
          "Subject variants of one generated template, not independent real incidents or a proof across all histories.",
          "A query matching only retired terms abstains; no semantic alias expansion is implied."]}),
    )?;
    if !ok {
        return Err("correction invariant failure".into());
    }
    Ok(())
}

fn main() -> Result<()> {
    match Args::parse().mode {
        Mode::Retrieve {
            corpus,
            queries,
            store_dir,
            dataset,
            k,
        } => retrieve(&corpus, &queries, &store_dir, &dataset, k),
        Mode::Corrections { store_dir, cases } => corrections(&store_dir, cases),
    }
}
