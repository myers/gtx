use clap::Args;
use eyre::Result;

use crate::config::Config;
use crate::repo;

#[derive(Args)]
pub struct ApiCommand {
    /// API endpoint path (e.g., repos/{owner}/{repo}/issues)
    endpoint: String,

    /// HTTP method
    #[arg(short = 'X', long = "method")]
    method: Option<String>,

    /// Add a string parameter (key=value)
    #[arg(short = 'f', long = "raw-field", value_name = "KEY=VALUE")]
    raw_fields: Vec<String>,

    /// Add a typed parameter (key=value). Values "true", "false", "null",
    /// and integers are converted to their JSON types. @file reads from file.
    #[arg(short = 'F', long = "field", value_name = "KEY=VALUE")]
    fields: Vec<String>,

    /// Add a request header (key:value)
    #[arg(short = 'H', long = "header", value_name = "KEY:VALUE")]
    headers: Vec<String>,

    /// Include HTTP response headers in output
    #[arg(short = 'i', long = "include")]
    include: bool,

    /// Silence response body
    #[arg(long)]
    silent: bool,

    /// Filter JSON output with a jq expression
    #[arg(long = "jq", value_name = "EXPR")]
    jq_expr: Option<String>,

    /// Make additional requests to fetch all pages
    #[arg(long)]
    paginate: bool,

    /// Print the equivalent curl command (with masked token) and exit
    /// without making the request. Pair with `--show-secrets` if you need
    /// the real token in the output.
    #[arg(long)]
    curl: bool,
}

impl ApiCommand {
    pub async fn run(&self) -> Result<()> {
        let config = Config::load()?;
        let api = config.client()?;

        // Expand {owner} and {repo} placeholders
        let endpoint = self.expand_placeholders(&config)?;

        // Build the full URL using the API crate
        let url: url::Url = api.url_for(&endpoint).parse()
            .map_err(|e| eyre::eyre!("Invalid URL: {e}"))?;

        // Determine method
        let has_body_fields = !self.raw_fields.is_empty() || !self.fields.is_empty();
        let method = match &self.method {
            Some(m) => m.to_uppercase(),
            None => {
                if has_body_fields {
                    "POST".to_string()
                } else {
                    "GET".to_string()
                }
            }
        };

        // Build request body
        let body = if has_body_fields {
            Some(self.build_body()?)
        } else {
            None
        };

        if self.curl {
            let show_secrets = gitea_api::verbose::config()
                .map(|c| c.show_secrets)
                .unwrap_or(false);
            let line = build_curl_command(&method, &url, &self.headers, body.as_ref(), &config.token, show_secrets);
            println!("{line}");
            return Ok(());
        }

        if self.paginate {
            self.run_paginated(&api, &url, &method, &body).await
        } else {
            self.run_single(&api, &url, &method, &body).await
        }
    }

    fn expand_placeholders(&self, config: &Config) -> Result<String> {
        let mut endpoint = self.endpoint.clone();
        if endpoint.contains("{owner}") || endpoint.contains("{repo}") {
            let repo_info = repo::resolve_repo(None, &config.url)?;
            endpoint = endpoint.replace("{owner}", &repo_info.owner);
            endpoint = endpoint.replace("{repo}", &repo_info.name);
        }
        Ok(endpoint)
    }

    fn build_body(&self) -> Result<serde_json::Value> {
        let mut map = serde_json::Map::new();

        for field in &self.raw_fields {
            let (key, value) = parse_key_value(field)?;
            map.insert(key, serde_json::Value::String(value));
        }

        for field in &self.fields {
            let (key, value) = parse_key_value(field)?;
            let typed_value = parse_typed_value(&value)?;
            map.insert(key, typed_value);
        }

        Ok(serde_json::Value::Object(map))
    }

    async fn run_single(
        &self,
        api: &gitea_api::Gitea,
        url: &url::Url,
        method: &str,
        body: &Option<serde_json::Value>,
    ) -> Result<()> {
        let resp = self.send_request(api, url, method, body).await?;

        if self.include {
            println!("{} {}", resp.status().as_u16(), resp.status().canonical_reason().unwrap_or(""));
            for (key, value) in resp.headers() {
                println!("{}: {}", key, value.to_str().unwrap_or(""));
            }
            println!();
        }

        let text = resp.text().await?;
        if let Some(cfg) = gitea_api::verbose::config()
            && cfg.show_body()
        {
            gitea_api::verbose::print_response_body(&text);
        }
        if !self.silent {
            self.output(&text)?;
        }

        Ok(())
    }

