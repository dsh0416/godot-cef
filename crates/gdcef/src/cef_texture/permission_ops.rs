use crate::browser::{App, PendingPermissionAggregate, PendingPermissionDecision};

pub(crate) fn resolve_permission_request(app: &App, request_id: i64, grant: bool) -> bool {
    let Some(state) = app.state.as_ref() else {
        godot::global::godot_warn!(
            "[CefTexture] Cannot resolve permission request {}: no active browser",
            request_id
        );
        return false;
    };

    let (decision, remaining_for_callback) = {
        let Ok(mut pending) = state.pending_permission_requests.lock() else {
            godot::global::godot_warn!("[CefTexture] Failed to lock pending permission requests");
            return false;
        };
        let Some(decision) = pending.remove(&request_id) else {
            return {
                godot::global::godot_warn!(
                    "[CefTexture] Unknown or stale permission request id: {}",
                    request_id
                );
                false
            };
        };

        let token = match &decision {
            PendingPermissionDecision::Media { callback_token, .. } => *callback_token,
            PendingPermissionDecision::Prompt { callback_token, .. } => *callback_token,
        };
        let remaining_for_callback = pending
            .values()
            .filter(|entry| match entry {
                PendingPermissionDecision::Media { callback_token, .. } => *callback_token == token,
                PendingPermissionDecision::Prompt { callback_token, .. } => {
                    *callback_token == token
                }
            })
            .count();

        (decision, remaining_for_callback)
    };

    let mut aggregates = match state.pending_permission_aggregates.lock() {
        Ok(aggregates) => aggregates,
        Err(_) => {
            godot::global::godot_warn!("[CefTexture] Failed to lock pending permission aggregates");
            return false;
        }
    };

    match decision {
        PendingPermissionDecision::Media {
            callback,
            permission_bit,
            callback_token,
        } => {
            let entry = aggregates
                .entry(callback_token)
                .or_insert_with(|| PendingPermissionAggregate::new_media(callback.clone(), 0));
            match entry {
                PendingPermissionAggregate::Media { granted_mask, .. } => {
                    if grant {
                        *granted_mask |= permission_bit;
                    }
                }
                PendingPermissionAggregate::Prompt { .. } => {
                    godot::global::godot_warn!(
                        "[CefTexture] Permission aggregate type mismatch for callback token {}",
                        callback_token
                    );
                    *entry = PendingPermissionAggregate::new_media(
                        callback.clone(),
                        if grant { permission_bit } else { 0 },
                    );
                }
            }

            if remaining_for_callback == 0
                && let Some(PendingPermissionAggregate::Media {
                    callback,
                    granted_mask,
                }) = aggregates.remove(&callback_token)
            {
                let allowed_mask = if granted_mask == 0 {
                    media_permission_none_mask()
                } else {
                    granted_mask
                };
                use cef::ImplMediaAccessCallback;
                callback.cont(allowed_mask);
            }
        }
        PendingPermissionDecision::Prompt {
            callback,
            callback_token,
            ..
        } => {
            let entry = aggregates
                .entry(callback_token)
                .or_insert_with(|| PendingPermissionAggregate::new_prompt(callback.clone(), true));
            match entry {
                PendingPermissionAggregate::Prompt { all_granted, .. } => {
                    *all_granted &= grant;
                }
                PendingPermissionAggregate::Media { .. } => {
                    godot::global::godot_warn!(
                        "[CefTexture] Permission aggregate type mismatch for callback token {}",
                        callback_token
                    );
                    *entry = PendingPermissionAggregate::new_prompt(callback.clone(), grant);
                }
            }

            if remaining_for_callback == 0
                && let Some(PendingPermissionAggregate::Prompt {
                    callback,
                    all_granted,
                }) = aggregates.remove(&callback_token)
            {
                let result = if all_granted {
                    cef::PermissionRequestResult::ACCEPT
                } else {
                    cef::PermissionRequestResult::DENY
                };
                use cef::ImplPermissionPromptCallback;
                callback.cont(result);
            }
        }
    }

    true
}

fn media_permission_none_mask() -> u32 {
    #[cfg(target_os = "windows")]
    {
        cef::MediaAccessPermissionTypes::NONE.get_raw() as u32
    }
    #[cfg(not(target_os = "windows"))]
    {
        cef::MediaAccessPermissionTypes::NONE.get_raw()
    }
}
