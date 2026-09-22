use std::path::Path;

use eyre::Result;

/// Read a body from a file path, optionally resolving local file references.
pub fn read_body_file(path: &str) -> Result<String> {
    let p = Path::new(path);
    if !p.exists() {
        eyre::bail!("File not found: {path}");
    }
    Ok(std::fs::read_to_string(p)?)
}

/// Markdown link/image pattern: `![alt](path)` or `[text](path)`
/// Returns Vec of (full_match, path) for local file references.
pub fn find_local_refs(body: &str, base_dir: &Path) -> Vec<(String, String)> {
    let mut refs = Vec::new();
    // Match ![...](...) and [...](...) patterns
    let re_pattern = regex_lite::Regex::new(r"!?\[([^\]]*)\]\(([^)]+)\)").unwrap();

    for cap in re_pattern.captures_iter(body) {
        let full = cap.get(0).unwrap().as_str().to_string();
        let path_str = cap.get(2).unwrap().as_str();

        // Only process relative paths (not http/https URLs)
        if path_str.starts_with("http://") || path_str.starts_with("https://") {
            continue;
        }

        // Check if the file exists relative to base_dir
        let file_path = base_dir.join(path_str);
        if file_path.exists() && file_path.is_file() {
            refs.push((full, file_path.to_string_lossy().to_string()));
        }
    }

    refs
}

/// Upload local file attachments and rewrite the body with attachment URLs.
pub async fn upload_and_rewrite(
    api: &gitea_api::Gitea,
    config: &crate::config::Config,
    owner: &str,
    repo: &str,
    issue_index: i64,
    body: &str,
    base_dir: &Path,
) -> Result<String> {
    let refs = find_local_refs(body, base_dir);
    if refs.is_empty() {
        return Ok(body.to_string());
    }

    let mut result = body.to_string();

    for (full_match, file_path) in &refs {
        let path = Path::new(file_path);
        let filename = path
            .file_name()
            .ok_or_else(|| eyre::eyre!("Invalid filename: {file_path}"))?
            .to_string_lossy()
            .to_string();

        let file_bytes = std::fs::read(path)?;

        // Upload as issue attachment (multipart form)
        let url = api.url_for(&format!(
            "repos/{owner}/{repo}/issues/{issue_index}/assets?name={filename}"
        ));

        let part = reqwest::multipart::Part::bytes(file_bytes).file_name(filename.clone());
        let form = reqwest::multipart::Form::new().part("attachment", part);

        let resp = reqwest::Client::new()
            .post(&url)
            .header("Authorization", format!("token {}", config.token))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            eprintln!("Warning: failed to upload {filename}: HTTP {}", status.as_u16());
            continue;
        }

        let attachment: serde_json::Value = resp.json().await?;
        if let Some(download_url) = attachment["browser_download_url"].as_str() {
            // Determine if this was an image or a link
            let is_image = full_match.starts_with('!');
            let alt = if is_image {
                &full_match[2..full_match.find(']').unwrap_or(2)]
            } else {
                &full_match[1..full_match.find(']').unwrap_or(1)]
            };

            let replacement = if is_image {
                format!("![{alt}]({download_url})")
            } else {
                format!("[{alt}]({download_url})")
            };

            result = result.replace(full_match, &replacement);
            eprintln!("Uploaded {filename} → {download_url}");
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_local_refs_images() {
        let body = "Here is ![screenshot](./img/shot.png) and text";
        let refs = find_local_refs(body, Path::new("/nonexistent"));
        // File doesn't exist, so no refs returned
        assert!(refs.is_empty());
    }

    #[test]
    fn test_find_local_refs_skips_urls() {
        let body = "![img](https://example.com/img.png) and [link](http://example.com)";
        let refs = find_local_refs(body, Path::new("."));
        assert!(refs.is_empty());
    }

    #[test]
    fn test_find_local_refs_with_existing_file() {
        // Cargo.toml exists in the gt crate root
        let body = "See [config](Cargo.toml) for details";
        let refs = find_local_refs(body, Path::new(env!("CARGO_MANIFEST_DIR")));
        assert_eq!(refs.len(), 1);
        assert!(refs[0].1.contains("Cargo.toml"));
    }
}
