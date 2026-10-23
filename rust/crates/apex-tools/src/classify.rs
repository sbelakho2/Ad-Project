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

/// One exclusion with its glob precompiled.
#[derive(Debug)]
struct CompiledExclusion {
    matcher: Option<regex::Regex>,
    code: String,
}

/// One domain with its include/exclude globs precompiled. An invalid glob
/// compiles to `None` and never matches, exactly like the one-shot matcher.
#[derive(Debug)]
struct CompiledDomain {
    id: String,
    kind: String,
    include: Vec<Option<regex::Regex>>,
    exclude: Vec<Option<regex::Regex>>,
}

/// The classifier compiles every policy glob once at construction: the
/// manifest pass matches every file of the repository against every policy
/// pattern, and compiling a regex per (file, pattern) pair dominates the
/// gate's runtime.
#[derive(Debug)]
pub struct Classifier {
    exclusions: Vec<CompiledExclusion>,
    domains: Vec<CompiledDomain>,
}

impl Classifier {
    pub fn new(policies: Policies) -> Self {
        let exclusions = policies
            .exclusions
            .into_iter()
            .map(|exclusion| CompiledExclusion {
                matcher: glob::compile(&exclusion.glob),
                code: exclusion.code,
            })
            .collect();
        let domains = policies
            .domains
            .into_iter()
            .map(|domain| CompiledDomain {
                id: domain.id,
                kind: domain.kind,
                include: domain
                    .include
                    .iter()
                    .map(|glob| glob::compile(glob))
                    .collect(),
                exclude: domain
                    .exclude
                    .iter()
                    .map(|glob| glob::compile(glob))
                    .collect(),
            })
            .collect();
        Self {
            exclusions,
            domains,
        }
    }

