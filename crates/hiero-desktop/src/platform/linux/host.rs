//! AppIndicator owns registration and re-registration. This observer only reports
//! host availability: recreating the indicator on NameOwnerChanged duplicates it.
use crate::events::Sender;
use gtk::{gio, glib};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const PATH: &str = "/StatusNotifierWatcher";
pub const MISSING: &str = "Waiting for a tray host; on GNOME install and enable AppIndicator Support from https://extensions.gnome.org/extension/615/appindicator-support/";
type Subscription = Option<(gio::DBusConnection, gio::SignalSubscriptionId)>;
pub struct HostObserver {
    unwatch: Option<Box<dyn FnOnce()>>,
    subscription: Arc<Mutex<Subscription>>,
}
fn clear_subscription(subscription: &Mutex<Subscription>) {
    if let Some((connection, id)) = subscription
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        connection.signal_unsubscribe(id);
    }
}
impl HostObserver {
    pub fn new(state: Arc<Mutex<Option<String>>>, wake: Sender) -> Self {
        let subscription = Arc::new(Mutex::new(None));
        let appeared_subscription = subscription.clone();
        let vanished_subscription = subscription.clone();
        let generation = Arc::new(AtomicU64::new(0));
        let vanished_state = state.clone();
        let vanished_wake = wake.clone();
        let vanished_generation = generation.clone();
        let watch = gio::bus_watch_name(
            gio::BusType::Session,
            WATCHER,
            gio::BusNameWatcherFlags::NONE,
            move |connection, _, owner| {
                clear_subscription(&appeared_subscription);
                let signal_state = state.clone();
                let signal_wake = wake.clone();
                let signal_generation = generation.clone();
                let id = connection.signal_subscribe(
                    Some(owner),
                    Some(WATCHER),
                    None,
                    Some(PATH),
                    None,
                    gio::DBusSignalFlags::NONE,
                    move |_, _, _, _, signal, _| {
                        let available = match signal {
                            "StatusNotifierHostRegistered" => true,
                            "StatusNotifierHostUnregistered" => false,
                            _ => return,
                        };
                        signal_generation.fetch_add(1, Ordering::SeqCst);
                        *signal_state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            (!available).then(|| MISSING.into());
                        signal_wake.wake();
                    },
                );
                *appeared_subscription
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some((connection.clone(), id));
                let current = generation.fetch_add(1, Ordering::SeqCst) + 1;
                let state = state.clone();
                let wake = wake.clone();
                let generation = generation.clone();
                connection.call(
                    Some(owner),
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&(WATCHER, "IsStatusNotifierHostRegistered").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                    gio::Cancellable::NONE,
                    move |result| {
                        if generation.load(Ordering::SeqCst) != current {
                            return;
                        }
                        let available = result
                            .ok()
                            .and_then(|value| value.get::<(glib::Variant,)>())
                            .and_then(|(value,)| value.get::<bool>())
                            .unwrap_or(false);
                        *state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            (!available).then(|| MISSING.into());
                        wake.wake();
                    },
                );
            },
            move |_, _| {
                clear_subscription(&vanished_subscription);
                vanished_generation.fetch_add(1, Ordering::SeqCst);
                *vanished_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(MISSING.into());
                vanished_wake.wake();
            },
        );
        Self {
            unwatch: Some(Box::new(move || gio::bus_unwatch_name(watch))),
            subscription,
        }
    }
}
impl Drop for HostObserver {
    fn drop(&mut self) {
        clear_subscription(&self.subscription);
        if let Some(unwatch) = self.unwatch.take() {
            unwatch();
        }
    }
}
use glib::variant::ToVariant;
