use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::leafwiki::Account;

/// The burrow executable under test: `BURROW_BIN`, or the one cargo built with these tests.
fn burrow_bin() -> String {
    std::env::var("BURROW_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_burrow").to_string())
}

pub struct Answer {
    pub text: String,
    pub is_error: bool,
}

impl Answer {
    #[track_caller]
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|e| panic!("not JSON ({e}):\n{}", self.text))
    }
}

/// An MCP client talking to a burrow process over stdio, as a client app does.
pub struct Burrow {
    child: Child,
    stdin: ChildStdin,
    stdout: Receiver<String>,
    log: Arc<Mutex<Vec<String>>>,
    next_id: u64,
    /// Every tool this client called.
    pub touched: BTreeSet<String>,
}

impl Burrow {
    /// burrow signed in to the wiki at `url` as `user` (none: no account), with `env` on top.
    pub fn connect(url: &str, user: Option<&Account>, env: &[(&str, &str)]) -> Burrow {
        let mut cmd = Command::new(burrow_bin());
        // Only what the process needs, as MCP clients do: no LEAFWIKI_*, BURROW_* or
        // proxy settings from the shell that runs the tests.
        cmd.env_clear();
        for key in ["PATH", "HOME", "SystemRoot", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        cmd.env("LEAFWIKI_URL", url);
        if let Some(user) = user {
            cmd.env("LEAFWIKI_USERNAME", user.username)
                .env("LEAFWIKI_PASSWORD", user.password);
        }
        cmd.envs(env.iter().copied());
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("cannot start {}: {e}", burrow_bin()));

        let (tx, stdout) = mpsc::channel();
        let out = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let log = Arc::new(Mutex::new(Vec::new()));
        let err = child.stderr.take().unwrap();
        let sink = Arc::clone(&log);
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                sink.lock()
                    .unwrap()
                    .push(line.strip_prefix("burrow: ").unwrap_or(&line).to_string());
            }
        });

        let stdin = child.stdin.take().unwrap();
        let mut burrow = Burrow {
            child,
            stdin,
            stdout,
            log,
            next_id: 0,
            touched: BTreeSet::new(),
        };
        let init = burrow.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "burrow-test", "version": "1" },
            }),
        );
        assert_eq!(
            init["serverInfo"]["name"],
            "burrow",
            "{} is not burrow",
            burrow_bin()
        );
        burrow.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        burrow
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}")
            .and_then(|()| self.stdin.flush())
            .expect("burrow closed its input");
    }

    #[track_caller]
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let line = self
                .stdout
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| {
                    panic!(
                        "no answer to {method} within 30 s; burrow's log:\n{}",
                        self.log.lock().unwrap().join("\n")
                    )
                });
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("not JSON-RPC ({e}): {line}"));
            if message["id"] != id {
                continue;
            }
            if let Some(error) = message.get("error") {
                panic!("{method} failed: {error}");
            }
            return message["result"].clone();
        }
    }

    pub fn call(&mut self, name: &str, args: Value) -> Answer {
        self.touched.insert(name.to_string());
        let result = self.request("tools/call", json!({ "name": name, "arguments": args }));
        let text = result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Answer {
            text,
            is_error: result["isError"] == true,
        }
    }

    /// Like `call`, but fails the test when the tool reports an error.
    #[track_caller]
    pub fn must(&mut self, name: &str, args: Value) -> Answer {
        let answer = self.call(name, args);
        assert!(!answer.is_error, "{name} failed:\n{}", answer.text);
        answer
    }

    pub fn tools(&mut self) -> Vec<String> {
        let list = self.request("tools/list", json!({}));
        list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    /// burrow's log lines so far, without the `burrow: ` prefix.
    pub fn log(&self) -> Vec<String> {
        // stderr is read on its own thread: give lines written before the last answer time to arrive.
        std::thread::sleep(Duration::from_millis(200));
        self.log.lock().unwrap().clone()
    }
}

impl Drop for Burrow {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
