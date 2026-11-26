//! The single place where burrow talks to LeafWiki.

use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use url::Url;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// A failed request. HTTP failures are values, never panics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiErr {
    /// HTTP status, or 0 when no response was received (network error, timeout, not signed in).
    pub status: u16,
    /// One line: LeafWiki's own error message when it sent one.
    pub message: String,
    /// LeafWiki's error code (`page_version_conflict`, `page_has_children`, …), when it sent one.
    pub code: Option<String>,
    /// Raw response body, or the reason no response was received.
    pub body: String,
    /// `METHOD path`, for diagnostics.
    pub request: Option<String>,
}

pub fn err(status: u16, message: impl Into<String>) -> WikiErr {
    WikiErr {
        status,
        message: message.into(),
        code: None,
        body: String::new(),
        request: None,
    }
}

pub type Res<T> = Result<T, WikiErr>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Empty,
    Json(String),
    /// Multipart form with one `file` field.
    File {
        name: String,
        bytes: Vec<u8>,
    },
}

/// A request below `/api`.
#[derive(Debug, Clone)]
pub struct Spec {
    pub method: Method,
    /// Path below `/api`, starting with `/`.
    pub path: String,
    /// Empty values are left out, as LeafWiki's web app does.
    pub query: Vec<(&'static str, String)>,
    pub body: Body,
}

impl Spec {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Spec {
            method,
            path: path.into(),
            query: Vec::new(),
            body: Body::Empty,
        }
    }

    pub fn query(mut self, key: &'static str, value: Option<impl ToString>) -> Self {
        if let Some(v) = value {
            let v = v.to_string();
            if !v.is_empty() {
                self.query.push((key, v));
            }
        }
        self
    }

    pub fn json(mut self, value: Value) -> Self {
        self.body = Body::Json(value.to_string());
        self
    }
}

/// What goes over the wire; the transport adds nothing but the body encoding.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub location: Option<String>,
    pub set_cookies: Vec<String>,
    pub csrf: Option<String>,
    /// The body, or why reading it failed.
    pub body: Result<String, String>,
}

impl Default for Response {
    fn default() -> Self {
        Response {
            status: 200,
            location: None,
            set_cookies: Vec::new(),
            csrf: None,
            body: Ok(String::new()),
        }
    }
}

/// Sends one request without following redirects. `Err` when no response arrived.
pub trait Transport: Send + Sync {
    fn send(&self, request: Request) -> BoxFuture<'_, Result<Response, String>>;
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// The real network, through reqwest.
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("the HTTP client builds with static settings");
        Http { client }
    }
}

impl Transport for Http {
    fn send(&self, request: Request) -> BoxFuture<'_, Result<Response, String>> {
        Box::pin(async move {
            let method = match request.method {
                Method::Get => reqwest::Method::GET,
                Method::Post => reqwest::Method::POST,
                Method::Put => reqwest::Method::PUT,
                Method::Delete => reqwest::Method::DELETE,
            };
            let mut builder = self.client.request(method, &request.url);
            for (name, value) in request.headers {
                builder = builder.header(name, value);
            }
            builder = match request.body {
                Body::Empty => builder,
                Body::Json(text) => builder.body(text),
                Body::File { name, bytes } => {
                    let part = reqwest::multipart::Part::bytes(bytes)
                        .file_name(name)
                        .mime_str("application/octet-stream")
                        .expect("a static MIME type parses");
                    builder.multipart(reqwest::multipart::Form::new().part("file", part))
                }
            };
            let response = builder.send().await.map_err(reason_of)?;
            let header = |name: &str| {
                response
                    .headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            let status = response.status().as_u16();
            let location = header("location");
            let csrf = header("x-csrf-token");
            let set_cookies = response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok().map(str::to_string))
                .collect();
            let body = response.text().await.map_err(reason_of);
            Ok(Response {
                status,
                location,
                set_cookies,
                csrf,
                body,
            })
        })
    }
}

/// The error and its innermost cause (refused, DNS, TLS), without the URL.
fn reason_of(error: reqwest::Error) -> String {
    let error = error.without_url();
    let message = error.to_string();
    let mut cause: Option<&dyn std::error::Error> = std::error::Error::source(&error);
    let mut why = String::new();
    while let Some(c) = cause {
        why = c.to_string();
        cause = c.source();
    }
    if !why.is_empty() && !message.contains(&why) {
        format!("{message} ({why})")
    } else {
        message
    }
}