    async fn run_paginated(
        &self,
        api: &gitea_api::Gitea,
        base_url: &url::Url,
        method: &str,
        body: &Option<serde_json::Value>,
    ) -> Result<()> {
        let mut page = 1u32;
        loop {
            let mut url = base_url.clone();
            url.query_pairs_mut().append_pair("page", &page.to_string());
            if !url.query_pairs().any(|(k, _)| k == "limit") {
                url.query_pairs_mut().append_pair("limit", "50");
            }

            let resp = self.send_request(api, &url, method, body).await?;

            let text = resp.text().await?;
            if let Some(cfg) = gitea_api::verbose::config()
                && cfg.show_body()
            {
                gitea_api::verbose::print_response_body(&text);
            }
            if !self.silent {
                self.output(&text)?;
            }

            // Check if there's more data
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            if let Some(arr) = parsed.as_array() {
                if arr.is_empty() {
                    break;
                }
            } else {
                break; // Non-array response, no pagination
            }

            page += 1;
        }

        Ok(())
    }

    async fn send_request(
        &self,
        api: &gitea_api::Gitea,
        url: &url::Url,
        method: &str,
        body: &Option<serde_json::Value>,
    ) -> Result<gitea_api::Response> {
        // Parse custom headers
        let mut headers = Vec::new();
        for h in &self.headers {
            let (key, value) = h
                .split_once(':')
                .ok_or_else(|| eyre::eyre!("Invalid header format: {h}. Use key:value"))?;
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }

        let method = method.parse::<gitea_api::Method>()
            .map_err(|_| eyre::eyre!("Unsupported HTTP method: {method}"))?;

        let resp = api
            .request(method, url.as_str(), &headers, body.as_ref())
            .await
            .map_err(|e| eyre::eyre!("{e}"))?;

        if !resp.status().is_success() && !self.include {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            for line in auth_hint_for(status.as_u16(), &text) {
                eprintln!("hint: {line}");
            }
            eyre::bail!("{} {}\n{}", status.as_u16(), status.canonical_reason().unwrap_or(""), text);
        }

        Ok(resp)
    }

    fn output(&self, text: &str) -> Result<()> {
        if let Some(ref expr) = self.jq_expr {
            // Simple jq-like field extraction: .field or .[].field
            let parsed: serde_json::Value = serde_json::from_str(text)?;
            let results = crate::json::jq_select(&parsed, expr)?;
            for r in results {
                match r {
                    serde_json::Value::String(s) => println!("{s}"),
                    other => println!("{}", serde_json::to_string_pretty(&other)?),
                }
            }
        } else {
            // Pretty-print JSON, or raw if not JSON
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                Err(_) => print!("{text}"),
            }
        }
        Ok(())
    }
}

/// Render an equivalent `curl` command for a given request. Authorization
/// header is masked unless `show_secrets` is true. The body, when present,
/// is rendered as `--data-raw '<json>'` with single quotes escaped.
fn build_curl_command(
    method: &str,
    url: &url::Url,
    custom_headers: &[String],
    body: Option<&serde_json::Value>,
    token: &str,
    show_secrets: bool,
) -> String {
    let mut parts = vec![format!("curl -X {method}")];
    parts.push(format!("'{}'", url.as_str()));

    let auth_value = if show_secrets {
        format!("token {token}")
    } else {
        gitea_api::verbose::mask_header_value("Authorization", &format!("token {token}"))
    };
    parts.push(format!("-H 'Authorization: {auth_value}'"));
    parts.push("-H 'Accept: application/json'".to_string());

    for h in custom_headers {
        parts.push(format!("-H '{h}'"));
    }

    if let Some(body) = body {
        let json = serde_json::to_string(body).unwrap_or_default();
        let escaped = json.replace('\'', r"'\''");
        parts.push("-H 'Content-Type: application/json'".to_string());
        parts.push(format!("--data-raw '{escaped}'"));
    }

    parts.join(" \\\n  ")
}

