use minesweeper_agent::{
    Coord, DeterministicEngine, InferenceIR, LLMMode, MonteCarloEngine,
    PlayerView, RegionAnalyzer, Translator,
};
use std::collections::HashSet;

#[test]
fn test_end_to_end_analysis() {
    let board = vec![
        vec![1, 2, -1, -1, 0],
        vec![-1, 2, -1, 0, 0],
        vec![-1, 1, -1, 0, 0],
    ];
    let view = PlayerView::from_2d(&board, 5);
    assert!(view.validate());

    let deterministic = DeterministicEngine::solve_with_subset_rule(&view);

    let known_mines: HashSet<Coord> = deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();
    let known_safe: HashSet<Coord> = deterministic
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();

    let mc = MonteCarloEngine::new(500);
    let probabilities = mc.simulate_with_deductions(&view, &known_mines, &known_safe);
    let regions = RegionAnalyzer::analyze(&view, &probabilities);

    let ir = InferenceIR {
        deterministic,
        probabilities,
        regions,
    };

    let translator = Translator::new(LLMMode::Answer);
    let analysis = translator.local_translate(&ir);

    assert!(!analysis.is_empty());
}

#[test]
fn test_data_firewall() {
    // 确保 IR 中不包含未翻开格子的真实雷藏
    let board = vec![
        vec![1, -1, -1],
        vec![-1, -1, -1],
        vec![-1, -1, -1],
    ];
    let view = PlayerView::from_2d(&board, 1);

    let deterministic = DeterministicEngine::solve_with_subset_rule(&view);
    let known_mines: HashSet<Coord> = deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();
    let known_safe: HashSet<Coord> = deterministic
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();

    let mc = MonteCarloEngine::new(500);
    let probabilities = mc.simulate_with_deductions(&view, &known_mines, &known_safe);
    let regions = RegionAnalyzer::analyze(&view, &probabilities);

    let ir = InferenceIR {
        deterministic,
        probabilities,
        regions,
    };

    let ir_json = serde_json::to_string(&ir).unwrap();
    // IR JSON 不应包含 "mine" 作为真实雷藏 (只包含 "is_mine" 作为推理结论)
    // 关键: 不包含 "true_mine" 或 "solution" 等字段
    assert!(!ir_json.contains("true_mine"));
    assert!(!ir_json.contains("solution"));
    assert!(!ir_json.contains("actual_board"));
}

#[test]
fn test_all_modes_produce_output() {
    let board = vec![
        vec![3, -1, -1],
        vec![-1, -1, -1],
        vec![0, -1, -1],
    ];
    let view = PlayerView::from_2d(&board, 3);

    let deterministic = DeterministicEngine::solve_with_subset_rule(&view);
    let mc = MonteCarloEngine::new(500);
    let probabilities = mc.simulate(&view);
    let regions = RegionAnalyzer::analyze(&view, &probabilities);

    let ir = InferenceIR {
        deterministic,
        probabilities,
        regions,
    };

    for mode in [LLMMode::Answer, LLMMode::Teaching, LLMMode::Strategy] {
        let translator = Translator::new(mode.clone());
        let result = translator.local_translate(&ir);
        assert!(!result.is_empty(), "Mode {:?} produced empty output", mode);
    }
}
