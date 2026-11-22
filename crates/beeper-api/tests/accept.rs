//! The server outlives a failed accept. It runs in its own test process, because it lowers the limit of open files.
#![cfg(target_os = "linux")]

use beeper_api::server::serve_http;
use beeper_core::config::CentralConfig;
use beeper_core::store::{BeeperStore, StoreLock};
use beeper_core::testing::demo_config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};

fn nofile(soft: &str) {
    let pid = std::process::id().to_string();
    let set = std::process::Command::new("prlimit").args(["--pid", &pid, &format!("--nofile={soft}:")]).status();
    assert!(set.expect("run prlimit").success(), "set the soft limit of open files to {soft}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_keeps_answering_after_running_out_of_files() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = CentralConfig { port, ..demo_config("accept-files") };
    tokio::spawn(serve_http(listener, StoreLock::shared(BeeperStore::from_config(cfg))));
    let soft = std::fs::read_to_string("/proc/self/limits").unwrap();
    let soft = soft.lines().find(|l| l.starts_with("Max open files")).unwrap().split_whitespace().nth(3).unwrap();
    let soft = soft.to_string();
    let open = std::fs::read_dir("/proc/self/fd").unwrap().count();
    nofile(&(open + 16).to_string());
    // The clients take the last files, so the server cannot accept their connections.
    let mut clients = Vec::new();
    while let Ok(Ok(c)) = timeout(Duration::from_secs(1), TcpStream::connect(("127.0.0.1", port))).await {
        clients.push(c);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(clients);
    nofile(&soft);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(b"GET /v1/info HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n").await.unwrap();
    let mut out = Vec::new();
    timeout(Duration::from_secs(10), stream.read_to_end(&mut out)).await.expect("the server answers").unwrap();
    assert!(out.starts_with(b"HTTP/1.1 200 OK"), "{}", String::from_utf8_lossy(&out));
}
