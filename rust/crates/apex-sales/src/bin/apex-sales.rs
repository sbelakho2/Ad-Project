//! `apex-sales`: ApexMediation's outbound sales engine. See
//! `apex_sales::cli` for commands and configuration.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use apex_sales::cli::{mailer_from_env, run, Env};
use apex_sales::io::CurlFetcher;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env: Env = std::env::vars()
        .filter(|(k, _)| k.starts_with("APEX_SALES_"))
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let mailer = mailer_from_env(&env);
    match run(&args, &env, now, &CurlFetcher::default(), mailer.as_ref()) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("apex-sales: {e}");
            ExitCode::from(2)
        }
    }
}
