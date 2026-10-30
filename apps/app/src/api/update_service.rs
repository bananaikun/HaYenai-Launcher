//! Tauri shell for the update service.
//!
//! The logic lives in [`theseus::update_service`]; this module only exposes it
//! to the webview: three commands and one event. Keeping the shell thin is what
//! lets the interesting parts be unit tested without a running application, and
//! it keeps the resolved host off the frontend's capability whitelist.

use std::sync::Arc;
use std::time::Duration;

use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{Emitter, Manager, Runtime, WindowEvent};
use theseus::update_service::{UpdateService, UpdateServiceStatus};

use crate::api::Result;

/// Event the frontend subscribes to for status changes.
pub const STATUS_EVENT: &str = "update-service-status";

/// How often to check whether the main window has been shown yet.
const WINDOW_VISIBILITY_POLL: Duration = Duration::from_millis(500);
/// Upper bound on that wait, so a window that never reports itself visible
/// (for example in lightweight mode) still gets a sampler eventually.
const WINDOW_VISIBILITY_ATTEMPTS: usize = 120;
/// Delay between the window becoming visible and the first probe.
///
/// The first probe competes with the frontend for the runtime and the network;
/// two idle seconds keep it out of the critical path of the first paint.
const FIRST_PROBE_DELAY: Duration = Duration::from_secs(2);

/// Registers the plugin commands, the status event forwarder and the sampler.
///
/// The plugin name is `update-service`, not `update_service`: Tauri identifiers
/// reject underscores, so the module name and the plugin name differ on purpose.
/// Commands are reached as `plugin:update-service|status`.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::new("update-service")
        .invoke_handler(tauri::generate_handler![
            status,
            refresh,
            base_url,
            announcements,
            latest
        ])
        .setup(|app, _api| {
            let service = UpdateService::shared().clone();
            forward_status_events(app.clone(), Arc::clone(&service));
            pause_while_minimised(app.clone(), Arc::clone(&service));
            start_sampling_after_show(app.clone(), service);
            Ok(())
        })
        .build()
}

/// Copies status changes from the library's broadcast channel into a Tauri
/// event, so the frontend renders from events instead of polling.
fn forward_status_events<R: Runtime>(
    app: tauri::AppHandle<R>,
    service: Arc<UpdateService>,
) {
    let mut receiver = service.subscribe();
    tauri::async_runtime::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;

        loop {
            match receiver.recv().await {
                Ok(status) => {
                    if let Err(error) = app.emit(STATUS_EVENT, &status) {
                        tracing::warn!(
                            "Could not emit {STATUS_EVENT} to the frontend: {error}"
                        );
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    // Only the newest snapshot matters; dropping older ones is
                    // exactly what the bounded channel is for.
                    tracing::debug!(
                        "Skipped {skipped} update service status events"
                    );
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// Suspends sampling while the window is minimised and resumes it afterwards.
///
/// The resume path also runs an immediate probe: the indicator would otherwise
/// stay stale for a whole sampling interval after the user restores the window.
fn pause_while_minimised<R: Runtime>(
    app: tauri::AppHandle<R>,
    service: Arc<UpdateService>,
) {
    let Some(window) = app.get_window("main") else {
        tracing::debug!(
            "No main window at plugin setup; the minimised-pause hook is not installed"
        );
        return;
    };

    window.on_window_event(move |event| {
        if !matches!(event, WindowEvent::Resized(_)) {
            return;
        }

        let Some(window) = app.get_window("main") else {
            return;
        };
        match window.is_minimized() {
            Ok(minimized) => {
                if service.is_paused() == minimized {
                    return;
                }
                service.set_paused(minimized);
                if !minimized {
                    service.spawn_probe();
                }
            }
            Err(error) => {
                tracing::debug!("Could not read the window state: {error}");
            }
        }
    });
}

/// Starts the sampler once the main window is on screen.
fn start_sampling_after_show<R: Runtime>(
    app: tauri::AppHandle<R>,
    service: Arc<UpdateService>,
) {
    tauri::async_runtime::spawn(async move {
        for _ in 0..WINDOW_VISIBILITY_ATTEMPTS {
            let visible = app
                .get_window("main")
                .is_some_and(|window| window.is_visible().unwrap_or(false));
            if visible {
                break;
            }
            tokio::time::sleep(WINDOW_VISIBILITY_POLL).await;
        }

        tokio::time::sleep(FIRST_PROBE_DELAY).await;
        service.spawn_sampling();
    });
}

// ── Commands ────────────────────────────────────────────────────

/// The current snapshot. Never performs a network request.
#[tauri::command]
pub async fn status() -> Result<UpdateServiceStatus> {
    Ok(UpdateService::shared().status())
}

/// Triggers a probe and returns the snapshot taken *before* it runs.
///
/// The caller is expected to render the returned state and then wait for the
/// `update-service-status` event; blocking the invoke until the network answers
/// would freeze the retry button for seconds on a dead link.
#[tauri::command]
pub async fn refresh() -> Result<UpdateServiceStatus> {
    let service = UpdateService::shared().clone();
    let snapshot = service.status();
    service.spawn_probe();
    Ok(snapshot)
}

/// The base URL from the last resolution, if there has been one.
#[tauri::command]
pub async fn base_url() -> Result<Option<String>> {
    Ok(UpdateService::shared().base_url())
}

/// Announcements from `{base}api/announcements`.
///
/// Proxied here rather than fetched by the webview: the service address comes
/// from the SRV lookup, so it is not in (and must not be added to) the static
/// `http:` capability list. `Ok(None)` means "offline or no data", which the
/// caller renders as an empty feed — never as an error dialog.
#[tauri::command]
pub async fn announcements() -> Result<Option<Vec<serde_json::Value>>> {
    let Some(payload) = UpdateService::shared().get_json("api/announcements").await
    else {
        return Ok(None);
    };

    // Contract: `{"ok": true, "data": [...]}`. Anything else is treated as no
    // data rather than guessed at.
    let ok = payload
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !ok {
        return Ok(None);
    }

    Ok(payload
        .get("data")
        .and_then(serde_json::Value::as_array)
        .cloned())
}

/// The updater manifest from `{base}latest?channel={channel}`.
///
/// Shares the resolved base with [`base_url`], so the manifest is read from the
/// same host and port the health probe validated.
#[tauri::command]
pub async fn latest(
    channel: String,
) -> Result<Option<serde_json::Value>> {
    // The channel is a fixed vocabulary; anything else would be reflected into
    // the query string, so it is rejected here instead of being forwarded.
    if !matches!(channel.as_str(), "release" | "beta") {
        tracing::warn!("Unknown update channel requested: {channel}");
        return Ok(None);
    }

    Ok(
        UpdateService::shared()
            .get_json(&format!("latest?channel={channel}"))
            .await,
    )
}
