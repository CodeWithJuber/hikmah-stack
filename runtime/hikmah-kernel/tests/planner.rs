use hikmah_kernel::planner::{plan, Action, PlanProblem};

#[test]
fn branch_loom_finds_short_symbolic_plan() {
    let problem = PlanProblem {
        initial: vec!["code_ready".into()],
        goal: vec!["production_verified".into()],
        actions: vec![
            Action {
                name: "run_tests".into(),
                requires: vec!["code_ready".into()],
                adds: vec!["tests_passed".into()],
                removes: vec![],
            },
            Action {
                name: "deploy_canary".into(),
                requires: vec!["tests_passed".into()],
                adds: vec!["canary_live".into()],
                removes: vec![],
            },
            Action {
                name: "verify_canary".into(),
                requires: vec!["canary_live".into()],
                adds: vec!["production_verified".into()],
                removes: vec![],
            },
        ],
        max_depth: 6,
        max_states: 10_000,
    };
    let result = plan(&problem).unwrap();
    assert!(result.found);
    assert_eq!(
        result.actions,
        vec!["run_tests", "deploy_canary", "verify_canary"]
    );
}

#[test]
fn search_stops_at_the_state_budget() {
    let actions = (0..16)
        .map(|i| Action {
            name: format!("toggle_{i}"),
            requires: vec![],
            adds: vec![format!("f{i}")],
            removes: vec![],
        })
        .collect();
    let problem = PlanProblem {
        initial: vec![],
        goal: vec!["unreachable".into()],
        actions,
        max_depth: 12,
        max_states: 5_000,
    };
    let result = plan(&problem).unwrap();
    assert!(!result.found);
    assert!(result.budget_exhausted);
    assert!(result.explored_states <= 5_000);
}