const CSRF_COOKIE: &str = "leafwiki_csrf";
/// Refresh this long before the access token runs out (LeafWiki's default lifetime is 15 minutes).
const REFRESH_MARGIN_MS: i64 = 60_000;

#[derive(Default)]
struct State {
    /// In the order LeafWiki set them; a renewed cookie keeps its place.
    cookies: Vec<(String, String)>,
    csrf: String,
    /// When the access token runs out (ms); 0 when not signed in.
    expires_at: i64,
    auth_disabled: bool,
    /// Counts successful sign-ins and refreshes, to tell a stale 401 from a lost session.
    sign_ins: u64,
    /// A sign-in failure that repeats until the configuration changes (wrong password, TOTP
    /// account): not retried, since every failed attempt counts toward LeafWiki's lockout.
    rejected: Option<WikiErr>,
}

pub struct SessionOptions {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub transport: Box<dyn Transport>,
    pub log: Logger,
    pub now: Clock,
}

pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    })
}

/// Signs in the way the web app does: session cookies plus a CSRF token echoed in
/// `X-CSRF-Token` on every change. Refreshes the access token before it runs out, and signs
/// in again once when LeafWiki answers 401 (nothing was done then, so any request can be
/// repeated). Never follows redirects, so cookies and the password only go to `base_url`.
pub struct Session {
    pub base_url: String,
    username: String,
    password: String,
    transport: Box<dyn Transport>,
    log: Logger,
    now: Clock,
    state: Mutex<State>,
    /// One sign-in at a time: parallel tool calls wait for the same one.
    signing_in: tokio::sync::Mutex<()>,
}

