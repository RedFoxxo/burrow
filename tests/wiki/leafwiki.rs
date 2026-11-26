use std::fs::{self, File};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

/// The LeafWiki release burrow is tested against. Moving it is a deliberate change.
pub const LEAFWIKI_VERSION: &str = "v0.14.2";

pub struct Account {
    pub username: &'static str,
    pub password: &'static str,
}

const ADMIN: Account = Account {
    username: "admin",
    password: "admin-pass-123",
};
pub const EDITOR: Account = Account {
    username: "editor",
    password: "editor-pass-123",
};
pub const VIEWER: Account = Account {
    username: "viewer",
    password: "viewer-pass-123",
};

/// Runs one future to completion: the harness is synchronous, reqwest is not.
fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// burrow's reqwest brings no TLS provider; like `main.rs`, the harness installs ring.
fn http() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

fn tmp() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn asset() -> String {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        other => panic!("No LeafWiki {LEAFWIKI_VERSION} build for {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => panic!("No LeafWiki {LEAFWIKI_VERSION} build for {other}"),
    };
    let ext = if os == "windows" { ".exe" } else { "" };
    format!("leafwiki-{LEAFWIKI_VERSION}-{os}-{arch}{ext}")
}

async fn download(client: &reqwest::Client, url: &str) -> Vec<u8> {
    let r = client
        .get(url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("download of {url} failed: {e}"));
    assert!(
        r.status().is_success(),
        "download of {url} failed: {}",
        r.status()
    );
    r.bytes().await.unwrap().to_vec()
}

/// The pinned binary under `target/tmp/`, downloaded once and checked against the release's sha256.
fn binary() -> PathBuf {
    static DOWNLOAD: Mutex<()> = Mutex::new(());
    let _one_at_a_time = DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner());
    let name = asset();
    let file = tmp().join("leafwiki").join(&name);
    if file.exists() {
        return file;
    }
    let base =
        format!("https://github.com/perber/leafwiki/releases/download/{LEAFWIKI_VERSION}/{name}");
    let (bytes, sum) = block_on(async {
        let client = http();
        (
            download(&client, &base).await,
            download(&client, &format!("{base}.sha256")).await,
        )
    });
    let expected = String::from_utf8_lossy(&sum)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let actual: String = ring::digest::digest(&ring::digest::SHA256, &bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        actual, expected,
        "{name}: sha256 differs from the release's"
    );
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    let partial = file.with_file_name(format!("{name}.download"));
    fs::write(&partial, &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::rename(&partial, &file).unwrap();
    file
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A running LeafWiki; stopped, and its data removed, when dropped.
pub struct Wiki {
    pub url: String,
    child: Child,
    data: PathBuf,
    log: PathBuf,
}

impl Wiki {
    /// Starts LeafWiki with throwaway data, revisions and link refactoring on,
    /// and adds an editor and a viewer. `extra` arguments go to LeafWiki.
    pub fn start(extra: &[&str]) -> Wiki {
        let bin = binary();
        let port = free_port();
        let id = format!("leafwiki-{}-{port}", std::process::id());
        let data = tmp().join(&id);
        let log = tmp().join(format!("{id}.log"));
        let _ = fs::remove_dir_all(&data);
        fs::create_dir_all(&data).unwrap();
        let out = File::create(&log).unwrap();
        let child = Command::new(bin)
            .args([
                format!("--port={port}"),
                "--host=127.0.0.1".into(),
                format!("--data-dir={}", data.display()),
                "--jwt-secret=burrow-test-secret-burrow-test-secret".into(),
                format!("--admin-password={}", ADMIN.password),
                "--allow-insecure=true".into(),
                "--enable-revision".into(),
                "--enable-link-refactor".into(),
                "--revision-coalesce-window=0".into(),
            ])
            .args(extra)
            .stdin(Stdio::null())
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap();
        let mut wiki = Wiki {
            url: format!("http://127.0.0.1:{port}"),
            child,
            data,
            log,
        };
        wiki.wait_until_up();
        block_on(add_users(&wiki.url));
        wiki
    }

    fn wait_until_up(&mut self) {
        let health = format!("{}/api/health", self.url);
        let child = &mut self.child;
        let up = block_on(async {
            let client = http();
            for _ in 0..100 {
                if matches!(client.get(&health).send().await, Ok(r) if r.status().is_success()) {
                    return true;
                }
                if child.try_wait().unwrap().is_some() {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            false
        });
        assert!(
            up,
            "LeafWiki did not start:\n{}",
            fs::read_to_string(&self.log).unwrap_or_default()
        );
    }
}

impl Drop for Wiki {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.data);
        let _ = fs::remove_file(&self.log);
    }
}

/// Signs in as the admin the way a browser does and creates the test accounts.
async fn add_users(url: &str) {
    let client = http();
    let login = client
        .post(format!("{url}/api/auth/login"))
        .header("content-type", "application/json")
        .body(json!({ "identifier": ADMIN.username, "password": ADMIN.password }).to_string())
        .send()
        .await
        .unwrap();
    let headers = login.headers().clone();
    let status = login.status();
    assert!(
        status.is_success(),
        "admin sign-in failed: {status} {}",
        login.text().await.unwrap_or_default()
    );
    let cookie = headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|c| c.split(';').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("; ");
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    for (user, role) in [(&EDITOR, "editor"), (&VIEWER, "viewer")] {
        let r = client
            .post(format!("{url}/api/users"))
            .header("content-type", "application/json")
            .header("cookie", &cookie)
            .header("x-csrf-token", &csrf)
            .body(
                json!({
                    "username": user.username,
                    "email": format!("{}@example.com", user.username),
                    "password": user.password,
                    "role": role,
                })
                .to_string(),
            )
            .send()
            .await
            .unwrap();
        let status = r.status();
        assert!(
            status.is_success(),
            "creating {} failed: {status} {}",
            user.username,
            r.text().await.unwrap_or_default()
        );
    }
}
