//! The session bus of the tests: a bus in the process with no tray host, notification server or portal, like a
//! session without them, so that the tests neither need a session bus nor reach the one of the desktop that runs them.
//! A test serves a name of its own with [`serve`].
use crate::prelude::*;
use iced::futures::channel::mpsc;
use iced::futures::StreamExt;
use zbus::message::{Flags, Type};
use zbus::{Connection, Message};

/// The unique name that the bus gives the app.
pub(crate) const ME: &str = ":1.1";

/// The names that tests serve, with the calls to each.
static SERVED: Mutex<Vec<(String, mpsc::UnboundedSender<Message>)>> = Mutex::new(Vec::new());

/// The bus end of the app's connection, which answers the calls to the names that tests serve.
static BUS: OnceLock<Connection> = OnceLock::new();

/// The runtime of both ends, which outlives the runtime of any one test.
static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a tokio runtime"));

/// Connects the app to the bus: a socket pair whose other end answers `org.freedesktop.DBus` like a bus daemon, and
/// `ServiceUnknown` to the calls to names that nothing serves.
pub(crate) async fn connect() -> zbus::Result<Connection> {
    let app = RUNTIME.spawn(async {
        let (app, bus) = tokio::net::UnixStream::pair()?;
        let bus = zbus::connection::Builder::unix_stream(bus).server(zbus::Guid::generate())?.p2p();
        tokio::spawn(async move {
            // The stream exists before the socket reader runs, so it has the `Hello` of the app's handshake.
            let mut calls = bus.build_message_stream().await.expect("the bus end of the socket pair");
            let conn = BUS.get_or_init(|| Connection::from(&calls)).clone();
            while let Some(Ok(m)) = calls.next().await {
                answer(&conn, m).await;
            }
        });
        zbus::connection::Builder::unix_stream(app).build().await
    });
    app.await.expect("the test bus connects")
}

async fn answer(conn: &Connection, m: Message) {
    let h = m.header();
    if h.message_type() != Type::MethodCall {
        return;
    }
    let dest = h.destination().map(|d| d.to_string()).unwrap_or_default();
    let reply = match (dest.as_str(), h.member().map(|m| m.as_str())) {
        ("org.freedesktop.DBus", Some("Hello")) => conn.reply(&h, &ME).await,
        // DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER and DBUS_RELEASE_NAME_REPLY_RELEASED.
        ("org.freedesktop.DBus", Some("RequestName" | "ReleaseName")) => conn.reply(&h, &1u32).await,
        ("org.freedesktop.DBus", Some("AddMatch" | "RemoveMatch")) => conn.reply(&h, &()).await,
        ("org.freedesktop.DBus", member) => {
            let error = "org.freedesktop.DBus.Error.UnknownMethod";
            conn.reply_error(&h, error, &format!("the test bus has no {member:?}")).await
        }
        _ => {
            let served = SERVED.lock().expect("the served names").iter().find(|s| s.0 == dest).map(|s| s.1.clone());
            if let Some(tx) = served {
                drop(tx.unbounded_send(m.clone()));
                return;
            }
            if h.primary().flags().contains(Flags::NoReplyExpected) {
                return;
            }
            let error = "org.freedesktop.DBus.Error.ServiceUnknown";
            conn.reply_error(&h, error, &format!("The name {dest} was not provided by any .service files")).await
        }
    };
    reply.expect("the test bus answers");
}

/// Serves `name` on the bus: the calls to it, and the connection that answers them and sends its signals.
pub(crate) async fn serve(name: &str) -> (mpsc::UnboundedReceiver<Message>, Connection) {
    let (tx, rx) = mpsc::unbounded();
    SERVED.lock().expect("the served names").push((name.to_owned(), tx));
    crate::notify::bus().await.expect("the test bus");
    (rx, BUS.get().expect("the test bus is up").clone())
}