impl Session {
    pub fn new(options: SessionOptions) -> Self {
        Session {
            base_url: options.base_url,
            username: options.username,
            password: options.password,
            transport: options.transport,
            log: options.log,
            now: options.now,
            state: Mutex::new(State::default()),
            signing_in: tokio::sync::Mutex::new(()),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Absolute URL of a path LeafWiki returns relative to its root (`/assets/…`).
    pub fn absolute(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{path}", self.base_url)
        } else {
            format!("{}/{path}", self.base_url)
        }
    }

    pub async fn request(&self, spec: Spec) -> Res<Value> {
        let target = format!("{}/api{}", self.base_url, spec.path);
        if Url::parse(&target)
            .map(|u| u.as_str() != target)
            .unwrap_or(true)
        {
            let mut e = err(
                0,
                format!(
                    "Not sent: \"{}\" would not reach the intended LeafWiki route",
                    spec.path
                ),
            );
            e.request = Some(format!("{} /api{}", spec.method.as_str(), spec.path));
            return Err(e);
        }
        self.ensure_signed_in().await?;
        let seen = self.state().sign_ins;
        let first = self.send(&spec).await;
        match &first {
            Err(e) if e.status == 401 && !self.state().auth_disabled => {}
            _ => return first,
        }
        // A sign-in finished while this call was under way: its cookies were old, the session is fine.
        if seen == self.state().sign_ins {
            self.state().expires_at = 0;
            self.sign_in(true, seen).await?;
        }
        self.send(&spec).await
    }

    async fn ensure_signed_in(&self) -> Res<()> {
        let (seen, refresh) = {
            let s = self.state();
            if s.auth_disabled
                || (s.expires_at != 0 && (self.now)() < s.expires_at - REFRESH_MARGIN_MS)
            {
                return Ok(());
            }
            (s.sign_ins, s.expires_at != 0)
        };
        self.sign_in(refresh, seen).await
    }

    /// Signs in unless another call did since `seen`; a rejected configuration is not retried.
    async fn sign_in(&self, refresh_first: bool, seen: u64) -> Res<()> {
        if let Some(rejected) = self.state().rejected.clone() {
            return Err(rejected);
        }
        let _one_at_a_time = self.signing_in.lock().await;
        {
            let s = self.state();
            if let Some(rejected) = &s.rejected {
                return Err(rejected.clone());
            }
            if s.sign_ins != seen || s.auth_disabled {
                return Ok(());
            }
        }
        self.do_sign_in(refresh_first).await
    }

    async fn do_sign_in(&self, refresh_first: bool) -> Res<()> {
        if refresh_first && !self.state().cookies.is_empty() {
            match self
                .send(&Spec::new(Method::Post, "/auth/refresh-token"))
                .await
            {
                Ok(data) => {
                    self.set_expiry(&data);
                    self.state().sign_ins += 1;
                    return Ok(());
                }
                Err(e) => (self.log)(&format!("refresh failed ({}), signing in again", e.status)),
            }
        }

        // `/config` issues the CSRF cookie and says whether the wiki has sign-in at all.
        let config = self.send(&Spec::new(Method::Get, "/config")).await?;
        if config.get("authDisabled").and_then(Value::as_bool) == Some(true) {
            self.state().auth_disabled = true;
            return Ok(());
        }
        if self.username.is_empty() {
            return Err(self.reject(err(
                0,
                "This wiki needs sign-in: set LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD, then restart the MCP client",
            )));
        }

        let login = Spec::new(Method::Post, "/auth/login")
            .json(serde_json::json!({ "identifier": self.username, "password": self.password }));
        let data = match self.send(&login).await {
            Ok(data) => data,
            Err(e) if e.code.as_deref() == Some("auth_invalid_credentials") => {
                let message = format!(
                    "Sign-in as \"{}\" failed: {}. Check LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD, then restart the MCP client",
                    self.username, e.message
                );
                return Err(self.reject(WikiErr { message, ..e }));
            }
            Err(e) => return Err(e),
        };
        if data.get("requiresTotp").and_then(Value::as_bool) == Some(true) {
            return Err(self.reject(err(
                0,
                format!(
                    "The account \"{}\" uses two-factor codes (TOTP), which burrow cannot enter. Use an account without TOTP",
                    self.username
                ),
            )));
        }
        self.set_expiry(&data);
        self.state().sign_ins += 1;
        (self.log)(&format!("signed in as {}", self.username));
        Ok(())
    }

    fn reject(&self, e: WikiErr) -> WikiErr {
        self.state().rejected = Some(e.clone());
        e
    }

    fn set_expiry(&self, data: &Value) {
        // Unknown lifetime: assume LeafWiki's default 15 minutes.
        let seconds = data
            .get("accessTokenExpiresAt")
            .and_then(Value::as_f64)
            .filter(|s| *s != 0.0);
        let at = match seconds {
            Some(s) => (s * 1000.0) as i64,
            None => (self.now)() + 15 * 60_000,
        };
        self.state().expires_at = at;
    }

    fn url(&self, spec: &Spec) -> String {
        let mut url = format!("{}/api{}", self.base_url, spec.path);
        if !spec.query.is_empty() {
            let mut qs = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in &spec.query {
                qs.append_pair(k, v);
            }
            url.push('?');
            url.push_str(&qs.finish());
        }
        url
    }

    async fn send(&self, spec: &Spec) -> Res<Value> {
        let request = format!("{} /api{}", spec.method.as_str(), spec.path);
        let mut headers: Vec<(&'static str, String)> = vec![("accept", "application/json".into())];
        {
            let s = self.state();
            if !s.cookies.is_empty() {
                let cookie = s
                    .cookies
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                headers.push(("cookie", cookie));
            }
            if spec.method != Method::Get && !s.csrf.is_empty() {
                headers.push(("x-csrf-token", s.csrf.clone()));
            }
        }
        if matches!(spec.body, Body::Json(_)) {
            headers.push(("content-type", "application/json".into()));
        }

        let sent = self
            .transport
            .send(Request {
                method: spec.method,
                url: self.url(spec),
                headers,
                body: spec.body.clone(),
            })
            .await;
        let (response, text) = match sent {
            Ok(response) => {
                self.remember(&response);
                match response.body.clone() {
                    Ok(text) => (response, text),
                    Err(reason) => return Err(self.unanswered(&request, reason)),
                }
            }
            Err(reason) => return Err(self.unanswered(&request, reason)),
        };

        if (200..300).contains(&response.status) {
            if text.is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text).map_err(|_| WikiErr {
                status: response.status,
                message: "LeafWiki answered with something that is not JSON".into(),
                code: None,
                body: text.chars().take(500).collect(),
                request: Some(request.clone()),
            });
        }
        (self.log)(&format!("{request} -> {}", response.status));
        if (300..400).contains(&response.status) {
            if let Some(location) = response.location.as_deref().filter(|l| !l.is_empty()) {
                let to = location.split(['?', '#']).next().unwrap_or("");
                let mut e = err(
                    response.status,
                    format!(
                        "LeafWiki redirected to {to}; burrow does not follow redirects: set LEAFWIKI_URL to the address the wiki answers on"
                    ),
                );
                e.request = Some(request);
                return Err(e);
            }
        }
        Err(failure_of(response.status, &text, request))
    }

    fn unanswered(&self, request: &str, reason: String) -> WikiErr {
        (self.log)(&format!("{request} failed: {reason}"));
        WikiErr {
            status: 0,
            message: format!("LeafWiki did not answer: {reason}"),
            code: None,
            body: reason,
            request: Some(request.to_string()),
        }
    }

    fn remember(&self, response: &Response) {
        let mut s = self.state();
        for line in &response.set_cookies {
            let mut parts = line.split(';');
            let pair = parts.next().unwrap_or("");
            let Some(at) = pair.find('=').filter(|at| *at > 0) else {
                continue;
            };
            let name = pair[..at].trim().to_string();
            let value = pair[at + 1..].trim().to_string();
            let expired = value.is_empty()
                || parts.any(|a| {
                    let a = a.trim_start().to_ascii_lowercase();
                    a.starts_with("max-age=0") || a.starts_with("max-age=-")
                });
            if expired {
                s.cookies.retain(|(k, _)| *k != name);
            } else if let Some(slot) = s.cookies.iter_mut().find(|(k, _)| *k == name) {
                slot.1 = value;
            } else {
                s.cookies.push((name, value));
            }
        }
        let cookie = |name: &str| {
            s.cookies
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        let csrf = response
            .csrf
            .clone()
            .or_else(|| cookie(CSRF_COOKIE))
            .or_else(|| cookie(&format!("__Host-{CSRF_COOKIE}")));
        if let Some(csrf) = csrf {
            s.csrf = csrf;
        }
    }
}

/// LeafWiki answers `{"error":"text"}`, `{"error":{"code","message"}}` or plain text.
fn failure_of(status: u16, text: &str, request: String) -> WikiErr {
    let first_line = text.trim().split('\n').next().unwrap_or("");
    let mut message = if first_line.is_empty() {
        format!("HTTP {status}")
    } else {
        first_line.to_string()
    };
    let mut code = None;
    if let Ok(Value::Object(body)) = serde_json::from_str::<Value>(text) {
        match body.get("error") {
            Some(Value::String(e)) => message = e.clone(),
            Some(Value::Object(e))
                if e.get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|m| !m.is_empty()) =>
            {
                message = e["message"].as_str().unwrap_or_default().to_string();
                code = e
                    .get("code")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty())
                    .map(str::to_string);
            }
            _ => {
                if let Some(m) = body
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                {
                    message = m.to_string();
                }
            }
        }
    }
    WikiErr {
        status,
        message: message.chars().take(300).collect(),
        code,
        body: text.chars().take(2000).collect(),
        request: Some(request),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicI64, Ordering};

