use crate::error::{KernelError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanProblem {
    pub initial: Vec<String>,
    pub goal: Vec<String>,
    pub actions: Vec<Action>,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub adds: Vec<String>,
    #[serde(default)]
    pub removes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResult {
    pub found: bool,
    pub actions: Vec<String>,
    pub final_state: Vec<String>,
    pub explored_states: usize,
}

pub fn plan(problem: &PlanProblem) -> Result<PlanResult> {
    if problem.goal.is_empty() {
        return Err(KernelError::Invalid("planning goal cannot be empty".into()));
    }
    let initial: BTreeSet<String> = problem.initial.iter().cloned().collect();
    let goal: BTreeSet<String> = problem.goal.iter().cloned().collect();
    if goal.is_subset(&initial) {
        return Ok(PlanResult {
            found: true,
            actions: Vec::new(),
            final_state: initial.into_iter().collect(),
            explored_states: 1,
        });
    }

    let mut queue = VecDeque::new();
    queue.push_back((initial.clone(), Vec::<String>::new()));
    let mut seen = HashSet::new();
    seen.insert(state_key(&initial));
    let mut explored = 0_usize;

    while let Some((state, path)) = queue.pop_front() {
        explored += 1;
        if path.len() >= problem.max_depth {
            continue;
        }
        for action in &problem.actions {
            let requires: BTreeSet<String> = action.requires.iter().cloned().collect();
            if !requires.is_subset(&state) {
                continue;
            }
            let mut next = state.clone();
            for fact in &action.removes {
                next.remove(fact);
            }
            for fact in &action.adds {
                next.insert(fact.clone());
            }
            let mut next_path = path.clone();
            next_path.push(action.name.clone());
            if goal.is_subset(&next) {
                return Ok(PlanResult {
                    found: true,
                    actions: next_path,
                    final_state: next.into_iter().collect(),
                    explored_states: explored,
                });
            }
            let key = state_key(&next);
            if seen.insert(key) {
                queue.push_back((next, next_path));
            }
        }
    }

    Ok(PlanResult {
        found: false,
        actions: Vec::new(),
        final_state: initial.into_iter().collect(),
        explored_states: explored,
    })
}

fn state_key(state: &BTreeSet<String>) -> String {
    state.iter().cloned().collect::<Vec<_>>().join("\u{1f}")
}

fn default_max_depth() -> usize {
    12
}
