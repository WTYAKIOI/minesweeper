use clap::{Parser, Subcommand};
use std::path::PathBuf;
use base64::Engine as _;

#[derive(Parser)]
#[command(name = "minesweeper-agent")]
#[command(about = "扫雷认知助手 - Minesweeper Cognitive Agent")]
#[command(version = "0.2.0")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// 启动 Web 服务
    Serve {
        #[arg(short, long, default_value = "8080")]
        port: u16,
        /// 蒙特卡洛模拟次数，0=动态自适应（推荐）
        #[arg(long, default_value = "0")]
        mc_iterations: usize,
        /// OCR 微服务地址
        #[arg(long, default_value = "http://localhost:5001")]
        ocr_url: String,
    },
    /// 分析一个局面 (从 JSON 文件输入)
    Analyze {
        /// JSON 文件路径
        #[arg(short, long)]
        input: PathBuf,
        /// LLM 模式
        #[arg(short, long, default_value = "answer")]
        mode: String,
        /// 是否调用 LLM
        #[arg(long)]
        use_llm: bool,
        /// 蒙特卡洛模拟次数，0=动态自适应（推荐）
        #[arg(long, default_value = "0")]
        mc_iterations: usize,
    },
    /// 从截图识别棋盘 (调用 OCR 微服务)
    Ocr {
        /// 图像文件路径
        #[arg(short, long)]
        input: PathBuf,
        /// OCR 微服务地址
        #[arg(long, default_value = "http://localhost:5001")]
        ocr_url: String,
    },
    /// 从 JSON 文件导入局面并输出 PlayerView
    Import {
        #[arg(short, long)]
        input: PathBuf,
    },
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve { port, mc_iterations, ocr_url }) => {
            run_server(port, mc_iterations, ocr_url).await
        }
        Some(Commands::Analyze { input, mode, use_llm, mc_iterations }) => {
            run_analyze(input, mode, use_llm, mc_iterations).await
        }
        Some(Commands::Ocr { input, ocr_url }) => {
            run_ocr(input, ocr_url).await
        }
        Some(Commands::Import { input }) => {
            run_import(input)
        }
        None => {
            // 默认启动 Web 服务 (动态迭代)
            run_server(8080, 0, "http://localhost:5001".to_string()).await
        }
    }
}

