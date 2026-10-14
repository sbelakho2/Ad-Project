//! Two-way claims contract, ported from claims-contract.mjs: every claim in
//! site/claims.json must be implementable evidence (anchor path exists, pinned
//! test name present in the cited file) and the claim text must literally
//! appear on a site page after whitespace normalization.

use serde_json::Value;

use crate::repo::Repo;
use crate::util::re;

pub fn normalize(text: &str) -> Result<String, String> {
    Ok(re(r"\s+")?.replace_all(text, " ").trim().to_lowercase())
}

fn claim_str(claim: &Value, field: &str) -> Option<String> {
    claim.get(field).and_then(Value::as_str).map(str::to_string)
}

/// Pure: validate a parsed claims file against a virtual fs surface.
pub fn check_claims(
    claims: &Value,
    exists: &dyn Fn(&str) -> bool,
    read_file: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    let Some(list) = claims.get("claims").and_then(Value::as_array) else {
        return Ok(vec!["claims.json has no claims array".to_string()]);
    };
    if list.is_empty() {
        return Ok(vec!["claims.json has no claims array".to_string()]);
    }
    let pinned_re = re(r"^(.*?)\s*\(([^)]+)\)\s*$")?;
    let mut violations = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for claim in list {
        let id = claim
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("<missing id>")
            .to_string();
        if seen.contains(&id) {
            violations.push(format!("{id}: duplicate claim id"));
        }
        seen.insert(id.clone());
        let text = claim_str(claim, "claim").filter(|s| !s.is_empty());
        let anchor = claim_str(claim, "anchor").filter(|s| !s.is_empty());
        let test = claim_str(claim, "test").filter(|s| !s.is_empty());
        let (Some(_text), Some(anchor), Some(test)) = (text, anchor, test) else {
            violations.push(format!("{id}: claim/anchor/test are all required"));
            continue;
        };
        if !exists(&anchor) {
            violations.push(format!("{id}: anchor path does not exist: {anchor}"));
            continue;
        }
        let (test_name, test_path) = match pinned_re.captures(&test) {
            Some(c) => (c[1].trim().to_string(), c[2].trim().to_string()),
            None => (test.trim().to_string(), anchor.clone()),
        };
        if !exists(&test_path) {
            violations.push(format!(
                "{id}: pinned test file does not exist: {test_path}"
            ));
            continue;
        }
        match read_file(&test_path) {
            Err(error) => violations.push(format!("{id}: pinned test file unreadable: {error}")),
            Ok(test_text) => {
                if !normalize(&test_text)?.contains(&normalize(&test_name)?) {
                    violations.push(format!(
                        "{id}: pinned test '{test_name}' not found in {test_path}"
                    ));
                }
            }
        }
    }
    Ok(violations)
}

/// Pure: every claim text must appear somewhere in the site pages.
pub fn check_pages(claims: &Value, pages: &[String]) -> Result<Vec<String>, String> {
    let mut violations = Vec::new();
    let haystack = normalize(&pages.join("\n"))?;
    let empty = Vec::new();
    let list = claims
        .get("claims")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    for claim in list {
        let Some(text) = claim_str(claim, "claim").filter(|s| !s.is_empty()) else {
            continue;
        };
        if !haystack.contains(&normalize(&text)?) {
            let id = claim
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("<missing id>");
            violations.push(format!(
                "{id}: claim text not found on any page: {}",
                serde_json::to_string(&text).unwrap_or_default()
            ));
        }
    }
    Ok(violations)
}

