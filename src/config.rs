use std::collections::HashMap;

use regex::Regex;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Wiki root without a trailing slash, e.g. `https://wiki.example.com` (a base path is kept).
    pub base_url: String,
    /// Empty when the wiki runs without sign-in (`--disable-auth`).
    pub username: String,
    pub password: String,
    /// Register only the `read_` tools.
    pub read_only: bool,
}

const LOCAL: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];
const TRUE: [&str; 4] = ["1", "true", "yes", "on"];
const FALSE: [&str; 5] = ["0", "false", "no", "off", ""];

/// Reads and validates the environment. Fails with every problem at once.
pub fn load_config(env: &HashMap<String, String>) -> Result<Config, String> {
    let get = |name: &str| env.get(name).map(String::as_str);
    let mut problems: Vec<String> = Vec::new();

    // A client that cannot find a variable may pass its reference on as text (Claude Code does).
    let unexpanded = Regex::new(r"^(\$\{[^}]*\}|\{env:[^}]*\})$").expect("valid regex");
    for name in ["LEAFWIKI_USERNAME", "LEAFWIKI_PASSWORD"] {
        let value = get(name).unwrap_or("").trim();
        if unexpanded.is_match(value) {
            problems.push(format!(
                "{name} is the unexpanded \"{value}\": set that variable where the MCP client starts, then restart the client"
            ));
        }
    }

    let raw_url = get("LEAFWIKI_URL").map(str::trim).unwrap_or("");
    let mut base_url = String::new();
    if raw_url.is_empty() {
        problems.push("LEAFWIKI_URL is required, e.g. https://wiki.example.com".into());
    } else {
        match Url::parse(raw_url) {
            Err(_) => problems.push(format!("LEAFWIKI_URL is not a valid URL: \"{raw_url}\"")),
            Ok(url) => {
                let host = url.host_str().unwrap_or("");
                let api = Regex::new(r"(?i)/api(/|$)").expect("valid regex");
                if !url.username().is_empty() || url.password().is_some() {
                    // Not echoed: it holds a password.
                    problems.push(
                        "LEAFWIKI_URL must not contain a user name or password; use LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD".into(),
                    );
                } else if url.scheme() != "https"
                    && !(url.scheme() == "http" && LOCAL.contains(&host))
                {
                    problems.push(format!(
                        "LEAFWIKI_URL must be an https URL (the sign-in cookies travel with every request), got \"{raw_url}\""
                    ));
                } else if api.is_match(url.path()) {
                    problems.push(format!(
                        "LEAFWIKI_URL must be the wiki root without /api, got \"{raw_url}\""
                    ));
                } else if url.query().is_some_and(|q| !q.is_empty())
                    || url.fragment().is_some_and(|f| !f.is_empty())
                {
                    problems
                        .push("LEAFWIKI_URL must not contain a query string or fragment".into());
                } else {
                    let joined = format!("{}{}", url.origin().ascii_serialization(), url.path());
                    base_url = joined.trim_end_matches('/').to_string();
                }
            }
        }
    }

    let username = get("LEAFWIKI_USERNAME").unwrap_or("").trim().to_string();
    let password = get("LEAFWIKI_PASSWORD").unwrap_or("").to_string();
    if username.is_empty() != password.is_empty() {
        problems.push(
            "Set both LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD, or neither when the wiki runs without sign-in".into(),
        );
    }

    let raw_read_only = get("BURROW_READ_ONLY").unwrap_or("");
    let read_only_value = raw_read_only.trim().to_lowercase();
    let mut read_only = false;
    if TRUE.contains(&read_only_value.as_str()) {
        read_only = true;
    } else if !FALSE.contains(&read_only_value.as_str()) {
        problems.push(format!(
            "BURROW_READ_ONLY must be true or false, got \"{raw_read_only}\""
        ));
    }

    if !problems.is_empty() {
        return Err(format!(
            "Invalid burrow configuration:\n- {}",
            problems.join("\n- ")
        ));
    }
    Ok(Config {
        base_url,
        username,
        password,
        read_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const BASE: [(&str, &str); 3] = [
        ("LEAFWIKI_URL", "https://wiki.example.com/"),
        ("LEAFWIKI_USERNAME", "ed"),
        ("LEAFWIKI_PASSWORD", "pw"),
    ];

    fn with(extra: &[(&str, &str)]) -> HashMap<String, String> {
        let mut e = env(&BASE);
        e.extend(env(extra));
        e
    }

    fn problems(e: HashMap<String, String>) -> String {
        load_config(&e).err().unwrap_or_default()
    }

    #[test]
    fn keeps_the_wiki_root_and_a_base_path_without_trailing_slashes() {
        assert_eq!(
            load_config(&env(&BASE)).unwrap(),
            Config {
                base_url: "https://wiki.example.com".into(),
                username: "ed".into(),
                password: "pw".into(),
                read_only: false
            }
        );
        assert_eq!(
            load_config(&with(&[("LEAFWIKI_URL", "https://example.com/wiki/")]))
                .unwrap()
                .base_url,
            "https://example.com/wiki"
        );
        assert_eq!(
            load_config(&with(&[("LEAFWIKI_URL", "http://localhost:8080")]))
                .unwrap()
                .base_url,
            "http://localhost:8080"
        );
        assert_eq!(
            load_config(&with(&[("LEAFWIKI_URL", "http://[::1]:8080/")]))
                .unwrap()
                .base_url,
            "http://[::1]:8080"
        );
    }

    #[test]
    fn refuses_plain_http_off_localhost_an_api_url_half_the_credentials_and_an_unclear_read_only_value()
     {
        assert!(
            problems(with(&[("LEAFWIKI_URL", "http://wiki.example.com")]))
                .contains("must be an https URL")
        );
        assert!(
            problems(with(&[("LEAFWIKI_URL", "https://wiki.example.com/api")]))
                .contains("without /api")
        );
        assert!(
            problems(env(&[
                ("LEAFWIKI_URL", "https://wiki.example.com/"),
                ("LEAFWIKI_USERNAME", "ed")
            ]))
            .contains("Set both")
        );
        assert!(problems(with(&[("BURROW_READ_ONLY", "maybe")])).contains("BURROW_READ_ONLY"));
        assert!(problems(env(&[])).contains("LEAFWIKI_URL is required"));
        for url in [
            "https://ed:s3cret@wiki.example.com",
            "http://ed:s3cret@wiki.example.com/api",
        ] {
            let message = problems(with(&[("LEAFWIKI_URL", url)]));
            assert!(message.contains("LEAFWIKI_URL must not contain a user name or password"));
            assert!(!message.contains("s3cret"));
        }
        assert!(
            problems(with(&[("LEAFWIKI_PASSWORD", "${LEAFWIKI_PASSWORD}")]))
                .contains("LEAFWIKI_PASSWORD is the unexpanded \"${LEAFWIKI_PASSWORD}\"")
        );
        assert!(
            problems(with(&[("LEAFWIKI_USERNAME", "{env:LEAFWIKI_USERNAME}")]))
                .contains("LEAFWIKI_USERNAME is the unexpanded")
        );
        assert!(
            problems(with(&[("LEAFWIKI_URL", "https://wiki.example.com/?x=1")]))
                .contains("query string")
        );
    }

    #[test]
    fn read_only_accepts_the_usual_spellings() {
        assert!(
            load_config(&with(&[("BURROW_READ_ONLY", "Yes")]))
                .unwrap()
                .read_only
        );
        assert!(
            !load_config(&with(&[("BURROW_READ_ONLY", "0")]))
                .unwrap()
                .read_only
        );
    }
}
