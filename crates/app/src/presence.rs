use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use zbus::Connection;

use snellar_core::journal;
use snellar_core::signals::Presence;
use snellar_core::tasks::TaskSet;

pub const IDLE_AFTER: Duration = Duration::from_secs(600);
const POLL: Duration = Duration::from_secs(20);
const ASK: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub idle: Option<bool>,
    pub locked: Option<bool>,
}

pub fn verdict(s: Sample) -> Presence {
    match (s.locked, s.idle) {
        (Some(true), _) => Presence::Locked,
        (_, Some(true)) => Presence::Idle,
        (Some(false), _) | (_, Some(false)) => Presence::Present,
        (None, None) => Presence::Unknown,
    }
}

fn any(answers: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    answers.into_iter().flatten().reduce(|a, b| a || b)
}

pub fn watch(tasks: &TaskSet) -> Arc<AtomicU8> {
    let state = Arc::new(AtomicU8::new(Presence::Unknown.code()));
    let wayland = wayland::start(IDLE_AFTER);
    let out = state.clone();
    tasks.spawn("presence", async move {
        let session = Connection::session().await.ok();
        let system = Connection::system().await.ok();
        let mut told = None;
        loop {
            let idle = match &wayland {
                Some(flag) => Some(flag.load(Ordering::Relaxed)),
                None => idle_by_bus(session.as_ref(), system.as_ref()).await,
            };
            let locked = locked_by_bus(session.as_ref(), system.as_ref()).await;
            let now = verdict(Sample { idle, locked });
            if told != Some(now) {
                journal::info(
                    "presence",
                    format!("{now:?} (idle from {}, lock from the bus)", if wayland.is_some() { "wayland" } else { "the bus" }),
                );
                told = Some(now);
            }
            out.store(now.code(), Ordering::Relaxed);
            tokio::time::sleep(POLL).await;
        }
    });
    state
}

async fn ask<T: DeserializeOwned + zbus::zvariant::Type>(
    conn: Option<&Connection>,
    dest: &str,
    path: &str,
    iface: &str,
    method: &str,
) -> Option<T> {
    let conn = conn?;
    let call = conn.call_method(Some(dest), path, Some(iface), method, &());
    let reply = tokio::time::timeout(ASK, call).await.ok()?.ok()?;
    reply.body().deserialize::<T>().ok()
}

async fn property<T: TryFrom<zbus::OwnedVariant>>(conn: Option<&Connection>, prop: &str) -> Option<T> {
    let conn = conn?;
    let body = ("org.freedesktop.login1.Session", prop);
    let call = conn.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1/session/auto",
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &body,
    );
    let reply = tokio::time::timeout(ASK, call).await.ok()?.ok()?;
    let value = reply.body().deserialize::<zbus::OwnedVariant>().ok()?;
    T::try_from(value).ok()
}

async fn idle_by_bus(session: Option<&Connection>, system: Option<&Connection>) -> Option<bool> {
    let limit = IDLE_AFTER.as_millis() as u64;
    if let Some(ms) = ask::<u64>(
        session,
        "org.gnome.Mutter.IdleMonitor",
        "/org/gnome/Mutter/IdleMonitor/Core",
        "org.gnome.Mutter.IdleMonitor",
        "GetIdletime",
    )
    .await
    {
        return Some(ms >= limit);
    }
    if let Some(s) = ask::<u32>(
        session,
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver",
        "GetSessionIdleTime",
    )
    .await
    {
        return Some(u64::from(s) * 1000 >= limit);
    }
    property::<bool>(system, "IdleHint").await.filter(|idle| *idle)
}

async fn locked_by_bus(session: Option<&Connection>, system: Option<&Connection>) -> Option<bool> {
    any([
        property::<bool>(system, "LockedHint").await,
        ask::<bool>(session, "org.gnome.ScreenSaver", "/org/gnome/ScreenSaver", "org.gnome.ScreenSaver", "GetActive").await,
        ask::<bool>(session, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver", "GetActive")
            .await,
    ])
}

mod wayland {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use wayland_client::protocol::{wl_registry, wl_seat};
    use wayland_client::{Connection, Dispatch, QueueHandle};
    use wayland_protocols::ext::idle_notify::v1::client::{ext_idle_notification_v1, ext_idle_notifier_v1};

    use snellar_core::journal;

    struct Watch {
        seat: Option<wl_seat::WlSeat>,
        notifier: Option<(ext_idle_notifier_v1::ExtIdleNotifierV1, u32)>,
        idle: Arc<AtomicBool>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Watch {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            let wl_registry::Event::Global { name, interface, version } = event else { return };
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => state.seat = Some(registry.bind(name, 1, qh, ())),
                "ext_idle_notifier_v1" => {
                    let version = version.min(2);
                    state.notifier = Some((registry.bind(name, version, qh, ()), version));
                }
                _ => {}
            }
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for Watch {
        fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for Watch {
        fn event(
            _: &mut Self,
            _: &ext_idle_notifier_v1::ExtIdleNotifierV1,
            _: ext_idle_notifier_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for Watch {
        fn event(
            state: &mut Self,
            _: &ext_idle_notification_v1::ExtIdleNotificationV1,
            event: ext_idle_notification_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                ext_idle_notification_v1::Event::Idled => state.idle.store(true, Ordering::Relaxed),
                ext_idle_notification_v1::Event::Resumed => state.idle.store(false, Ordering::Relaxed),
                _ => {}
            }
        }
    }

    pub fn start(after: Duration) -> Option<Arc<AtomicBool>> {
        std::env::var_os("WAYLAND_DISPLAY")?;
        let conn = Connection::connect_to_env().ok()?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let idle = Arc::new(AtomicBool::new(false));
        let mut watch = Watch { seat: None, notifier: None, idle: idle.clone() };
        queue.roundtrip(&mut watch).ok()?;
        let (Some(seat), Some((notifier, version))) = (watch.seat.clone(), watch.notifier.clone()) else {
            journal::info("presence", "the compositor has no ext-idle-notify, idle time comes from the bus");
            return None;
        };
        let ms = after.as_millis().min(u128::from(u32::MAX)) as u32;
        if version >= 2 {
            notifier.get_input_idle_notification(ms, &seat, &qh, ());
        } else {
            notifier.get_idle_notification(ms, &seat, &qh, ());
        }
        std::thread::Builder::new()
            .name("presence".into())
            .spawn(move || {
                while queue.blocking_dispatch(&mut watch).is_ok() {}
                journal::info("presence", "the Wayland connection for idle time closed");
            })
            .ok()?;
        Some(idle)
    }
}