fn page_files(repo: &Repo) -> Vec<String> {
    let mut out = Vec::new();
    for (dir, ext) in [("site/content", "md"), ("site/templates", "html")] {
        let root = repo.path(dir);
        for file in crate::util::walk_files(&root, &[]) {
            if file.extension().map(|e| e == ext).unwrap_or(false) {
                out.push(file.to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

pub fn claims_contract(repo: &Repo) -> Result<(), String> {
    let claims_path = "site/claims.json";
    if !repo.exists(claims_path) {
        return Err("claims-contract: site/claims.json is missing".into());
    }
    let text = repo.read(claims_path)?;
    let claims: Value = serde_json::from_str(&text).map_err(|e| format!("{claims_path}: {e}"))?;
    let exists = |p: &str| repo.exists(p);
    let read = |p: &str| repo.read(p);
    let mut violations = check_claims(&claims, &exists, &read)?;

    let files = page_files(repo);
    if files.is_empty() {
        violations.push("no site content/templates found to verify claim text".to_string());
    }
    let mut pages = Vec::new();
    for f in &files {
        let rel = std::path::Path::new(f)
            .strip_prefix(&repo.root)
            .map(crate::util::slash)
            .unwrap_or_else(|_| f.clone());
        pages.push(repo.read(&rel)?);
    }
    violations.extend(check_pages(&claims, &pages)?);

    if !violations.is_empty() {
        return Err(format!(
            "CLAIMS CONTRACT FAILED ({}):\n  - {}",
            violations.len(),
            violations.join("\n  - ")
        ));
    }
    let count = claims
        .get("claims")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    println!("claims contract PASS: {count} claims anchored and present on the site");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_collapses_whitespace_and_case() {
        assert_eq!(normalize(" A  b\n C ").unwrap(), "a b c");
    }

    #[test]
    fn missing_anchor_path_and_missing_test_token_are_violations() {
        let claims = json!({"claims": [
            {"id": "a1", "claim": "we do x", "anchor": "no/such/file.rs", "test": "t"},
            {"id": "a2", "claim": "we do y", "anchor": "exists.rs", "test": "settle_keeps_positive_lift"},
        ]});
        let exists = |p: &str| p == "exists.rs";
        let read = |_p: &str| Ok("fn other_test() {}".to_string());
        let v = check_claims(&claims, &exists, &read).unwrap();
        assert!(v
            .iter()
            .any(|m| m.starts_with("a1: anchor path does not exist")));
        assert!(v
            .iter()
            .any(|m| m.contains("pinned test 'settle_keeps_positive_lift' not found")));
    }

    #[test]
    fn cross_file_pin_name_path_resolves_against_the_cited_file() {
        let claims = json!({"claims": [
            {"id": "x1", "claim": "c", "anchor": "impl.rs", "test": "my_test (tests/it.rs)"},
        ]});
        let exists = |p: &str| p == "impl.rs" || p == "tests/it.rs";
        let read = |p: &str| {
            Ok(if p == "tests/it.rs" {
                "fn my_test() {}"
            } else {
                "// no test here"
            }
            .to_string())
        };
        assert_eq!(
            check_claims(&claims, &exists, &read).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn cross_file_pin_with_missing_or_empty_test_file_is_a_violation() {
        let claims = json!({"claims": [
            {"id": "x2", "claim": "c", "anchor": "impl.rs", "test": "my_test (tests/nope.rs)"},
            {"id": "x3", "claim": "c", "anchor": "impl.rs", "test": "my_test (tests/empty.rs)"},
        ]});
        let exists = |p: &str| p == "impl.rs" || p == "tests/empty.rs";
        let read = |_p: &str| Ok("other content".to_string());
        let v = check_claims(&claims, &exists, &read).unwrap();
        assert!(v
            .iter()
            .any(|m| m.contains("pinned test file does not exist: tests/nope.rs")));
        assert!(v
            .iter()
            .any(|m| m.contains("pinned test 'my_test' not found in tests/empty.rs")));
    }

    #[test]
    fn duplicate_ids_are_violations() {
        let claims = json!({"claims": [
            {"id": "d", "claim": "x", "anchor": "e.rs", "test": "t"},
            {"id": "d", "claim": "y", "anchor": "e.rs", "test": "t"},
        ]});
        let exists = |_p: &str| true;
        let read = |_p: &str| Ok("t".to_string());
        let v = check_claims(&claims, &exists, &read).unwrap();
        assert!(v.iter().any(|m| m.contains("duplicate claim id")));
    }

    #[test]
    fn claim_text_missing_from_pages_is_a_violation_and_present_passes() {
        let claims = json!({"claims": [{"id": "p", "claim": "No SDK in your app.", "anchor": "a", "test": "t"}]});
        let v = check_pages(&claims, &["unrelated".to_string()]).unwrap();
        assert!(v.iter().any(|m| m.contains("not found on any page")));
        assert_eq!(
            check_pages(
                &claims,
                &["intro: No SDK in   your app. indeed".to_string()]
            )
            .unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn empty_claims_array_is_a_violation() {
        let exists = |_p: &str| true;
        let read = |_p: &str| Ok(String::new());
        assert_eq!(
            check_claims(&json!({"claims": []}), &exists, &read).unwrap(),
            vec!["claims.json has no claims array".to_string()]
        );
    }
}
