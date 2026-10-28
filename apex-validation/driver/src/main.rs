//! Read JSONL fixtures and call the unmodified production crate.
use std::io::{self, BufRead};
use apex_lift::{stats, holdout};
use serde_json::{Value, json};

fn values(v: &Value, key: &str) -> Vec<f64> {
    v[key].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()
}
fn process(v: &Value) -> Value {
    let c = values(v, "control");
    let t = values(v, "test");
    if v["kind"] == "ht" {
        match stats::horvitz_thompson_estimate(&c, &t, v["q"].as_f64().unwrap()) {
            Some(e) => json!({"lift":e.estimate.lift,"mean_log":e.estimate.mean_log,
                "variance":e.variance,"lower90":e.estimate.lower_90,"lower99":e.estimate.lower_99}),
            None => Value::Null,
        }
    } else {
        let mut players = Vec::new();
        for (exposed, group) in [(false, &c), (true, &t)] {
            for (i, y) in group.iter().enumerate() {
                players.push(holdout::PlayerRevenue {
                    user: format!("{exposed}-{i}"), revenue_micros: y.round() as i64,
                    pre_revenue_micros: None, exposed,
                });
            }
        }
        match holdout::estimate(&players) {
            Some(e) => {
                let scope = v["scope_revenue"].as_i64().unwrap_or(0);
                let change = apex_lift::billing::KeptChange {
                    change_id:"synthetic-witness".into(), lambda:0., rate_bp:3000,
                    kept_day:"2026-09-01".into(), overridden_day:None, rolled_back_day:None,
                    pre_registration_hash:"synthetic-prereg".into(), result_hash:"synthetic-result".into(),
                };
                let monthly = apex_lift::run::run_holdout_month("2026-10","2026-10-01",31,scope,
                    &[apex_lift::run::HoldoutMonthChange { change, estimate:Some(e.clone()),
                        ledger_references:vec![], evidence:None }]).unwrap();
                json!({"lift":e.lift,"se":e.standard_error,"lower90":e.lower_90,
                    "split_ok":holdout::split_is_90_10(&e),
                    "scope_fee_micros":holdout::monthly_fee_on_scope(scope,&e),
                    "actual_treated_fee_micros":holdout::monthly_fee(&e),
                    "monthly_run_fee_micros":monthly.fees[0].fee_micros})
            },
            None => Value::Null,
        }
    }
}
fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        println!("{}", process(&v));
    }
}