    use tokio::sync::Semaphore;

    use super::*;

    #[derive(Debug, Clone)]
    struct Seen {
        method: Method,
        path: String,
        cookie: String,
        csrf: String,
    }

    #[derive(Default)]
    struct FakeState {
        issued: u32,
        valid: HashMap<String, i64>,
        seen: Vec<Seen>,
        held: HashMap<String, Arc<Semaphore>>,
    }

    /// A tiny LeafWiki sign-in: `/config` issues the CSRF cookie, login and refresh issue an
    /// access cookie valid for `ttl` seconds of the fake clock, and anything else needs a valid
    /// access cookie. Cookies are named as on https.
    struct FakeWiki {
        ttl: i64,
        totp: bool,
        refresh_works: bool,
        clock: Arc<AtomicI64>,
        state: Mutex<FakeState>,
    }

    fn reply(status: u16, body: Value, cookies: &[String], csrf: Option<&str>) -> Response {
        Response {
            status,
            location: None,
            set_cookies: cookies.to_vec(),
            csrf: csrf.map(str::to_string),
            body: Ok(body.to_string()),
        }
    }

    impl FakeWiki {
        fn new() -> Arc<Self> {
            Self::with(900, false, true)
        }

        fn with(ttl: i64, totp: bool, refresh_works: bool) -> Arc<Self> {
            Arc::new(FakeWiki {
                ttl,
                totp,
                refresh_works,
                clock: Arc::new(AtomicI64::new(1_000_000_000_000)),
                state: Mutex::new(FakeState::default()),
            })
        }

