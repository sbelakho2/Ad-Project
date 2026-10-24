//! MAX S2S impression postbacks (A3 D1). AppLovin does not retry, so the
//! handler validates, appends one durable line and answers immediately. The
//! raw payload is kept; the lift runner reduces it later.

use std::io::Write;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Postback {
    #[serde(alias = "userId", alias = "user_id")]
    user_id: String,
    #[serde(alias = "adUnitId", alias = "ad_unit", alias = "ad_unit_id")]
    ad_unit: String,
    revenue: f64,
    #[serde(default, alias = "testName", alias = "adUnitTestName")]
    test_name: Option<String>,
    #[serde(default)]
    time: Option<String>,
}

/// Append one postback to the spool. The path is deployment configuration.
pub fn append(path: &Path, body: &[u8]) -> Result<usize, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| format!("postback JSON: {error}"))?;
    let rows = match value {
        serde_json::Value::Array(rows) => rows,
        value => vec![value],
    };
    if rows.is_empty() || rows.len() > 500 {
        return Err("postback batch must contain 1..=500 rows".into());
    }
    let mut lines = String::new();
    for row in rows {
        let postback: Postback =
            serde_json::from_value(row).map_err(|error| format!("postback shape: {error}"))?;
        if postback.user_id.is_empty()
            || postback.ad_unit.is_empty()
            || !postback.revenue.is_finite()
        {
            return Err("postback needs user id, ad unit and finite revenue".into());
        }
        let line = serde_json::json!({
            "user_id": postback.user_id,
            "ad_unit": postback.ad_unit,
            "revenue_micros": (postback.revenue * 1_000_000.0).round() as i64,
            "test_name": postback.test_name,
            "time": postback.time,
        });
        lines.push_str(&line.to_string());
        lines.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("postback dir: {error}"))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("postback spool: {error}"))?;
    file.write_all(lines.as_bytes())
        .map_err(|error| format!("postback write: {error}"))?;
    Ok(lines.lines().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_one_line_per_impression() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("max.jsonl");
        let body = br#"[{"userId":"u1","adUnitId":"rw","revenue":0.25,"testName":"a"},{"userId":"u1","adUnitId":"rw","revenue":0.5}]"#;
        assert_eq!(append(&path, body).unwrap(), 2);
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\"revenue_micros\":250000"));
    }

    #[test]
    fn refuses_a_bad_shape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(append(&dir.path().join("x"), br#"{"user_id":""}"#).is_err());
    }
}