async fn run_server(port: u16, mc_iterations: usize, ocr_url: String) -> Result<(), Box<dyn std::error::Error>> {
    let state = crate::server::routes::AppState {
        llm_client: crate::llm::LLMClient::from_env(),
        mc_iterations,
        ocr_url,
        usage_store: std::sync::Arc::new(crate::llm::UsageStore::open()),
    };
    let app = crate::server::create_router(state);
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("扫雷认知助手服务已启动: http://{}", addr);
    println!("  蒙特卡洛: {}", if mc_iterations == 0 { "动态自适应".to_string() } else { format!("{}次", mc_iterations) });
    println!("  OCR 服务: {}", std::env::var("OCR_URL").unwrap_or_else(|_| "http://localhost:5001".to_string()));
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_analyze(
    input: PathBuf,
    mode: String,
    use_llm: bool,
    mc_iterations: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(&input)?;
    let request: serde_json::Value = serde_json::from_str(&content)?;

    let board_raw = request["board"].as_array().ok_or("missing board array")?;
    let board: Vec<Vec<i32>> = board_raw
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap_or(-1) as i32)
                .collect()
        })
        .collect();
    let remaining_mines = request["remaining_mines"].as_u64().unwrap_or(0) as u32;

    // 旗帜正确性判定 (容错视图: 矛盾旗降级为未知, 推理不再信任误标旗)
    let (view, flag_verification, flag_proofs) = crate::engine::verify_board_full(&board, remaining_mines);
    if flag_verification.has_contradiction {
        println!("[旗帜验证] {}", flag_verification.summary);
        for f in flag_verification
            .flags
            .iter()
            .filter(|f| f.status == crate::model::FlagVerifyStatus::Contradicted)
        {
            println!("  [矛盾旗] {} — {}", f.coord, f.reason);
        }
    }

    let mut deterministic = crate::engine::DeterministicEngine::solve_with_subset_rule(&view);
    let has_coord = |c: &crate::model::Coord, proofs: &[crate::model::Proof]| proofs.iter().any(|p| &p.conclusion.coord == c);
    for p in flag_proofs
        .iter()
        .chain(crate::engine::derive_local_forced_proofs(&view).iter())
    {
        if !has_coord(&p.conclusion.coord, &deterministic) {
            deterministic.push(p.clone());
        }
    }

    let known_mines: std::collections::HashSet<_> = deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();
    let known_safe: std::collections::HashSet<_> = deterministic
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();

    // 剩余雷数已为 0: 所有未知格必为安全
    if view.remaining_mines == 0 {
        for c in &view.unknown {
            let covered = deterministic.iter().any(|p| p.conclusion.coord == *c);
            if !covered {
                deterministic.push(crate::model::Proof {
                    conclusion: crate::model::Conclusion { coord: *c, is_mine: false },
                    depends_on: Vec::new(),
                    rule: "剩余雷数已为 0, 该格必为安全".to_string(),
                });
            }
        }
    }
    let mut probabilities = crate::engine::ProbabilityEngine::compute(&view, &known_mines, &known_safe);
    // 雷概率 = 0 的格 → 判定为安全并给出理由 (与 Web 管道一致)
    let mut zero_safe: std::collections::HashSet<crate::model::Coord> = std::collections::HashSet::new();
    for p in &probabilities {
        if p.mine_probability > 0.0 || known_mines.contains(&p.coord) || known_safe.contains(&p.coord) {
            continue;
        }
        zero_safe.insert(p.coord);
        let reason = if view.remaining_mines == 0 {
            "剩余雷数已为 0, 该格必为安全".to_string()
        } else {
            "约束解析下该格雷概率为 0%, 判定为安全".to_string()
        };
        deterministic.push(crate::model::Proof {
            conclusion: crate::model::Conclusion { coord: p.coord, is_mine: false },
            depends_on: view.revealed.iter()
                .filter(|rv| rv.coord.neighbors(view.width, view.height).contains(&p.coord))
                .map(|rv| rv.coord).take(8).collect(),
            rule: reason,
        });
    }
    probabilities.retain(|p| !zero_safe.contains(&p.coord));
    let regions = crate::engine::RegionAnalyzer::analyze(&view, &probabilities);

    let ir = crate::model::InferenceIR {
        deterministic,
        probabilities,
        regions,
        flag_verification,
    };

    let llm_mode = match mode.as_str() {
        "teaching" => crate::llm::LLMMode::Teaching,
        "strategy" => crate::llm::LLMMode::Strategy,
        _ => crate::llm::LLMMode::Answer,
    };
    let translator = crate::llm::Translator::new(llm_mode);

    let analysis = if use_llm {
        if let Some(client) = crate::llm::LLMClient::from_env() {
            let system_prompt = translator.build_system_prompt();
            let user_message = translator.build_user_message(&view, &ir);
            match client.chat(&system_prompt, &user_message).await {
                Ok(result) => {
                    if let Some(u) = &result.usage {
                        println!("[usage] {} tokens (prompt {}, completion {})",
                            u.total_tokens, u.prompt_tokens, u.completion_tokens);
                    }
                    result.content
                }
                Err(e) => format!("LLM 调用失败: {}, 回退到本地分析\n\n{}", e, translator.local_translate(&ir)),
            }
        } else {
            "未配置 OPENAI_API_KEY, 回退到本地分析\n\n".to_string() + &translator.local_translate(&ir)
        }
    } else {
        translator.local_translate(&ir)
    };

    println!("=== 棋盘信息 ===");
    println!("尺寸: {}x{}", view.width, view.height);
    println!("剩余雷数: {}", view.remaining_mines);
    println!("已翻开: {}, 已标旗: {}, 未知: {}", view.revealed.len(), view.flagged.len(), view.unknown.len());
    println!();
    println!("=== 分析结果 ===");
    println!("{}", analysis);
    println!();
    println!("=== IR (JSON) ===");
    println!("{}", serde_json::to_string_pretty(&ir)?);

    Ok(())
}

async fn run_ocr(input: PathBuf, ocr_url: String) -> Result<(), Box<dyn std::error::Error>> {
    let image_data = std::fs::read(&input)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&image_data);

    let client = reqwest::Client::new();
    let url = format!("{}/api/ocr", ocr_url);
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "image": b64 }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await?;
        return Err(format!("OCR 服务错误 {}: {}", status, body).into());
    }

    let result: serde_json::Value = resp.json().await?;

    println!("=== OCR 识别结果 ===");
    println!("成功: {}", result["success"]);
    if let Some(board) = result["board"].as_array() {
        println!("棋盘尺寸: {}x{}", board.len(), board[0].as_array().map(|r| r.len()).unwrap_or(0));
        println!();
        for row in board {
            let row_vals: Vec<String> = row.as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    let n = v.as_i64().unwrap_or(-1);
                    match n {
                        -1 => "?".to_string(),
                        -2 => "F".to_string(),
                        n => n.to_string(),
                    }
                })
                .collect();
            println!("  {}", row_vals.join(" "));
        }
    }
    if let Some(e) = result.get("error") {
        println!("错误: {}", e);
    }

    Ok(())
}

fn run_import(input: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(&input)?;
    let request: serde_json::Value = serde_json::from_str(&content)?;
    let board_raw = request["board"].as_array().ok_or("missing board array")?;
    let board: Vec<Vec<i32>> = board_raw
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap_or(-1) as i32)
                .collect()
        })
        .collect();
    let remaining_mines = request["remaining_mines"].as_u64().unwrap_or(0) as u32;
    let view = crate::model::PlayerView::from_2d(&board, remaining_mines);
    println!("{}", serde_json::to_string_pretty(&view)?);
    Ok(())
}
