//! Settings preference callback wiring.
//!
//! These callbacks share one lifecycle and persistence boundary: update the
//! session immediately where needed, then write through the core when it is
//! available. Keeping them together prevents the application composition root
//! from accumulating one closure per setting.

use super::*;

pub(super) fn register_settings_preference_callbacks(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let app_weak = app.as_weak();
    let state_for_notifications = Rc::clone(state);
    let runtime_for_notifications = Rc::clone(runtime);
    app.on_save_notification_settings(move |enabled, sound, scope| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_notifications.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Notification preferences are active for this session.",
            ));
            return;
        };
        match runtime_for_notifications.block_on(core.set_notification_settings(
            enabled,
            sound,
            scope.as_str(),
        )) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Notification preferences saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail(
                    "Could not save notifications: {}",
                    error,
                ))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_sync_interval = Rc::clone(state);
    let runtime_for_sync_interval = Rc::clone(runtime);
    app.on_save_sync_interval(move |minutes| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_sync_interval.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Sync interval is active for this session.",
            ));
            return;
        };
        match runtime_for_sync_interval.block_on(core.set_sync_interval_minutes(i64::from(minutes)))
        {
            Ok(()) => {
                app.set_sync_status(UiMessage::detail(
                    "Sync interval set to {} minutes.",
                    minutes,
                ))
            }
            Err(error) => {
                app.set_sync_status(UiMessage::detail(
                    "Could not save sync interval: {}",
                    error,
                ))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_read_on_open = Rc::clone(state);
    let runtime_for_read_on_open = Rc::clone(runtime);
    app.on_save_mark_read_on_open(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        state_for_read_on_open.borrow_mut().mark_read_on_open = enabled;
        let Some(core) = state_for_read_on_open.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Message-opening preference updated for this session.",
            ));
            return;
        };
        match runtime_for_read_on_open.block_on(core.set_mark_read_on_open(enabled)) {
            Ok(()) => app.set_sync_status(UiMessage::plain(
                "Message-opening preference saved.",
            )),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save message-opening preference: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_theme = Rc::clone(state);
    let runtime_for_theme = Rc::clone(runtime);
    app.on_save_theme(move |theme| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_theme.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Theme updated for this session."));
            return;
        };
        match runtime_for_theme.block_on(core.set_theme(&theme)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Theme preference saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save theme: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_language = Rc::clone(state);
    let runtime_for_language = Rc::clone(runtime);
    app.on_save_language(move |language| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        apply_language(&app, language.as_str());
        let Some(core) = state_for_language.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Language updated for this session."));
            return;
        };
        match runtime_for_language.block_on(core.set_language(language.as_str())) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Language preference saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save language: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_sidebar_icons = Rc::clone(state);
    let runtime_for_sidebar_icons = Rc::clone(runtime);
    app.on_save_sidebar_icon_style(move |monochrome| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_sidebar_icons.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Sidebar icon style updated for this session.",
            ));
            return;
        };
        match runtime_for_sidebar_icons.block_on(core.set_monochrome_sidebar_icons(monochrome)) {
            Ok(()) => app.set_sync_status(UiMessage::plain(
                "Sidebar icon preference saved.",
            )),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save sidebar icon preference: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_avatars = Rc::clone(state);
    let runtime_for_avatars = Rc::clone(runtime);
    app.on_save_avatar_visibility(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_avatars.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Avatar preference updated for this session.",
            ));
            return;
        };
        match runtime_for_avatars.block_on(core.set_show_avatars(enabled)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Avatar preference saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail(
                    "Could not save avatar preference: {}",
                    error,
                ))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_workspace_layout = Rc::clone(state);
    let runtime_for_workspace_layout = Rc::clone(runtime);
    app.on_save_workspace_layout(move |layout| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_workspace_layout.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Workspace layout updated for this session.",
            ));
            return;
        };
        match runtime_for_workspace_layout.block_on(core.set_workspace_layout(layout.as_str())) {
            Ok(()) => app.set_sync_status(UiMessage::plain(
                "Workspace layout preference saved.",
            )),
            Err(error) => {
                app.set_sync_status(UiMessage::detail(
                    "Could not save workspace layout: {}",
                    error,
                ))
            }
        }
    });
}
