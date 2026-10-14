//! Classification + manifest, ported from coverage-lib.mjs/build-coverage-manifest.mjs.

use serde::{Deserialize, Serialize};

use crate::glob;
use crate::policy::Policies;
use crate::repo::Repo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub domain: Option<String>,
    pub kind: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub suite: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub policy: String,
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    pub counts: std::collections::BTreeMap<String, u64>,
    pub files: Vec<ManifestFile>,
}

#[derive(Debug)]
pub struct Classified {
    pub domain: Option<String>,
    pub kind: Option<String>,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug)]
pub struct Classifier {
    policies: Policies,
}

impl Classifier {
    pub fn new(policies: Policies) -> Self {
        Self { policies }
    }

    /// tools/coverage is the gate stack itself: exclusions never apply to it.
    pub fn classify(&self, rel: &str) -> Result<Classified, String> {
        let is_gate_stack = rel.starts_with("tools/coverage/");
        if !is_gate_stack {
            for ex in &self.policies.exclusions {
                if glob::matches(&ex.glob, rel) {
                    return Ok(Classified {
                        domain: None,
                        kind: None,
                        status: "excluded".into(),
                        reason: Some(ex.code.clone()),
                    });
                }
            }
        }
        for d in &self.policies.domains {
            let included = d.include.iter().any(|g| glob::matches(g, rel));
            if !included {
                continue;
            }
            if d.exclude.iter().any(|g| glob::matches(g, rel)) {
                continue;
            }
            return Ok(Classified {
                domain: Some(d.id.clone()),
                kind: Some(d.kind.clone()),
                status: if d.kind == "code" {
                    "planned".into()
                } else {
                    d.kind.clone()
                },
                reason: None,
            });
        }
        Ok(Classified {
            domain: None,
            kind: None,
            status: "unclassified".into(),
            reason: None,
        })
    }
}

pub fn build_manifest(repo: &Repo) -> Result<Manifest, String> {
    let policies = Policies::load(repo)?;
    let classifier = Classifier::new(policies);
    let mut files = Vec::new();
    for path in repo.walk() {
        let c = classifier.classify(&path)?;
        if c.status == "excluded" {
            let reason = c.reason.clone().unwrap_or_default();
            // Closed set: a typo in a reason code must fail loudly.
            if ![
                "VENDOR",
                "GENERATED",
                "BUILD_ARTIFACT",
                "SAMPLE_APP",
                "DATA_ARTIFACT",
                "NON_EXECUTABLE",
                "FUTURE_SURFACE",
            ]
            .contains(&reason.as_str())
            {
                return Err(format!("unknown exclusion reason '{reason}' for {path}"));
            }
        }
        files.push(ManifestFile {
            path,
            domain: c.domain,
            kind: c.kind,
            status: c.status,
            reason: c.reason,
            suite: None,
        });
    }
    let unclassified: Vec<&str> = files
        .iter()
        .filter(|f| f.status == "unclassified")
        .map(|f| f.path.as_str())
        .take(40)
        .collect();
    if !unclassified.is_empty() {
        return Err(format!(
            "{} unclassified files, first: {}",
            files.iter().filter(|f| f.status == "unclassified").count(),
            unclassified.join(", ")
        ));
    }
    let mut counts = std::collections::BTreeMap::new();
    for f in &files {
        *counts.entry(f.status.clone()).or_insert(0) += 1;
    }
    let generated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    Ok(Manifest {
        policy: "docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md".into(),
        generated_at,
        counts,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_matches_policy_expectations() {
        let repo = Repo::discover().expect("repo");
        let policies = Policies::load(&repo).expect("policies");
        let c = Classifier::new(policies);
        // Product crates are code domains.
        assert_eq!(
            c.classify("rust/crates/apex-core/src/money.rs")
                .unwrap()
                .domain
                .as_deref(),
            Some("apex-core")
        );
        assert_eq!(
            c.classify("rust/crates/apex-connectors/src/max.rs")
                .unwrap()
                .domain
                .as_deref(),
            Some("apex-connectors")
        );
        // Fixtures and test paths land in the meta-test domain.
        let helper = c
            .classify("rust/crates/apex-api/fixtures/admob.json")
            .unwrap();
        assert_eq!(helper.domain.as_deref(), Some("meta-test"));
        assert_eq!(helper.status, "test");
        // Docs and root markdown.
        assert_eq!(
            c.classify("docs/INDEX.md").unwrap().domain.as_deref(),
            Some("docs")
        );
        assert_eq!(
            c.classify("README.md").unwrap().domain.as_deref(),
            Some("docs")
        );
        // Contract domains: pipelines, monitoring rules, build files.
        assert_eq!(
            c.classify(".woodpecker/dogfood-audit.yml")
                .unwrap()
                .domain
                .as_deref(),
            Some("contract-iac")
        );
        assert_eq!(
            c.classify("monitoring/alerts.yml")
                .unwrap()
                .domain
                .as_deref(),
            Some("contract-iac")
        );
        assert_eq!(
            c.classify("tools/coverage/policies.json")
                .unwrap()
                .domain
                .as_deref(),
            Some("contract-build")
        );
        // Reason codes: a built page and an image are excluded, never owned.
        assert_eq!(
            c.classify("site/public/404.html")
                .unwrap()
                .reason
                .as_deref(),
            Some("BUILD_ARTIFACT")
        );
        assert_eq!(
            c.classify("logo.jpg").unwrap().reason.as_deref(),
            Some("NON_EXECUTABLE")
        );
    }
}
