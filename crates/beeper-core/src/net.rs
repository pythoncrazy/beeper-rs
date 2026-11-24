//! The one way out of the process: every HTTP client of beeper-rs is built here, and the demo closes it. A demo
//! process calls [`go_offline`] once its store is open, and from then on each connection of these clients fails
//! before it looks up a name or opens a socket, so that no flow of the demo reaches Beeper, KLIPY or a web page with
//! the demo's made-up token. The clippy configuration of the workspace forbids building a client anywhere else.

use std::sync::atomic::{AtomicBool, Ordering};

/// The error of a connection that the demo refused.
pub const OFFLINE_ERROR: &str = "The demo does not use the network";

/// Whether this process runs the demo.
static OFFLINE: AtomicBool = AtomicBool::new(false);

/// Closes the network for the rest of the process: the demo never opens it again, a sign-in restarts the app.
pub fn go_offline() {
    OFFLINE.store(true, Ordering::SeqCst);
}

/// Whether [`go_offline`] closed the network.
pub fn offline() -> bool {
    OFFLINE.load(Ordering::SeqCst)
}

/// The client of `builder`, whose connections fail at once while `closed` is set: the gate comes before the DNS
/// lookup, the proxy and the TCP connection of reqwest's connector.
fn gated(builder: reqwest::ClientBuilder, closed: &'static AtomicBool) -> reqwest::Client {
    crate::cloud::install_tls();
    let gate = move |dst| if closed.load(Ordering::SeqCst) { Err(tower::BoxError::from(OFFLINE_ERROR)) } else { Ok(dst) };
    builder.connector_layer(tower::filter::FilterLayer::new(gate)).build().expect("build the HTTP client")
}

/// A new HTTP client of the settings of `builder`, closed in the demo.
pub(crate) fn client(builder: reqwest::ClientBuilder) -> reqwest::Client {
    gated(builder, &OFFLINE)
}

/// The settings of a new HTTP client, for [`client`].
#[allow(clippy::disallowed_methods, reason = "the clients of this module are the ones that the demo closes")]
pub(crate) fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
}

/// The Matrix client of a homeserver, which the demo never builds: matrix-sdk has its own HTTP client.
#[allow(clippy::disallowed_methods, reason = "the only Matrix client, which the demo does not build")]
pub(crate) fn matrix_client() -> Result<matrix_sdk::ClientBuilder, String> {
    match offline() {
        true => Err(OFFLINE_ERROR.into()),
        false => Ok(matrix_sdk::Client::builder()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// A listener on a free loopback port that counts the connections that reach it.
    fn recorder() -> (String, Arc<AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let url = format!("http://{}", listener.local_addr().expect("a bound port"));
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                seen.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });
        (url, count)
    }

    /// The messages of an error and of its sources.
    fn chain(e: &dyn std::error::Error) -> String {
        let mut text = e.to_string();
        let mut source = e.source();
        while let Some(s) = source {
            text = format!("{text}: {s}");
            source = s.source();
        }
        text
    }

    #[tokio::test]
    async fn a_closed_client_connects_nowhere_and_looks_up_no_name() {
        static CLOSED: AtomicBool = AtomicBool::new(false);
        let client = gated(builder(), &CLOSED);
        let (url, count) = recorder();
        // The listener closes the connection without an answer.
        let _ = client.get(&url).send().await;
        assert_eq!(count.load(Ordering::SeqCst), 1, "an open client connects");
        CLOSED.store(true, Ordering::SeqCst);
        let e = client.get(&url).send().await.expect_err("a closed client does not connect");
        assert!(chain(&e).contains(OFFLINE_ERROR), "{}", chain(&e));
        // A host name fails at the gate too, before its lookup.
        let named = url.replace("127.0.0.1", "localhost");
        let e = client.get(&named).send().await.expect_err("a closed client looks up no name");
        assert!(chain(&e).contains(OFFLINE_ERROR), "{}", chain(&e));
        assert_eq!(count.load(Ordering::SeqCst), 1, "no connection after the gate closed");
    }
}