    /// tools/coverage is the gate stack itself: exclusions never apply to it.
    pub fn classify(&self, rel: &str) -> Result<Classified, String> {
        let is_gate_stack = rel.starts_with("tools/coverage/");
        if !is_gate_stack {
            for ex in &self.exclusions {
                if ex
                    .matcher
                    .as_ref()
                    .is_some_and(|matcher| matcher.is_match(rel))
                {
                    return Ok(Classified {
                        domain: None,
                        kind: None,
                        status: "excluded".into(),
                        reason: Some(ex.code.clone()),
                    });
                }
            }
        }
        for d in &self.domains {
            let included = d
                .include
                .iter()
                .any(|matcher| matcher.as_ref().is_some_and(|m| m.is_match(rel)));
            if !included {
                continue;
            }
            if d.exclude
                .iter()
                .any(|matcher| matcher.as_ref().is_some_and(|m| m.is_match(rel)))
            {
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
            c.classify(".kiwi/pipeline.yaml").unwrap().domain.as_deref(),
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
    /// Regression: classification used to compile every policy glob for
    /// every (file, pattern) pair. The thread-local compile counter proves
    /// the compiled matchers are reused; this test fails on the pre-fix
    /// implementation where each classify call recompiles the patterns.
    #[test]
    fn classification_compiles_each_policy_glob_once() {
        let policies = Policies::load(&Repo::discover().expect("repo")).expect("policies");
        let classifier = Classifier::new(policies);
        let paths: Vec<String> = (0..200)
            .map(|i| format!("area{}/src/file_{i}.rs", i % 40))
            .collect();
        // Warm any lazily compiled state on this thread, then classify the
        // whole corpus and require zero further compilations.
        let _ = classifier.classify("area0/src/warm.rs").expect("classify");
        let before = crate::util::regex_compile_count();
        for path in &paths {
            let _ = classifier.classify(path).expect("classify");
        }
        assert_eq!(
            crate::util::regex_compile_count(),
            before,
            "classification must reuse the compiled policy globs"
        );
        let classified = classifier
            .classify("rust/crates/apex-core/src/money.rs")
            .expect("classify");
        assert_eq!(classified.domain.as_deref(), Some("apex-core"));
        assert_eq!(classified.status, "planned");
    }

    /// The policy globs are compiled once by `Classifier::new`, so the
    /// manifest's per-file cost is matching, never recompilation. Run the
    /// classifier at 13k and 26k paths and require zero compiles after the
    /// constructor at both sizes.
    #[test]
    fn classification_never_compiles_during_the_file_loop_at_13k_and_26k() {
        let repo = Repo::discover().expect("repo");
        let classifier = Classifier::new(Policies::load(&repo).expect("policies"));
        let _ = classifier.classify("docs/warm.md").expect("warm");
        let before = crate::util::regex_compile_count();
        for index in 0..13_000 {
            let path = match index % 4 {
                0 => format!("docs/page_{index}.md"),
                1 => format!("rust/crates/apex-core/src/file_{index}.rs"),
                2 => format!("tools/coverage/gate_{index}.rs"),
                _ => format!("site/public/asset_{index}.bin"),
            };
            let _ = classifier.classify(&path).expect("classify");
        }
        let after_n = crate::util::regex_compile_count();
        for index in 13_000..26_000 {
            let path = match index % 4 {
                0 => format!("docs/page_{index}.md"),
                1 => format!("rust/crates/apex-core/src/file_{index}.rs"),
                2 => format!("tools/coverage/gate_{index}.rs"),
                _ => format!("site/public/asset_{index}.bin"),
            };
            let _ = classifier.classify(&path).expect("classify");
        }
        let after_2n = crate::util::regex_compile_count();
        assert_eq!(
            after_n, before,
            "13k classifications compiled a policy glob"
        );
        assert_eq!(
            after_2n, after_n,
            "the second 13k classifications compiled a policy glob"
        );
    }

    #[test]
    fn classification_equals_the_naive_per_path_glob_scan() {
        let repo = Repo::discover().expect("repo");
        let classifier = Classifier::new(Policies::load(&repo).expect("policies"));
        let policies = Policies::load(&repo).expect("policies");
        // The reference evaluates the same patterns with matchers compiled
        // up front: the comparison is about classification order and
        // precedence, not about re-paying compilation per path.
        let naive_exclusions: Vec<(regex::Regex, String)> = policies
            .exclusions
            .iter()
            .filter_map(|ex| glob::compile(&ex.glob).map(|re| (re, ex.code.clone())))
            .collect();
        let naive_domains: Vec<(String, String, Vec<regex::Regex>, Vec<regex::Regex>)> = policies
            .domains
            .iter()
            .map(|d| {
                (
                    d.id.clone(),
                    d.kind.clone(),
                    d.include.iter().filter_map(|g| glob::compile(g)).collect(),
                    d.exclude.iter().filter_map(|g| glob::compile(g)).collect(),
                )
            })
            .collect();
        let naive = |rel: &str| -> Classified {
            let is_gate_stack = rel.starts_with("tools/coverage/");
            if !is_gate_stack {
                for (matcher, code) in &naive_exclusions {
                    if matcher.is_match(rel) {
                        return Classified {
                            domain: None,
                            kind: None,
                            status: "excluded".into(),
                            reason: Some(code.clone()),
                        };
                    }
                }
            }
            for (id, kind, include, exclude) in &naive_domains {
                let included = include.iter().any(|matcher| matcher.is_match(rel));
                if !included {
                    continue;
                }
                if exclude.iter().any(|matcher| matcher.is_match(rel)) {
                    continue;
                }
                return Classified {
                    domain: Some(id.clone()),
                    kind: Some(kind.clone()),
                    status: if kind == "code" {
                        "planned".into()
                    } else {
                        kind.clone()
                    },
                    reason: None,
                };
            }
            Classified {
                domain: None,
                kind: None,
                status: "unclassified".into(),
                reason: None,
            }
        };
        let paths: Vec<String> = (0..10_000)
            .map(|i| match i % 4 {
                0 => format!("docs/page_{i}.md"),
                1 => format!("rust/crates/apex-core/src/file_{i}.rs"),
                2 => format!("tools/coverage/gate_{i}.rs"),
                _ => format!("site/public/asset_{i}.bin"),
            })
            .collect();
        for path in &paths {
            let optimized = classifier.classify(path).expect("classify");
            let reference = naive(path);
            assert_eq!(optimized.domain, reference.domain, "{path}");
            assert_eq!(optimized.status, reference.status, "{path}");
            assert_eq!(optimized.reason, reference.reason, "{path}");
        }
    }
}
