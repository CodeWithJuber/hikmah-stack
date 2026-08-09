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
    };
    let result = plan(&problem).unwrap();
    assert!(result.found);
    assert_eq!(
        result.actions,
        vec!["run_tests", "deploy_canary", "verify_canary"]
    );
}
