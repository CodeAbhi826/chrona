//! D-Bus intake: owns `org.chrona.Watcher` on the session bus so the Chrona
//! KWin script can push window activations straight into the daemon.
#![cfg(feature = "dbus")]

use crate::state::Event;
use std::sync::mpsc::Sender;
use zbus::interface;

struct Watcher {
    tx: Sender<Event>,
}

#[interface(name = "org.chrona.Watcher")]
impl Watcher {
    /// Called by the Chrona KWin script on every window activation.
    /// Arguments mirror KWin's client properties: resource_name,
    /// resource_class (the app id), caption (the window title).
    async fn active_window_changed(&self, app: &str, app_class: &str, title: &str) {
        let class = app_class.trim().to_lowercase();
        let name = app.trim().to_lowercase();
        let app_id = if class.is_empty() { name } else { class };
        if !app_id.is_empty() {
            let _ = self.tx.send(Event::Window {
                app: app_id,
                title: title.to_string(),
            });
        }
    }

    /// Simple liveness probe (`qdbus6 org.chrona.Watcher /org/chrona/Watcher
    /// org.chrona.Watcher.Ping`).
    async fn ping(&self) -> &str {
        "chrona"
    }
}

pub fn spawn_intake(tx: Sender<Event>) {
    std::thread::Builder::new()
        .name("chrona-dbus".into())
        .spawn(move || {
            if let Err(e) = zbus::block_on(run(tx)) {
                eprintln!(
                    "[chronad] D-Bus intake unavailable ({e}); KWin script cannot reach the daemon"
                );
            }
        })
        .ok();
}

async fn run(tx: Sender<Event>) -> zbus::Result<()> {
    let iface = Watcher { tx };
    let _conn = zbus::connection::Builder::session()?
        .name("org.chrona.Watcher")?
        .serve_at("/org/chrona/Watcher", iface)?
        .build()
        .await?;
    eprintln!("[chronad] D-Bus intake ready at org.chrona.Watcher");
    std::future::pending::<()>().await;
    Ok(())
}
