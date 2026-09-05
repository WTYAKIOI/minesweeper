//! Token 用量统计 — 记录每次 LLM 调用的 usage, 聚合为 今日/近7日/本月/累计 维度
//!
//! 存储选型说明: 设计稿建议 SQLite, 但本项目为本地单用户工具、零原生依赖,
//! 用量记录量级很小 (每次调用一行), 故采用 JSONL 追加文件:
//!   - 无 schema 迁移, 人类可读, 可直接被 CSV/表格工具消费
//!   - record() 追加一行, 启动时全量载入内存聚合
//!   - TokenUsageRecord / 统计 API 形状与设计稿一致, 后续可无痛替换为 SQLite

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Mutex;

/// 单条用量记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Unix 时间戳 (秒)
    pub timestamp: u64,
    pub model: String,
    pub provider: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// 估算成本 (美元); 未知模型/本地模型为 0
    pub cost: f64,
}

/// 按模型聚合
#[derive(Debug, Serialize)]
pub struct ModelUsage {
    pub tokens: u64,
    pub cost: f64,
}

/// 趋势点 (按天)
#[derive(Debug, Serialize)]
pub struct TrendPoint {
    pub date: String,
    pub tokens: u64,
    pub cost: f64,
}

/// 统计响应 (一次返回全部维度, 前端按需取用)
#[derive(Debug, Serialize)]
pub struct UsageStats {
    pub today_tokens: u64,
    pub today_cost: f64,
    pub week_tokens: u64,
    pub week_cost: f64,
    pub month_tokens: u64,
    pub month_cost: f64,
    pub all_tokens: u64,
    pub all_cost: f64,
    pub by_model: BTreeMap<String, ModelUsage>,
    /// 近 7 天趋势 (含今日, 旧 → 新)
    pub trend: Vec<TrendPoint>,
    /// 月度 token 限额 (环境变量 LLM_MONTHLY_TOKEN_LIMIT, 未设置则不限)
    pub monthly_limit: Option<u64>,
    /// 本月用量是否已超限
    pub limit_exceeded: bool,
}

/// 模型单价表: (模型名前缀, 输入 $/1M tokens, 输出 $/1M tokens)
/// 匹配规则: 小写精确匹配优先, 其次最长前缀匹配; 未命中按 0 计 (不瞎估)
const MODEL_PRICES: &[(&str, f64, f64)] = &[
    ("gpt-4o-mini", 0.15, 0.60),
    ("gpt-4o", 2.50, 10.00),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("gpt-4.1", 2.00, 8.00),
    ("deepseek-chat", 0.14, 0.28),
    ("deepseek-reasoner", 0.55, 2.19),
    ("moonshot-v1-8k", 0.012, 0.012),
    ("moonshot-v1-32k", 0.024, 0.024),
    ("moonshot-v1-128k", 0.060, 0.060),
    ("kimi-k2", 0.60, 2.50),
    ("qwen-plus", 0.004, 0.012),
    ("qwen-turbo", 0.0005, 0.0015),
    ("qwen-max", 0.016, 0.064),
];

/// 估算一次调用的成本 (美元)
pub fn estimate_cost(model: &str, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    let m = model.to_lowercase();
    let mut hit: Option<&(&str, f64, f64)> = None;
    for entry in MODEL_PRICES {
        if m == entry.0 {
            hit = Some(entry);
            break;
        }
        // 最长前缀匹配
        if m.starts_with(entry.0) {
            hit = match hit {
                Some(cur) if cur.0.len() >= entry.0.len() => Some(cur),
                _ => Some(entry),
            };
        }
    }
    match hit {
        Some((_, pin, pout)) => {
            prompt_tokens as f64 / 1e6 * pin + completion_tokens as f64 / 1e6 * pout
        }
        None => 0.0,
    }
}

/// Unix 秒 → (年, 月, 日), Howard Hinnant 的 civil_from_days 算法 (无外部依赖)
pub fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 本地时区偏移 (小时)。服务端无法可靠获取系统时区, 默认 UTC+8 (与界面语言一致),
/// 可用环境变量 USAGE_TZ_OFFSET_HOURS 覆盖。
fn tz_offset_hours() -> i32 {
    std::env::var("USAGE_TZ_OFFSET_HOURS")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(8)
}