/// Build hint lines for auth-related failures (401/403). Returns an empty
/// vec for other status codes or unrecognized bodies.
fn auth_hint_for(status: u16, body: &str) -> Vec<String> {
    match status {
        401 => vec![
            "token rejected by server. Run `gt auth login` to refresh.".to_string(),
        ],
        403 => {
            let message = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .unwrap_or_default();
            if let Some(scopes) = extract_required_scopes(&message) {
                vec![
                    format!("token is missing required scope(s): {scopes}"),
                    "re-run `gt auth login` with a token that includes those scopes,".to_string(),
                    "      or use a different token (admin tokens cover read:admin).".to_string(),
                ]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// Pull the contents of `required=[...]` out of a Gitea 403 message, if present.
fn extract_required_scopes(message: &str) -> Option<String> {
    let start = message.find("required=[")? + "required=[".len();
    let rest = &message[start..];
    let end = rest.find(']')?;
    let scopes = rest[..end].trim();
    if scopes.is_empty() { None } else { Some(scopes.to_string()) }
}

fn parse_key_value(s: &str) -> Result<(String, String)> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| eyre::eyre!("Invalid field format: {s}. Use key=value"))?;
    Ok((key.to_string(), value.to_string()))
}

fn parse_typed_value(s: &str) -> Result<serde_json::Value> {
    // @file reads from file
    if let Some(path) = s.strip_prefix('@') {
        if path == "-" {
            let content = std::io::read_to_string(std::io::stdin())?;
            return Ok(serde_json::Value::String(content));
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| eyre::eyre!("Failed to read {path}: {e}"))?;
        return Ok(serde_json::Value::String(content));
    }

    // Boolean/null literals
    match s {
        "true" => return Ok(serde_json::Value::Bool(true)),
        "false" => return Ok(serde_json::Value::Bool(false)),
        "null" => return Ok(serde_json::Value::Null),
        _ => {}
    }

    // Integer
    if let Ok(n) = s.parse::<i64>() {
        return Ok(serde_json::json!(n));
    }

    // Fall back to string
    Ok(serde_json::Value::String(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_value() {
        let (k, v) = parse_key_value("name=hello").unwrap();
        assert_eq!(k, "name");
        assert_eq!(v, "hello");

        assert!(parse_key_value("noeq").is_err());
    }

    #[test]
    fn test_curl_masks_token_by_default() {
        let url: url::Url = "https://gt.example/api/v1/user".parse().unwrap();
        let line = build_curl_command("GET", &url, &[], None, "aafe123456789a263", false);
        assert!(line.contains("Authorization: token ***a263"), "got: {line}");
        assert!(!line.contains("aafe123456789a263"), "raw token leaked: {line}");
        assert!(line.contains("curl -X GET"));
        assert!(line.contains("https://gt.example/api/v1/user"));
    }

    #[test]
    fn test_curl_show_secrets_emits_full_token() {
        let url: url::Url = "https://gt.example/api/v1/user".parse().unwrap();
        let line = build_curl_command("GET", &url, &[], None, "aafe123456789a263", true);
        assert!(line.contains("Authorization: token aafe123456789a263"), "got: {line}");
    }

    #[test]
    fn test_curl_includes_body_and_content_type() {
        let url: url::Url = "https://gt.example/api/v1/repos/o/r/issues".parse().unwrap();
        let body = serde_json::json!({"title": "hi", "body": "what's up"});
        let line = build_curl_command("POST", &url, &[], Some(&body), "tok12345678abcd", false);
        assert!(line.contains("-X POST"));
        assert!(line.contains("Content-Type: application/json"));
        assert!(line.contains("--data-raw"));
        assert!(line.contains(r#""title":"hi""#));
    }

    #[test]
    fn test_curl_escapes_single_quotes_in_body() {
        let url: url::Url = "https://gt.example/api/v1/x".parse().unwrap();
        let body = serde_json::json!({"msg": "it's fine"});
        let line = build_curl_command("POST", &url, &[], Some(&body), "tok12345678abcd", false);
        assert!(line.contains(r"'\''"), "expected single-quote escape, got: {line}");
    }

    #[test]
    fn test_curl_includes_custom_headers() {
        let url: url::Url = "https://gt.example/api/v1/x".parse().unwrap();
        let headers = vec!["X-Foo:bar".to_string()];
        let line = build_curl_command("GET", &url, &headers, None, "tok12345678abcd", false);
        assert!(line.contains("-H 'X-Foo:bar'"), "got: {line}");
    }

    #[test]
    fn test_auth_hint_for_401() {
        let hints = auth_hint_for(401, r#"{"message":"unauthorized"}"#);
        assert_eq!(hints.len(), 1);
        assert!(hints[0].contains("gt auth login"));
    }

    #[test]
    fn test_auth_hint_for_403_with_required_scope() {
        let body = r#"{"message":"token does not have at least one of required scope(s), required=[read:admin], token scope=write:user","url":"https://gt.example/api/swagger"}"#;
        let hints = auth_hint_for(403, body);
        assert!(!hints.is_empty(), "expected hints for scope-shaped 403");
        assert!(hints[0].contains("read:admin"), "expected hint to name missing scope, got {hints:?}");
    }

    #[test]
    fn test_auth_hint_for_403_without_scope_info() {
        let hints = auth_hint_for(403, r#"{"message":"forbidden"}"#);
        assert!(hints.is_empty(), "no scope info => no hint, got {hints:?}");
    }

    #[test]
    fn test_auth_hint_for_other_status() {
        assert!(auth_hint_for(404, r#"{"message":"not found"}"#).is_empty());
        assert!(auth_hint_for(500, "").is_empty());
    }

    #[test]
    fn test_extract_required_scopes() {
        assert_eq!(
            extract_required_scopes("required=[read:admin], token scope=..."),
            Some("read:admin".to_string()),
        );
        assert_eq!(
            extract_required_scopes("required=[read:admin write:user], token scope=..."),
            Some("read:admin write:user".to_string()),
        );
        assert_eq!(extract_required_scopes("forbidden"), None);
        assert_eq!(extract_required_scopes("required=[]"), None);
    }

    #[test]
    fn test_parse_typed_value() {
        assert_eq!(parse_typed_value("true").unwrap(), serde_json::json!(true));
        assert_eq!(parse_typed_value("false").unwrap(), serde_json::json!(false));
        assert_eq!(parse_typed_value("null").unwrap(), serde_json::json!(null));
        assert_eq!(parse_typed_value("42").unwrap(), serde_json::json!(42));
        assert_eq!(
            parse_typed_value("hello").unwrap(),
            serde_json::json!("hello")
        );
    }
}
