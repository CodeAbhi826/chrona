//! Wayland watcher for wlroots-based compositors (Sway, Hyprland, river,
//! niri, ...): subscribes to `wlr-foreign-toplevel-management` for window
//! events and `ext-idle-notify` for AFK detection, on a single connection.
#![cfg(feature = "wayland")]

use crate::state::Event;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{
    backend::ObjectId, Connection as WlConnection, Dispatch, Proxy as _, QueueHandle,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::ExtIdleNotificationV1;
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self, ZwlrForeignToplevelManagerV1,
};

/// wlr-foreign-toplevel state flags (from the protocol XML):
/// maximized=0, minimized=1, activated=2, fullscreen=3.
const STATE_ACTIVATED: u8 = 2;

/// Per-toplevel info, updated from handle events.
#[derive(Default)]
pub struct ToplevelInfo {
    pub app_id: String,
    pub title: String,
}

pub struct App {
    tx: Sender<Event>,
    manager: Option<ZwlrForeignToplevelManagerV1>,
    notifier: Option<ExtIdleNotifierV1>,
    seat: Option<wl_seat::WlSeat>,
    idle_created: bool,
    toplevels: HashMap<ObjectId, ToplevelInfo>,
}

impl App {
    fn create_idle_notification(&mut self, qh: &QueueHandle<Self>) {
        if self.idle_created {
            return;
        }
        if let (Some(notifier), Some(seat)) = (&self.notifier, &self.seat) {
            self.idle_created = true;
            let _ = notifier.get_idle_notification(60_000, seat, qh, ());
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &WlConnection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zwlr_foreign_toplevel_manager_v1" => {
                    state.manager = Some(registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    ));
                }
                "ext_idle_notifier_v1" => {
                    state.notifier = Some(registry.bind::<ExtIdleNotifierV1, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    ));
                    state.create_idle_notification(qh);
                }
                "wl_seat" => {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(1), qh, ()));
                    state.create_idle_notification(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _manager: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _data: &(),
        _conn: &WlConnection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state
                .toplevels
                .insert(toplevel.id(), ToplevelInfo::default());
        }
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for App {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _conn: &WlConnection,
        _qh: &QueueHandle<Self>,
    ) {
        let id = handle.id();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(t) = state.toplevels.get_mut(&id) {
                    t.title = title;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(t) = state.toplevels.get_mut(&id) {
                    t.app_id = app_id.to_lowercase();
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: states } => {
                let activated = states.contains(&STATE_ACTIVATED);
                if activated {
                    if let Some(t) = state.toplevels.get(&id) {
                        let _ = state.tx.send(Event::Window {
                            app: t.app_id.clone(),
                            title: t.title.clone(),
                        });
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.remove(&id);
                handle.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &ExtIdleNotifierV1,
        _event: ext_idle_notifier_v1::Event,
        _data: &(),
        _conn: &WlConnection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _data: &(),
        _conn: &WlConnection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                let _ = state.tx.send(Event::IdleStart);
            }
            ext_idle_notification_v1::Event::Resumed => {
                let _ = state.tx.send(Event::IdleEnd);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        state: &mut Self,
        _seat: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _data: &(),
        _conn: &WlConnection,
        qh: &QueueHandle<Self>,
    ) {
        state.create_idle_notification(qh);
    }
}

pub fn spawn(tx: Sender<Event>) -> anyhow::Result<()> {
    let conn = WlConnection::connect_to_env()?;
    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut app = App {
        tx,
        manager: None,
        notifier: None,
        seat: None,
        idle_created: false,
        toplevels: HashMap::new(),
    };
    let _registry = display.get_registry(&qh, ());
    queue.roundtrip(&mut app)?;

    if app.manager.is_none() {
        anyhow::bail!("compositor does not expose zwlr_foreign_toplevel_management_v1");
    }

    std::thread::Builder::new()
        .name("chrona-wayland".into())
        .spawn(move || loop {
            if let Err(e) = queue.blocking_dispatch(&mut app) {
                eprintln!("[chronad] wayland dispatch error: {e}");
                break;
            }
        })?;
    Ok(())
}