        fn now(&self) -> i64 {
            self.clock.load(Ordering::SeqCst)
        }

        fn advance(&self, seconds: i64) {
            self.clock.fetch_add(seconds * 1000, Ordering::SeqCst);
        }

        fn expire_all(&self) {
            self.state.lock().unwrap().valid.clear();
        }

        fn paths(&self) -> Vec<String> {
            self.state
                .lock()
                .unwrap()
                .seen
                .iter()
                .map(|s| s.path.trim_start_matches("/api").to_string())
                .collect()
        }

        fn seen(&self) -> Vec<Seen> {
            self.state.lock().unwrap().seen.clone()
        }

        /// Keeps calls to `path` waiting until the returned gate closes; later calls pass.
        fn hold(&self, path: &str) -> Arc<Semaphore> {
            let gate = Arc::new(Semaphore::new(0));
            self.state
                .lock()
                .unwrap()
                .held
                .insert(format!("/api{path}"), gate.clone());
            gate
        }

        fn access(&self) -> (String, i64) {
            let mut s = self.state.lock().unwrap();
            s.issued += 1;
            let token = format!("at{}", s.issued);
            let expires = self.now() + self.ttl * 1000;
            s.valid.insert(token.clone(), expires);
            (
                format!("leafwiki_at={token}; Path=/; HttpOnly"),
                expires / 1000,
            )
        }
    }

    struct FakeTransport(Arc<FakeWiki>);

