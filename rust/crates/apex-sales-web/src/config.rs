//! The service configuration: the owner token, the bind address, the sales
//! state directory and the captured environment the engine reads through
//! [`apex_sales::cli::config_from_env`].
//!
//! | variable | meaning | default |
//! |---|---|---|
//! | `APEX_SALES_WEB_BIND` | listen address | `127.0.0.1:8795` |
//! | `APEX_SALES_WEB_TOKEN` | owner gate; **required** to serve any page | none (fail closed) |
//! | `APEX_SALES_DIR` | engine state directory | `.apex/sales` |
//! | `APEX_SALES_OUTBOX` | `.eml` outbox directory (else `$APEX_SALES_DIR/outbox` in shadow/off mode) | none |
//!
//! Every `APEX_SALES_*` variable the CLI documents (`APEX_SALES_MODE`,
//! `APEX_SALES_FROM`, ...) is read through the same
//! [`apex_sales::cli::config_from_env`] the CLI uses, so the web UI plans
//! and reports with exactly the engine's own configuration.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use apex_sales::cli::Env;

/// The default bind address.
pub const DEFAULT_BIND: &str = "127.0.0.1:8795";
/// The default engine state directory.
pub const DEFAULT_SALES_DIR: &str = ".apex/sales";

/// Everything the process needs before it can serve.
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// The address served.
    pub bind: SocketAddr,
    /// The owner token, when configured. `None` starts the not-configured
    /// state: no page beyond the explanation, and no session can be minted.
    pub token: Option<String>,
    /// The engine state directory.
    pub sales_dir: PathBuf,
    /// The captured environment map handed to the engine's own loaders.
    pub env: Env,
}

impl WebConfig {
    /// Build from an environment map; refuses an unparsable bind address.
    pub fn from_env(env: &Env) -> Result<Self, String> {
        let bind = env
            .get("APEX_SALES_WEB_BIND")
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_BIND);
        let bind: SocketAddr = bind
            .parse()
            .map_err(|error| format!("APEX_SALES_WEB_BIND: {error}"))?;
        let token = env
            .get("APEX_SALES_WEB_TOKEN")
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let sales_dir = env
            .get("APEX_SALES_DIR")
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_SALES_DIR);
        Ok(Self {
            bind,
            token,
            sales_dir: PathBuf::from(sales_dir),
            env: env.clone(),
        })
    }

    /// Capture the current process environment.
    pub fn capture() -> Result<Self, String> {
        let env: Env = std::env::vars().collect::<BTreeMap<String, String>>();
        Self::from_env(&env)
    }

    /// Whether the owner gate is configured.
    pub fn is_configured(&self) -> bool {
        self.token.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_values() {
        let config = WebConfig::from_env(&Env::new()).unwrap();
        assert_eq!(config.bind, "127.0.0.1:8795".parse().unwrap());
        assert_eq!(config.sales_dir, PathBuf::from(DEFAULT_SALES_DIR));
        assert!(config.token.is_none());
        assert!(!config.is_configured());
    }

    #[test]
    fn values_come_from_the_environment_with_whitespace_tolerated() {
        let mut env = Env::new();
        env.insert("APEX_SALES_WEB_BIND".into(), "0.0.0.0:9000".into());
        env.insert("APEX_SALES_WEB_TOKEN".into(), "  s3cret  ".into());
        env.insert("APEX_SALES_DIR".into(), "/tmp/sales".into());
        let config = WebConfig::from_env(&env).unwrap();
        assert_eq!(config.bind, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(config.token.as_deref(), Some("s3cret"));
        assert_eq!(config.sales_dir, PathBuf::from("/tmp/sales"));
        assert!(config.is_configured());
    }

    #[test]
    fn blank_token_and_blank_dir_fall_back() {
        let mut env = Env::new();
        env.insert("APEX_SALES_WEB_TOKEN".into(), "   ".into());
        env.insert("APEX_SALES_DIR".into(), String::new());
        let config = WebConfig::from_env(&env).unwrap();
        assert!(config.token.is_none());
        assert_eq!(config.sales_dir, PathBuf::from(DEFAULT_SALES_DIR));
    }

    #[test]
    fn a_broken_bind_address_is_refused() {
        let mut env = Env::new();
        env.insert("APEX_SALES_WEB_BIND".into(), "not-an-address".into());
        assert!(WebConfig::from_env(&env).is_err());
    }
}
