use crate::model::{route, validate_route, Difficulty};
use crate::store::{BudgetMode, HistoricalUsageSample, Store};
use crate::usage::UsageSnapshot;
use serde::{Deserialize, Serialize};

const HISTORY_DAYS: i64 = 365;
const MIN_COHORT_SIZE: usize = 3;

#[derive(Clone, Debug, Deserialize)]
pub struct EstimateTaskInput {
    pub title: String,
    pub difficulty: Difficulty,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EstimateBatchInput {
    pub budget_mode: BudgetMode,
    #[serde(default)]
    pub weekly_cap_percent: Option<f64>,
    #[serde(default)]
    pub token_cap: Option<i64>,
    pub tasks: Vec<EstimateTaskInput>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskUsageEstimate {
    pub title: String,
    pub difficulty: String,
    pub model: String,
    pub effort: String,
    pub likely_tokens: Option<i64>,
    pub conservative_tokens: Option<i64>,
    pub token_sample_count: usize,
    pub token_cohort: String,
    pub token_confidence: String,
    pub likely_weekly_percent: Option<f64>,
    pub conservative_weekly_percent: Option<f64>,
    pub percentage_sample_count: usize,
    pub percentage_cohort: String,
    pub percentage_confidence: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CapAssessment {
    pub level: String,
    pub unit: String,
    pub cap: f64,
    pub likely_estimate: Option<f64>,
    pub conservative_estimate: Option<f64>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchUsageEstimate {
    pub generated_at: i64,
    pub history_days: i64,
    pub completed_runs_considered: usize,
    pub tasks: Vec<TaskUsageEstimate>,
    pub likely_tokens: Option<i64>,
    pub conservative_tokens: Option<i64>,
    pub likely_weekly_percent: Option<f64>,
    pub conservative_weekly_percent: Option<f64>,
    pub cap_assessment: CapAssessment,
    pub methodology: String,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug)]
struct RoutedTask {
    title: String,
    difficulty: String,
    model: String,
    effort: String,
}

pub fn estimate(
    store: &Store,
    input: EstimateBatchInput,
    now: i64,
) -> Result<BatchUsageEstimate, String> {
    validate_input(&input)?;
    let routed = input
        .tasks
        .into_iter()
        .map(route_task)
        .collect::<Result<Vec<_>, _>>()?;
    let history_since = now.saturating_sub(HISTORY_DAYS * 86_400);
    let samples = store.prediction_samples(history_since)?;
    let estimates = routed
        .iter()
        .map(|task| estimate_task(task, &samples))
        .collect::<Vec<_>>();

    let likely_tokens = sum_optional_i64(estimates.iter().map(|task| task.likely_tokens));
    let conservative_tokens =
        sum_optional_i64(estimates.iter().map(|task| task.conservative_tokens));
    let likely_weekly_percent = round_percent(sum_optional_f64(
        estimates.iter().map(|task| task.likely_weekly_percent),
    ));
    let conservative_weekly_percent = round_percent(sum_optional_f64(
        estimates
            .iter()
            .map(|task| task.conservative_weekly_percent),
    ));
    let cap_assessment = assess_cap(
        input.budget_mode,
        input.weekly_cap_percent,
        input.token_cap,
        likely_tokens,
        conservative_tokens,
        likely_weekly_percent,
        conservative_weekly_percent,
    )?;

    Ok(BatchUsageEstimate {
        generated_at: now,
        history_days: HISTORY_DAYS,
        completed_runs_considered: samples.len(),
        tasks: estimates,
        likely_tokens,
        conservative_tokens,
        likely_weekly_percent,
        conservative_weekly_percent,
        cap_assessment,
        methodology: "Local completed runs from the last 365 days. Likely is the observed p50 and conservative is p90. LimitWise prefers an exact difficulty/model/effort cohort with at least three samples, then model/effort, difficulty, and finally all history.".to_string(),
        limitations: vec![
            "Predictions are estimates, never guarantees, and do not change quota enforcement or the selected cap.".to_string(),
            "Weekly-percentage samples require before/after telemetry in the same reset window; concurrent interactive usage is included, so estimates are conservative and may be noisy.".to_string(),
            "For a percentage-mode batch spanning weekly reset windows, estimate each window's task group separately because the percentage cap renews per window.".to_string(),
            "Low-confidence or unavailable estimates mean more comparable completed runs are needed.".to_string(),
        ],
    })
}

fn validate_input(input: &EstimateBatchInput) -> Result<(), String> {
    if input.tasks.is_empty() {
        return Err("at least one task is required".to_string());
    }
    if input.tasks.iter().any(|task| task.title.trim().is_empty()) {
        return Err("every task needs a title".to_string());
    }
    match input.budget_mode {
        BudgetMode::Percentage => {
            if input.token_cap.is_some() {
                return Err("token_cap is only valid with budget_mode=tokens".to_string());
            }
            let cap = input
                .weekly_cap_percent
                .ok_or_else(|| "weekly_cap_percent is required".to_string())?;
            if !cap.is_finite() || cap <= 0.0 || cap > 100.0 {
                return Err("weekly_cap_percent must be greater than 0 and at most 100".to_string());
            }
        }
        BudgetMode::Tokens => {
            if input.weekly_cap_percent.is_some() {
                return Err(
                    "weekly_cap_percent is only valid with budget_mode=percentage".to_string(),
                );
            }
            let cap = input
                .token_cap
                .ok_or_else(|| "token_cap is required".to_string())?;
            if !(1..=1_000_000_000).contains(&cap) {
                return Err("token_cap must be from 1 to 1000000000".to_string());
            }
        }
    }
    Ok(())
}

fn route_task(input: EstimateTaskInput) -> Result<RoutedTask, String> {
    let default = route(input.difficulty);
    let model = input.model.unwrap_or(default.model);
    let effort = input.effort.unwrap_or(default.effort);
    validate_route(&model, &effort)?;
    Ok(RoutedTask {
        title: input.title.trim().to_string(),
        difficulty: input.difficulty.to_string(),
        model,
        effort,
    })
}

fn estimate_task(task: &RoutedTask, samples: &[HistoricalUsageSample]) -> TaskUsageEstimate {
    let (token_samples, token_cohort) = select_samples(task, samples, false);
    let token_values = token_samples
        .iter()
        .map(|sample| sample.tokens_used as f64)
        .collect::<Vec<_>>();
    let likely_tokens = percentile(&token_values, 0.50).map(|value| value.ceil() as i64);
    let conservative_tokens = percentile(&token_values, 0.90).map(|value| value.ceil() as i64);

    let (percentage_samples, percentage_cohort) = select_samples(task, samples, true);
    let percentage_values = percentage_samples
        .iter()
        .filter_map(|sample| weekly_delta(sample))
        .collect::<Vec<_>>();
    let likely_weekly_percent = round_percent(percentile(&percentage_values, 0.50));
    let conservative_weekly_percent = round_percent(percentile(&percentage_values, 0.90));

    TaskUsageEstimate {
        title: task.title.clone(),
        difficulty: task.difficulty.clone(),
        model: task.model.clone(),
        effort: task.effort.clone(),
        likely_tokens,
        conservative_tokens,
        token_sample_count: token_values.len(),
        token_confidence: confidence(&token_cohort, token_values.len()).to_string(),
        token_cohort,
        likely_weekly_percent,
        conservative_weekly_percent,
        percentage_sample_count: percentage_values.len(),
        percentage_confidence: confidence(&percentage_cohort, percentage_values.len()).to_string(),
        percentage_cohort,
    }
}

fn select_samples<'a>(
    task: &RoutedTask,
    samples: &'a [HistoricalUsageSample],
    require_percentage: bool,
) -> (Vec<&'a HistoricalUsageSample>, String) {
    let eligible =
        |sample: &&HistoricalUsageSample| !require_percentage || weekly_delta(sample).is_some();
    let exact = samples
        .iter()
        .filter(|sample| {
            sample.difficulty == task.difficulty
                && sample.model == task.model
                && sample.effort == task.effort
        })
        .filter(eligible)
        .collect::<Vec<_>>();
    if exact.len() >= MIN_COHORT_SIZE {
        return (exact, "difficulty_model_effort".to_string());
    }
    let route = samples
        .iter()
        .filter(|sample| sample.model == task.model && sample.effort == task.effort)
        .filter(eligible)
        .collect::<Vec<_>>();
    if route.len() >= MIN_COHORT_SIZE {
        return (route, "model_effort".to_string());
    }
    let difficulty = samples
        .iter()
        .filter(|sample| sample.difficulty == task.difficulty)
        .filter(eligible)
        .collect::<Vec<_>>();
    if difficulty.len() >= MIN_COHORT_SIZE {
        return (difficulty, "difficulty".to_string());
    }
    let all = samples.iter().filter(eligible).collect::<Vec<_>>();
    let label = if all.is_empty() {
        "unavailable"
    } else {
        "all_completed_runs"
    };
    (all, label.to_string())
}

fn weekly_delta(sample: &HistoricalUsageSample) -> Option<f64> {
    let before: UsageSnapshot = serde_json::from_str(sample.usage_before_json.as_deref()?).ok()?;
    let after: UsageSnapshot = serde_json::from_str(sample.usage_after_json.as_deref()?).ok()?;
    if before.weekly.resets_at != after.weekly.resets_at {
        return None;
    }
    let delta = after.weekly.used_percent - before.weekly.used_percent;
    (delta.is_finite() && delta > 0.0 && delta <= 100.0).then_some(delta)
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let index = (((sorted.len() - 1) as f64) * percentile).ceil() as usize;
    sorted.get(index).copied()
}

fn confidence(cohort: &str, sample_count: usize) -> &'static str {
    match (cohort, sample_count) {
        ("difficulty_model_effort", 10..) => "high",
        ("difficulty_model_effort", 5..) => "medium",
        ("model_effort", 10..) | ("difficulty", 10..) => "medium",
        ("unavailable", _) => "unavailable",
        _ => "low",
    }
}

fn sum_optional_i64(mut values: impl Iterator<Item = Option<i64>>) -> Option<i64> {
    values.try_fold(0_i64, |sum, value| {
        value.map(|value| sum.saturating_add(value))
    })
}

fn sum_optional_f64(mut values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values.try_fold(0.0_f64, |sum, value| value.map(|value| sum + value))
}

fn round_percent(value: Option<f64>) -> Option<f64> {
    value.map(|value| (value * 1000.0).round() / 1000.0)
}

#[allow(clippy::too_many_arguments)]
fn assess_cap(
    mode: BudgetMode,
    weekly_cap_percent: Option<f64>,
    token_cap: Option<i64>,
    likely_tokens: Option<i64>,
    conservative_tokens: Option<i64>,
    likely_weekly_percent: Option<f64>,
    conservative_weekly_percent: Option<f64>,
) -> Result<CapAssessment, String> {
    let (unit, cap, likely, conservative) = match mode {
        BudgetMode::Percentage => (
            "weekly_percentage_points",
            weekly_cap_percent.ok_or_else(|| "weekly_cap_percent is required".to_string())?,
            likely_weekly_percent,
            conservative_weekly_percent,
        ),
        BudgetMode::Tokens => (
            "tokens",
            token_cap.ok_or_else(|| "token_cap is required".to_string())? as f64,
            likely_tokens.map(|value| value as f64),
            conservative_tokens.map(|value| value as f64),
        ),
    };
    let (level, message) = match (likely, conservative) {
        (None, _) | (_, None) => (
            "unavailable",
            "Cap cannot be assessed because comparable local history is unavailable. Keep a safety margin and confirm with low confidence.".to_string(),
        ),
        (Some(likely), Some(_conservative)) if cap < likely => (
            "likely_insufficient",
            format!(
                "Cap {cap:.3} {unit} is below the likely estimate {likely:.3}; execution may exhaust the batch budget. Increase the cap or explicitly accept the risk before scheduling."
            ),
        ),
        (Some(_), Some(conservative)) if cap < conservative => (
            "tight",
            format!(
                "Cap {cap:.3} {unit} covers the likely estimate but is below the conservative estimate {conservative:.3}; warn the user and require explicit confirmation."
            ),
        ),
        (Some(_), Some(conservative)) => (
            "within_conservative",
            format!(
                "Cap {cap:.3} {unit} meets the conservative estimate {conservative:.3}; actual usage can still differ."
            ),
        ),
    };
    Ok(CapAssessment {
        level: level.to_string(),
        unit: unit.to_string(),
        cap,
        likely_estimate: likely,
        conservative_estimate: conservative,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tokens: i64, percent: f64) -> HistoricalUsageSample {
        HistoricalUsageSample {
            difficulty: "simple".to_string(),
            model: "gpt-5.6-luna".to_string(),
            effort: "low".to_string(),
            tokens_used: tokens,
            usage_before_json: Some(
                "{\"adapter\":\"test\",\"captured_at\":1,\"five_hour\":{\"used_percent\":1.0,\"remaining_percent\":99.0,\"duration_minutes\":300,\"resets_at\":10},\"weekly\":{\"used_percent\":2.0,\"remaining_percent\":98.0,\"duration_minutes\":10080,\"resets_at\":20}}"
                    .to_string(),
            ),
            usage_after_json: Some(format!(
                "{{\"adapter\":\"test\",\"captured_at\":2,\"five_hour\":{{\"used_percent\":1.0,\"remaining_percent\":99.0,\"duration_minutes\":300,\"resets_at\":10}},\"weekly\":{{\"used_percent\":{},\"remaining_percent\":90.0,\"duration_minutes\":10080,\"resets_at\":20}}}}",
                2.0 + percent
            )),
        }
    }

    #[test]
    fn exact_cohort_uses_p50_and_p90() {
        let task = RoutedTask {
            title: "small edit".to_string(),
            difficulty: "simple".to_string(),
            model: "gpt-5.6-luna".to_string(),
            effort: "low".to_string(),
        };
        let samples = vec![
            sample(10_000, 0.2),
            sample(20_000, 0.4),
            sample(30_000, 0.6),
        ];
        let estimate = estimate_task(&task, &samples);
        assert_eq!(estimate.likely_tokens, Some(20_000));
        assert_eq!(estimate.conservative_tokens, Some(30_000));
        assert_eq!(estimate.likely_weekly_percent, Some(0.4));
        assert_eq!(estimate.conservative_weekly_percent, Some(0.6));
        assert_eq!(estimate.token_cohort, "difficulty_model_effort");
    }

    #[test]
    fn cap_below_likely_is_warned() {
        let assessment = assess_cap(
            BudgetMode::Tokens,
            None,
            Some(10_000),
            Some(20_000),
            Some(30_000),
            None,
            None,
        )
        .unwrap();
        assert_eq!(assessment.level, "likely_insufficient");
    }

    #[test]
    fn reset_crossing_is_not_a_percentage_sample() {
        let mut value = sample(10_000, 0.2);
        value.usage_after_json = value
            .usage_after_json
            .map(|json| json.replace("\"resets_at\":20", "\"resets_at\":21"));
        assert_eq!(weekly_delta(&value), None);
    }

    #[test]
    fn unavailable_history_produces_unavailable_confidence() {
        let task = RoutedTask {
            title: "new route".to_string(),
            difficulty: "exceptional".to_string(),
            model: "gpt-5.6-sol".to_string(),
            effort: "xhigh".to_string(),
        };
        let estimate = estimate_task(&task, &[]);
        assert_eq!(estimate.likely_tokens, None);
        assert_eq!(estimate.token_confidence, "unavailable");
        assert_eq!(estimate.percentage_confidence, "unavailable");
    }
}