    impl Transport for FakeTransport {
        fn send(&self, request: Request) -> BoxFuture<'_, Result<Response, String>> {
            Box::pin(async move {
                let wiki = &self.0;
                let url = Url::parse(&request.url).unwrap();
                let header = |n: &str| {
                    request
                        .headers
                        .iter()
                        .find(|(k, _)| *k == n)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                };
                let cookie = header("cookie");
                let held = {
                    let mut s = wiki.state.lock().unwrap();
                    s.seen.push(Seen {
                        method: request.method,
                        path: url.path().to_string(),
                        cookie: cookie.clone(),
                        csrf: header("x-csrf-token"),
                    });
                    s.held.get(url.path()).cloned()
                };
                let token = cookie
                    .split("; ")
                    .find_map(|c| c.strip_prefix("leafwiki_at="))
                    .map(str::to_string);
                Ok(match url.path() {
                    "/api/config" => reply(
                        200,
                        serde_json::json!({ "authDisabled": false }),
                        &["__Host-leafwiki_csrf=c1; Path=/; Secure".into()],
                        Some("c1"),
                    ),
                    "/api/auth/login" => {
                        let Body::Json(body) = &request.body else {
                            panic!("login without JSON")
                        };
                        let body: Value = serde_json::from_str(body).unwrap();
                        if body["password"] != "pw" {
                            reply(
                                401,
                                serde_json::json!({ "error": { "code": "auth_invalid_credentials", "message": "Invalid credentials" } }),
                                &[],
                                None,
                            )
                        } else if wiki.totp {
                            reply(
                                200,
                                serde_json::json!({ "requiresTotp": true, "loginChallengeToken": "x" }),
                                &[],
                                None,
                            )
                        } else {
                            let (cookie, expires) = wiki.access();
                            reply(
                                200,
                                serde_json::json!({ "accessTokenExpiresAt": expires }),
                                &[cookie, "leafwiki_rt=rt; Path=/".into()],
                                Some("c1"),
                            )
                        }
                    }
                    "/api/auth/refresh-token" => {
                        if !wiki.refresh_works {
                            reply(401, serde_json::json!({ "error": "expired" }), &[], None)
                        } else {
                            let (cookie, expires) = wiki.access();
                            reply(
                                200,
                                serde_json::json!({ "accessTokenExpiresAt": expires }),
                                &[cookie],
                                Some("c1"),
                            )
                        }
                    }
                    _ => {
                        if let Some(gate) = held {
                            // A closed gate refuses at once: the call goes on.
                            let _ = gate.acquire().await;
                        }
                        let valid = token
                            .and_then(|t| wiki.state.lock().unwrap().valid.get(&t).copied())
                            .unwrap_or(0);
                        if valid < wiki.now() {
                            reply(
                                401,
                                serde_json::json!({ "error": "Missing or invalid access token" }),
                                &[],
                                None,
                            )
                        } else {
                            reply(200, serde_json::json!({ "ok": true }), &[], None)
                        }
                    }
                })
            })
        }
    }

    fn quiet() -> Logger {
        Arc::new(|_| {})
    }

    fn session(wiki: &Arc<FakeWiki>, user: &str, password: &str) -> Session {
        let clock = wiki.clock.clone();
        Session::new(SessionOptions {
            base_url: "https://wiki.test".into(),
            username: user.into(),
            password: password.into(),
            transport: Box::new(FakeTransport(wiki.clone())),
            log: quiet(),
            now: Arc::new(move || clock.load(Ordering::SeqCst)),
        })
    }

    fn get(path: &str) -> Spec {
        Spec::new(Method::Get, path)
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn signs_in_once_sends_cookies_and_the_csrf_token_only_on_changes() {
        let wiki = FakeWiki::new();
        let s = session(&wiki, "ed", "pw");
        let (a, b) = tokio::join!(s.request(get("/tree")), s.request(get("/tree")));
        assert!(a.is_ok() && b.is_ok());
        s.request(Spec::new(Method::Put, "/pages/1").json(serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(
            wiki.paths(),
            paths(&["/config", "/auth/login", "/tree", "/tree", "/pages/1"])
        );
        let seen = wiki.seen();
        assert!(seen[2].cookie.contains("leafwiki_at=at1"));
        assert_eq!(seen[2].csrf, "");
        assert_eq!(seen[4].method, Method::Put);
        assert_eq!(seen[4].csrf, "c1");
    }

    #[tokio::test]
    async fn refreshes_the_access_token_before_it_runs_out() {
        let wiki = FakeWiki::new();
        let s = session(&wiki, "ed", "pw");
        s.request(get("/tree")).await.unwrap();
        wiki.advance(870);
        assert!(s.request(get("/tree")).await.is_ok());
        assert_eq!(
            wiki.paths(),
            paths(&[
                "/config",
                "/auth/login",
                "/tree",
                "/auth/refresh-token",
                "/tree"
            ])
        );
    }

    #[tokio::test]
    async fn signs_in_again_and_repeats_the_call_once_when_leafwiki_answers_401() {
        let wiki = FakeWiki::with(900, false, false);
        let s = session(&wiki, "ed", "pw");
        s.request(get("/tree")).await.unwrap();
        wiki.expire_all();
        assert!(
            s.request(Spec::new(Method::Post, "/pages").json(serde_json::json!({})))
                .await
                .is_ok()
        );
        assert_eq!(
            wiki.paths(),
            paths(&[
                "/config",
                "/auth/login",
                "/tree",
                "/pages",
                "/auth/refresh-token",
                "/config",
                "/auth/login",
                "/pages"
            ])
        );
    }

    #[tokio::test]
    async fn explains_a_totp_account_and_missing_credentials_instead_of_failing_on_every_call() {
        let totp = session(&FakeWiki::with(900, true, true), "ed", "pw")
            .request(get("/tree"))
            .await
            .unwrap_err();
        assert!(totp.message.contains("two-factor codes"));
        let none = session(&FakeWiki::new(), "", "")
            .request(get("/tree"))
            .await
            .unwrap_err();
        assert!(
            none.message
                .contains("set LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD")
        );
    }

    struct Fails(Result<Response, String>);

    impl Transport for Fails {
        fn send(&self, _request: Request) -> BoxFuture<'_, Result<Response, String>> {
            let answer = self.0.clone();
            Box::pin(async move { answer })
        }
    }

    fn session_over(transport: impl Transport + 'static, user: &str) -> Session {
        Session::new(SessionOptions {
            base_url: "https://wiki.test".into(),
            username: user.into(),
            password: if user.is_empty() {
                String::new()
            } else {
                "pw".into()
            },
            transport: Box::new(transport),
            log: quiet(),
            now: system_clock(),
        })
    }

    #[tokio::test]
    async fn a_network_failure_is_a_result_naming_its_cause() {
        let s = session_over(
            Fails(Err("error sending request (connection refused)".into())),
            "ed",
        );
        let e = s.request(get("/tree")).await.unwrap_err();
        assert_eq!(
            (e.status, e.message.as_str()),
            (
                0,
                "LeafWiki did not answer: error sending request (connection refused)"
            )
        );
    }

    #[tokio::test]
    async fn a_connection_lost_while_reading_the_answer_is_a_result() {
        let broken = Response {
            status: 200,
            body: Err("socket hang up".into()),
            ..Response::default()
        };
        let e = session_over(Fails(Ok(broken)), "")
            .request(get("/tree"))
            .await
            .unwrap_err();
        assert_eq!(
            (e.status, e.message.as_str()),
            (0, "LeafWiki did not answer: socket hang up")
        );
    }

    #[tokio::test]
    async fn stops_signing_in_once_leafwiki_rejects_the_password_so_the_account_is_not_locked() {
        let wiki = FakeWiki::new();
        let s = session(&wiki, "ed", "wrong");
        for _ in 0..6 {
            let e = s.request(get("/tree")).await.unwrap_err();
            assert!(
                e.message
                    .starts_with("Sign-in as \"ed\" failed: Invalid credentials."),
                "{}",
                e.message
            );
            assert!(e.message.contains("restart"));
        }
        assert_eq!(
            wiki.paths().iter().filter(|p| *p == "/auth/login").count(),
            1
        );
    }

    #[tokio::test]
    async fn a_401_for_a_call_sent_before_a_sign_in_finished_repeats_it_without_signing_in_again() {
        let wiki = FakeWiki::new();
        let s = Arc::new(session(&wiki, "ed", "pw"));
        s.request(get("/tree")).await.unwrap();
        wiki.expire_all();
        let release = wiki.hold("/slow");
        let slow = tokio::spawn({
            let s = s.clone();
            async move { s.request(get("/slow")).await }
        });
        // Let the slow call reach the wiki before the next one starts.
        while wiki.paths().len() < 4 {
            tokio::task::yield_now().await;
        }
        assert!(s.request(get("/tree")).await.is_ok());
        release.close();
        assert!(slow.await.unwrap().is_ok());
        assert_eq!(
            wiki.paths(),
            paths(&[
                "/config",
                "/auth/login",
                "/tree",
                "/slow",
                "/tree",
                "/auth/refresh-token",
                "/tree",
                "/slow"
            ])
        );
    }

    struct Records(Mutex<Vec<String>>);

    impl Transport for Arc<Records> {
        fn send(&self, request: Request) -> BoxFuture<'_, Result<Response, String>> {
            self.0.lock().unwrap().push(request.url);
            Box::pin(async {
                Ok(Response {
                    status: 302,
                    location: Some("https://sso.example/login?state=abc".into()),
                    body: Ok(String::new()),
                    ..Response::default()
                })
            })
        }
    }

    #[tokio::test]
    async fn never_follows_a_redirect_and_says_where_it_pointed() {
        let calls = Arc::new(Records(Mutex::new(Vec::new())));
        let e = session_over(calls.clone(), "ed")
            .request(get("/tree"))
            .await
            .unwrap_err();
        assert_eq!(
            *calls.0.lock().unwrap(),
            vec!["https://wiki.test/api/config".to_string()]
        );
        assert_eq!(e.status, 302);
        assert_eq!(
            e.message,
            "LeafWiki redirected to https://sso.example/login; burrow does not follow redirects: set LEAFWIKI_URL to the address the wiki answers on"
        );
    }

    #[tokio::test]
    async fn refuses_a_path_that_url_parsing_would_rewrite_before_sending_anything() {
        let wiki = FakeWiki::new();
        let e = session(&wiki, "ed", "pw")
            .request(Spec::new(Method::Delete, "/pages/p1/assets/.."))
            .await
            .unwrap_err();
        assert_eq!(e.status, 0);
        assert!(e.message.starts_with("Not sent"));
        assert!(wiki.seen().is_empty());
    }

    #[test]
    fn reads_leafwiki_error_shapes() {
        let e = failure_of(
            409,
            r#"{"error":{"code":"page_version_conflict","message":"Version conflict"}}"#,
            "PUT /api/pages/1".into(),
        );
        assert_eq!(
            (e.message.as_str(), e.code.as_deref()),
            ("Version conflict", Some("page_version_conflict"))
        );
        assert_eq!(
            failure_of(403, r#"{"error":"Invalid CSRF token"}"#, String::new()).message,
            "Invalid CSRF token"
        );
        assert_eq!(
            failure_of(404, "Page not found\nmore", String::new()).message,
            "Page not found"
        );
        assert_eq!(failure_of(500, "", String::new()).message, "HTTP 500");
    }
}
