#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use assert_cmd::Command;

/// A full reply from [`FakeGitea::start_raw`]: status, extra headers, and body bytes.
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    /// A JSON reply, as [`FakeGitea::start_with`] sends.
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.into().into_bytes(),
        }
    }

    /// A 302 to `location`.
    pub fn redirect(location: impl Into<String>) -> Self {
        Reply {
            status: 302,
            headers: vec![("Location".into(), location.into())],
            body: Vec::new(),
        }
    }

    /// A 200 with a binary body.
    pub fn bytes(content_type: &str, body: Vec<u8>) -> Self {
        Reply {
            status: 200,
            headers: vec![("Content-Type".into(), content_type.into())],
            body,
        }
    }
}

/// Fake Gitea server: answers each request with `route(method, path_and_query)`
/// as `(status, body)` and records `"METHOD target"` for every request it saw.
pub struct FakeGitea {
    pub url: String,
    seen: Arc<Mutex<Vec<String>>>,
    auth: Arc<Mutex<Vec<Option<String>>>>,
    bodies: Arc<Mutex<Vec<String>>>,
    content_types: Arc<Mutex<Vec<Option<String>>>>,
}

impl FakeGitea {
    /// Answer every request with 200 and `route(path_and_query)` as a JSON body.
    pub fn start(route: impl Fn(&str) -> String + Send + 'static) -> Self {
        Self::start_with(move |_, target| (200, route(target)))
    }

    pub fn start_with(route: impl Fn(&str, &str) -> (u16, String) + Send + 'static) -> Self {
        Self::start_raw(move |method, target| {
            let (status, body) = route(method, target);
            Reply::json(status, body)
        })
    }

    /// Answer each request with the full [`Reply`] `route(method, path_and_query)` returns.
    pub fn start_raw(route: impl Fn(&str, &str) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_thread = Arc::clone(&seen);
        let auth = Arc::new(Mutex::new(Vec::new()));
        let auth_thread = Arc::clone(&auth);
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let bodies_thread = Arc::clone(&bodies);
        let content_types = Arc::new(Mutex::new(Vec::new()));
        let content_types_thread = Arc::clone(&content_types);
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
                let mut content_type = None;
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
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("content-type")
                    {
                        content_type = Some(value.trim().to_string());
                    }
                }
                let mut req_body = vec![0; content_length];
                let _ = reader.read_exact(&mut req_body);

                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let reply = route(&method, &target);
                seen_thread
                    .lock()
                    .unwrap()
                    .push(format!("{method} {target}"));
                auth_thread.lock().unwrap().push(authorization);
                content_types_thread.lock().unwrap().push(content_type);
                bodies_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&req_body).into_owned());
                let mut head = if reply.status == 204 {
                    "HTTP/1.1 204 No Content\r\n".to_string()
                } else {
                    format!(
                        "HTTP/1.1 {} X\r\nContent-Length: {}\r\n",
                        reply.status,
                        reply.body.len()
                    )
                };
                for (name, value) in &reply.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("Connection: close\r\n\r\n");
                let _ = stream.write_all(head.as_bytes());
                if reply.status != 204 {
                    let _ = stream.write_all(&reply.body);
                }
            }
        });
        FakeGitea {
            url,
            seen,
            auth,
            bodies,
            content_types,
        }
    }

    pub fn gtx(&self) -> Command {
        let mut cmd = Command::cargo_bin("gtx").unwrap();
        cmd.env("GITEA_URL", &self.url)
            .env("GITEA_TOKEN", "t")
            .env_remove("GITEA_SERVER")
            .env_remove("GTX_DEBUG")
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

    /// Request body of each request (lossy UTF-8), parallel to [`Self::seen`].
    pub fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    /// `Content-Type` header of each request, parallel to [`Self::seen`].
    pub fn content_types(&self) -> Vec<Option<String>> {
        self.content_types.lock().unwrap().clone()
    }
}
