//! One process-wide CEF UI pump, independent of browser nodes and rendering.

use std::cell::RefCell;

use cef_app::message_pump::MESSAGE_PUMP;
use godot::classes::{Engine, Os, SceneTree};
use godot::prelude::*;

struct PumpConnection {
    tree_id: InstanceId,
    callable: Callable,
}

thread_local! {
    static CONNECTION: RefCell<Option<PumpConnection>> = const { RefCell::new(None) };
}

/// Install on the Godot main thread before initializing CEF. A SceneTree signal
/// is used instead of an instance's process callback so pausing, hiding, freeing,
/// or replacing browser nodes cannot stop browser-close callbacks.
///
/// The signal still depends on Godot's main loop: global FPS limits, minimized
/// window throttling, and a blocked main thread can delay CEF deadlines.
pub(crate) fn ensure_installed() -> Result<(), String> {
    let os = Os::singleton();
    if os.get_thread_caller_id() != os.get_main_thread_id() {
        return Err("CEF must be initialized on Godot's main thread".into());
    }

    let mut tree = Engine::singleton()
        .get_main_loop()
        .and_then(|main_loop| main_loop.try_cast::<SceneTree>().ok())
        .ok_or("CEF's external message pump requires a SceneTree main loop")?;

    CONNECTION.with_borrow_mut(|connection| {
        if connection.as_ref().is_some_and(|existing| {
            existing.tree_id == tree.instance_id()
                && tree.is_connected("process_frame", &existing.callable)
        }) {
            return Ok(());
        }

        if let Some(previous) = connection.take() {
            disconnect(previous);
        }

        // from_fn is bound to the creating thread. No Godot callable is invoked
        // by OnScheduleMessagePumpWork, which only touches the Rust scheduler.
        let callable = Callable::from_fn("gdcef_message_pump", |_| {
            MESSAGE_PUMP.run_due(cef::do_message_loop_work);
        });
        let error = tree.connect("process_frame", &callable);
        if error != godot::global::Error::OK {
            return Err(format!("Failed to connect CEF's message pump: {error:?}"));
        }
        *connection = Some(PumpConnection {
            tree_id: tree.instance_id(),
            callable,
        });
        Ok(())
    })
}

/// Called once CEF initialization succeeds; installation alone must not pump
/// an uninitialized runtime.
pub(crate) fn activate() {
    MESSAGE_PUMP.activate();
}

pub(crate) fn uninstall() {
    MESSAGE_PUMP.deactivate();
    CONNECTION.with_borrow_mut(|connection| {
        if let Some(connection) = connection.take() {
            disconnect(connection);
        }
    });
}

fn disconnect(connection: PumpConnection) {
    if let Ok(mut tree) = Gd::<SceneTree>::try_from_instance_id(connection.tree_id)
        && tree.is_connected("process_frame", &connection.callable)
    {
        tree.disconnect("process_frame", &connection.callable);
    }
}