/// Unix 秒 → 本地化 "YYYY-MM-DD"
pub fn local_date_key(unix_secs: u64) -> String {
    let offset = tz_offset_hours() as i64 * 3600;
    let days = (unix_secs as i64 + offset).div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Unix 秒 → 本地化 "YYYY-MM"
pub fn local_month_key(unix_secs: u64) -> String {
    local_date_key(unix_secs)[..7].to_string()
}

/// 当前 Unix 秒
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 用量存储: 内存聚合 + JSONL 追加持久化
pub struct UsageStore {
    path: PathBuf,
    records: Mutex<Vec<UsageRecord>>,
    monthly_limit: Option<u64>,
}

impl UsageStore {
    /// 打开 (或创建) 用量存储
    ///
    /// - 路径: 环境变量 USAGE_DB_PATH, 默认 ./data/usage.jsonl
    /// - 月度限额: 环境变量 LLM_MONTHLY_TOKEN_LIMIT
    pub fn open() -> Self {
        let path = std::env::var("USAGE_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data").join("usage.jsonl"));
        let records = load_records(&path);
        let monthly_limit = std::env::var("LLM_MONTHLY_TOKEN_LIMIT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|v| *v > 0);
        Self { path, records: Mutex::new(records), monthly_limit }
    }

    /// 记录一次调用 (带 provider); usage 缺失 (本地服务未返回) 时跳过
    pub fn record(&self, model: &str, provider: &str, usage: &crate::llm::client::TokenUsage) {
        if usage.total_tokens == 0 && usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
            return;
        }
        let rec = UsageRecord {
            timestamp: unix_now(),
            model: model.to_string(),
            provider: provider.to_string(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            cost: estimate_cost(model, usage.prompt_tokens, usage.completion_tokens),
        };
        let mut guard = match self.records.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Err(e) = append_record(&self.path, &rec) {
            eprintln!("[usage] 写入失败: {} (内存中仍保留)", e);
        }
        guard.push(rec);
    }

    /// 本月已用 token 数 (限额判断用)
    pub fn month_tokens(&self) -> u64 {
        let month = local_month_key(unix_now());
        let guard = match self.records.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard
            .iter()
            .filter(|r| local_month_key(r.timestamp) == month)
            .map(|r| r.total_tokens as u64)
            .sum()
    }

    /// 月度限额是否已超
    pub fn limit_exceeded(&self) -> bool {
        match self.monthly_limit {
            Some(limit) => self.month_tokens() >= limit,
            None => false,
        }
    }

    /// 全维度统计
    pub fn stats(&self) -> UsageStats {
        // 先在持锁前计算限额状态 (limit_exceeded 会再次加锁, 避免重入死锁)
        let limit_exceeded_now = self.limit_exceeded();
        let guard = match self.records.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let now = unix_now();
        let today = local_date_key(now);
        let month = local_month_key(now);
        // 近 7 天日期集合 (含今日), 顺序: 旧 → 新 (与设计稿 09/01..09/07 一致)
        let mut trend_dates: Vec<String> = Vec::with_capacity(7);
        for i in (0..7).rev() {
            let offset = tz_offset_hours() as i64 * 3600;
            let days = (now as i64 + offset).div_euclid(86_400) - i;
            let (y, m, d) = civil_from_days(days);
            trend_dates.push(format!("{:04}-{:02}-{:02}", y, m, d));
        }

        let mut today_tokens = 0u64;
        let mut today_cost = 0f64;
        let mut week_tokens = 0u64;
        let mut week_cost = 0f64;
        let mut month_tokens = 0u64;
        let mut month_cost = 0f64;
        let mut all_tokens = 0u64;
        let mut all_cost = 0f64;
        let mut by_model: BTreeMap<String, (u64, f64)> = BTreeMap::new();
        let mut trend_map: BTreeMap<String, (u64, f64)> = BTreeMap::new();

        for r in guard.iter() {
            let date = local_date_key(r.timestamp);
            let cost = r.cost;
            let tokens = r.total_tokens as u64;
            all_tokens += tokens;
            all_cost += cost;
            if date == today {
                today_tokens += tokens;
                today_cost += cost;
            }
            if trend_dates.contains(&date) {
                week_tokens += tokens;
                week_cost += cost;
            }
            if local_month_key(r.timestamp) == month {
                month_tokens += tokens;
                month_cost += cost;
            }
            let e = by_model.entry(r.model.clone()).or_insert((0, 0.0));
            e.0 += tokens;
            e.1 += cost;
            let te = trend_map.entry(date).or_insert((0, 0.0));
            te.0 += tokens;
            te.1 += cost;
        }

        let trend = trend_dates
            .iter()
            .map(|d| {
                let (t, c) = trend_map.get(d).copied().unwrap_or((0, 0.0));
                TrendPoint { date: d.clone(), tokens: t, cost: c }
            })
            .collect();

        let by_model = by_model
            .into_iter()
            .map(|(k, (tokens, cost))| (k, ModelUsage { tokens, cost }))
            .collect();

        UsageStats {
            today_tokens,
            today_cost,
            week_tokens,
            week_cost,
            month_tokens,
            month_cost,
            all_tokens,
            all_cost,
            by_model,
            trend,
            monthly_limit: self.monthly_limit,
            limit_exceeded: limit_exceeded_now,
        }
    }

    /// 明细日志 (按时间倒序), 分页
    pub fn logs(&self, limit: usize, offset: usize) -> Vec<UsageRecord> {
        let guard = match self.records.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut all: Vec<UsageRecord> = guard.clone();
        all.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        all.into_iter().skip(offset).take(limit).collect()
    }
}

fn load_records(path: &PathBuf) -> Vec<UsageRecord> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = std::io::BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<UsageRecord>(line) {
            Ok(r) => out.push(r),
            Err(_) => continue, // 跳过损坏行
        }
    }
    out
}

