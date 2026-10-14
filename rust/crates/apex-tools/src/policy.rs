//! Policy data loading (policies.json, floor/suite/baseline files) shared by
//! every subcommand.

use serde::Deserialize;
use serde_json::Value;

use crate::repo::Repo;

#[derive(Debug, Clone, Deserialize)]
pub struct Domain {
    pub id: String,
    pub kind: String,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Exclusion {
    pub glob: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct Policies {
    pub domains: Vec<Domain>,
    pub exclusions: Vec<Exclusion>,
    #[serde(rename = "exclusionCodes")]
    pub exclusion_codes: Vec<String>,
}

impl Policies {
    pub fn load(repo: &Repo) -> Result<Self, String> {
        let text = repo.read("tools/coverage/policies.json")?;
        serde_json::from_str(&text).map_err(|e| format!("policies.json: {e}"))
    }
}

#[derive(Debug, Clone)]
pub struct Floors {
    pub orphan_floor_percent: f64,
    pub mutation_kill_floor_percent: f64,
    pub domains: std::collections::BTreeMap<String, f64>,
    pub history: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    #[serde(default)]
    pub orphan_floor_percent: f64,
    #[serde(default)]
    pub mutation_kill_floor_percent: f64,
    #[serde(default)]
    pub domains: std::collections::BTreeMap<String, f64>,
    #[serde(default)]
    pub at: String,
}

impl Floors {
    pub fn load(repo: &Repo) -> Result<Self, String> {
        let text = repo.read("tools/coverage/coverage-floor.json")?;
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("coverage-floor.json: {e}"))?;
        let domains = v
            .get("domains")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, val)| val.as_f64().map(|f| (k.clone(), f)))
                    .collect()
            })
            .unwrap_or_default();
        let history = v
            .get("history")
            .and_then(|h| serde_json::from_value(h.clone()).ok())
            .unwrap_or_default();
        Ok(Self {
            orphan_floor_percent: v
                .get("orphanFloorPercent")
                .and_then(Value::as_f64)
                .unwrap_or(100.0),
            mutation_kill_floor_percent: v
                .get("mutationKillFloorPercent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            domains,
            history,
        })
    }

    pub fn floor_for(&self, domain: &str) -> f64 {
        self.domains.get(domain).copied().unwrap_or(0.0)
    }
}

/// Simple `domain value` whitespace file (floors.txt mirror).
pub fn read_floors_txt(repo: &Repo) -> Result<std::collections::BTreeMap<String, f64>, String> {
    let text = repo.read("tools/coverage/floors.txt")?;
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let mut parts = t.split_whitespace();
        if let (Some(d), Some(v)) = (parts.next(), parts.next()) {
            map.insert(d.to_string(), v.parse::<f64>().map_err(|e| e.to_string())?);
        }
    }
    Ok(map)
}

pub fn read_suites(repo: &Repo) -> Result<Vec<Vec<String>>, String> {
    let text = repo.read("tools/coverage/suites.json")?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let suites = v
        .get("suites")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for suite in suites {
        let covers = suite
            .get("covers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.push(covers);
    }
    Ok(out)
}

/// Policy baseline: pragma/skip/orTrue file lists plus the shell list.
#[derive(Debug, Default, Deserialize)]
pub struct PolicyBaseline {
    #[serde(default)]
    pub shell: Vec<String>,
    #[serde(default)]
    pub pragma: Vec<String>,
    #[serde(default)]
    pub skip: Vec<String>,
    #[serde(default, rename = "orTrue")]
    pub or_true: Vec<String>,
}

impl PolicyBaseline {
    pub fn load(repo: &Repo) -> Result<Self, String> {
        let path = "tools/coverage/policy-baseline.json";
        if !repo.exists(path) {
            return Ok(Self::default());
        }
        serde_json::from_str(&repo.read(path)?).map_err(|e| format!("policy-baseline.json: {e}"))
    }
}

/// CI-reference baseline: workflow -> list of known-missing refs.
#[derive(Debug, Default, Deserialize)]
pub struct CiRefBaseline {
    #[serde(default, rename = "knownMissing")]
    pub known_missing: std::collections::BTreeMap<String, String>,
}

impl CiRefBaseline {
    pub fn load(repo: &Repo) -> Result<Self, String> {
        let path = "tools/coverage/ci-ref-baseline.json";
        if !repo.exists(path) {
            return Ok(Self::default());
        }
        serde_json::from_str(&repo.read(path)?).map_err(|e| format!("ci-ref-baseline.json: {e}"))
    }
}
