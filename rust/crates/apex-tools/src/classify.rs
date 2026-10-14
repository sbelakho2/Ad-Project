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
    fn classifier_matches_js_expectations() {
        let repo = Repo::discover().expect("repo");
        let policies = Policies::load(&repo).expect("policies");
        let c = Classifier::new(policies);
        assert_eq!(
            c.classify("backend/src/services/billing/x.ts")
                .unwrap()
                .domain
                .as_deref(),
            Some("backend")
        );
        let helper = c.classify("backend/tests/helpers/testApp.ts").unwrap();
        assert_eq!(helper.domain.as_deref(), Some("meta-test"));
        assert_eq!(helper.status, "test");
        // root files
        assert_eq!(
            c.classify("package.json").unwrap().domain.as_deref(),
            Some("contract-build")
        );
        assert_eq!(
            c.classify("README.md").unwrap().domain.as_deref(),
            Some("docs")
        );
        // reason codes
        assert_eq!(
            c.classify("backend/src/types/foo.d.ts")
                .unwrap()
                .reason
                .as_deref(),
            Some("GENERATED")
        );
        assert_eq!(
            c.classify("website/public/logo.png")
                .unwrap()
                .reason
                .as_deref(),
            Some("NON_EXECUTABLE")
        );
        assert_eq!(
            c.classify("models/fraud/latest").unwrap().reason.as_deref(),
            Some("DATA_ARTIFACT")
        );
    }
}