fn append_record(path: &PathBuf, rec: &UsageRecord) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let mut line = serde_json::to_string(rec).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
    })?;
    line.push('\n');
    f.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_civil_from_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29)); // 闰年
        assert_eq!(civil_from_days(20_701), (2026, 9, 5));
        // 往返一致性: date -> days -> date
        let (y, m, d) = civil_from_days(20_701);
        assert_eq!((y, m, d), (2026, 9, 5));
    }

    #[test]
    fn test_local_date_key_tz() {
        // 默认 UTC+8: 2026-09-05T00:00Z → 本地 08:00, 同日
        let secs: u64 = 20_701 * 86_400;
        assert_eq!(local_date_key(secs), "2026-09-05");
        // 2026-09-05T16:00Z → 本地 2026-09-06 00:00, 跨日
        assert_eq!(local_date_key(secs + 16 * 3600), "2026-09-06");
        assert_eq!(local_month_key(secs), "2026-09");
    }

    #[test]
    fn test_estimate_cost() {
        // 精确匹配
        let c = estimate_cost("gpt-4o-mini", 1_000_000, 1_000_000);
        assert!((c - (0.15 + 0.60)).abs() < 1e-9);
        // 前缀匹配 (longest)
        let c3 = estimate_cost("deepseek-chat-2025", 1_000_000, 1_000_000);
        assert!((c3 - (0.14 + 0.28)).abs() < 1e-9);
        // 未知模型 → 0
        assert_eq!(estimate_cost("mystery-model", 1_000_000, 1_000_000), 0.0);
        // 大小写不敏感
        assert!(estimate_cost("GPT-4o-MINI", 1_000_000, 0) > 0.0);
    }

    #[test]
    fn test_store_record_stats_persist() {
        let dir = std::env::temp_dir().join(format!("ms-usage-test-{}", std::process::id()));
        let path = dir.join("usage.jsonl");
        let _ = std::fs::remove_file(&path);

        // 依赖环境变量, 手动构造 store
        let store = UsageStore {
            path: path.clone(),
            records: Mutex::new(Vec::new()),
            monthly_limit: Some(1000),
        };
        store.record(
            "gpt-4o-mini",
            "openai",
            &crate::llm::client::TokenUsage { prompt_tokens: 100, completion_tokens: 50, total_tokens: 150 },
        );
        store.record(
            "deepseek-chat",
            "deepseek",
            &crate::llm::client::TokenUsage { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
        );

        let stats = store.stats();
        assert_eq!(stats.all_tokens, 165);
        assert_eq!(stats.today_tokens, 165);
        assert_eq!(stats.month_tokens, 165);
        assert_eq!(stats.by_model.len(), 2);
        assert!(stats.trend.len() == 7);
        assert!(!stats.limit_exceeded); // 165 < 1000
        assert_eq!(stats.monthly_limit, Some(1000));

        // 持久化: 重新加载
        let reloaded = UsageStore {
            path: path.clone(),
            records: Mutex::new(load_records(&path)),
            monthly_limit: None,
        };
        assert_eq!(reloaded.stats().all_tokens, 165);

        // logs 倒序分页
        let logs = store.logs(1, 0);
        assert_eq!(logs.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_zero_usage_skipped() {
        let dir = std::env::temp_dir().join(format!("ms-usage-zero-{}", std::process::id()));
        let store = UsageStore {
            path: dir.join("usage.jsonl"),
            records: Mutex::new(Vec::new()),
            monthly_limit: None,
        };
        store.record("m", "p", &crate::llm::client::TokenUsage::default());
        assert_eq!(store.stats().all_tokens, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
