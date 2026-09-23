#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use assert_cmd::Command;

/// Fake Gitea server: answers each request with `route(method, path_and_query)`
/// as `(status, body)` and records `"METHOD target"` for every request it saw.
pub struct FakeGitea {
    pub url: String,
    seen: Arc<Mutex<Vec<String>>>,
    auth: Arc<Mutex<Vec<Option<String>>>>,
}

impl FakeGitea {
    /// Answer every request with 200 and `route(path_and_query)` as a JSON body.
    pub fn start(route: impl Fn(&str) -> String + Send + 'static) -> Self {
        Self::start_with(move |_, target| (200, route(target)))
    }

    pub fn start_with(route: impl Fn(&str, &str) -> (u16, String) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_thread = Arc::clone(&seen);
        let auth = Arc::new(Mutex::new(Vec::new()));
        let auth_thread = Arc::clone(&auth);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let mut content_length = 0usize;
                let mut authorization = None;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("authorization")
                    {
                        authorization = Some(value.trim().to_string());
                    }
                }
                let mut req_body = vec![0; content_length];
                let _ = reader.read_exact(&mut req_body);

                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let (status, body) = route(&method, &target);
                seen_thread
                    .lock()
                    .unwrap()
                    .push(format!("{method} {target}"));
                auth_thread.lock().unwrap().push(authorization);
                let resp = if status == 204 {
                    "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_string()
                } else {
                    format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        FakeGitea { url, seen, auth }
    }

    pub fn gtx(&self) -> Command {
        let mut cmd = Command::cargo_bin("gtx").unwrap();
        cmd.env("GITEA_URL", &self.url)
            .env("GITEA_TOKEN", "t")
            .env_remove("GITEA_SERVER")
            .env("GTX_CONFIG", "/nonexistent/gtx-test.toml");
        cmd
    }

    /// Requests seen, as `"METHOD path_and_query"`.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    /// `Authorization` header of each request, parallel to [`Self::seen`].
    pub fn auth(&self) -> Vec<Option<String>> {
        self.auth.lock().unwrap().clone()
    }
}
