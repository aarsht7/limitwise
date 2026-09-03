use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Simple,
    Standard,
    Complex,
    Exceptional,
}

impl FromStr for Difficulty {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "simple" => Ok(Self::Simple),
            "standard" | "normal" => Ok(Self::Standard),
            "complex" => Ok(Self::Complex),
            "exceptional" => Ok(Self::Exceptional),
            _ => Err("difficulty must be simple, standard, complex, or exceptional".to_string()),
        }
    }
}

impl fmt::Display for Difficulty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Simple => "simple",
            Self::Standard => "standard",
            Self::Complex => "complex",
            Self::Exceptional => "exceptional",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub model: String,
    pub effort: String,
}

impl Route {
    fn new(model: &str, effort: &str) -> Self {
        Self {
            model: model.to_string(),
            effort: effort.to_string(),
        }
    }
}

pub fn route(difficulty: Difficulty) -> Route {
    match difficulty {
        Difficulty::Simple => Route::new("gpt-5.6-luna", "low"),
        Difficulty::Standard => Route::new("gpt-5.6-terra", "medium"),
        Difficulty::Complex => Route::new("gpt-5.6-sol", "high"),
        Difficulty::Exceptional => Route::new("gpt-5.6-sol", "xhigh"),
    }
}

pub fn validate_route(model: &str, effort: &str) -> Result<(), String> {
    if !matches!(model, "gpt-5.6-luna" | "gpt-5.6-terra" | "gpt-5.6-sol") {
        return Err("model must be gpt-5.6-luna, gpt-5.6-terra, or gpt-5.6-sol".to_string());
    }
    if !matches!(effort, "low" | "medium" | "high" | "xhigh") {
        return Err(
            "effort must be low, medium, high, or xhigh; max and ultra are not allowed".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_every_difficulty() {
        assert_eq!(route(Difficulty::Simple), Route::new("gpt-5.6-luna", "low"));
        assert_eq!(
            route(Difficulty::Standard),
            Route::new("gpt-5.6-terra", "medium")
        );
        assert_eq!(
            route(Difficulty::Complex),
            Route::new("gpt-5.6-sol", "high")
        );
        assert_eq!(
            route(Difficulty::Exceptional),
            Route::new("gpt-5.6-sol", "xhigh")
        );
    }
    #[test]
    fn rejects_unbounded_effort() {
        assert!(validate_route("gpt-5.6-sol", "max").is_err());
        assert!(validate_route("gpt-5.6-sol", "ultra").is_err());
    }
}
