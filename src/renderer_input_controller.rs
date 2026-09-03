//! Native email-document pointer, scrolling, keyboard, link, and clipboard
//! callback wiring.

use super::*;
use crate::renderer::InputModifiers;

pub(super) fn register_renderer_input_callbacks(
    app: &AppWindow,
    email_renderer: &Rc<RefCell<GpuEmailRenderer>>,
    use_wgpu: bool,
) {
    let app_weak = app.as_weak();
    app.on_open_email_link(move |url| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        match open_email_link(url.as_str()) {
            Ok(()) => app.set_render_status(UiMessage::detail("Opened {}", url)),
            Err(error) => {
                app.set_render_status(UiMessage::detail("Could not open link: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    app.on_copy_email_status(move |status| {
        if let Some(app) = app_weak.upgrade() {
            let message = match status.as_str() {
                "Copied HTML source" => UiMessage::plain("Copied HTML source"),
                "Copied selected text" => UiMessage::plain("Copied selected text"),
                "Copied email text" => UiMessage::plain("Copied email text"),
                "Copied link" => UiMessage::plain("Copied link"),
                _ => {
                    tracing::warn!(key = %status, "ignored unknown copy-status message key");
                    return;
                }
            };
            app.set_render_status(message);
        }
    });

    let app_weak = app.as_weak();
    let renderer_for_pointer = Rc::clone(email_renderer);
    let use_wgpu_for_pointer = use_wgpu;
    app.on_email_pointer_event(move |x, y, kind, control, shift, alt, meta| {
        let selection_changed = renderer_for_pointer.borrow_mut().handle_pointer_event(
            x,
            y,
            kind.as_str(),
            InputModifiers::new(control, shift, alt, meta),
        );
        let selection_finished = kind.as_str() == "up" || kind.as_str() == "cancel";
        if !selection_changed && !selection_finished {
            return;
        }
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        if !use_wgpu_for_pointer {
            let (width, height) = email_viewport_size(&app);
            match renderer_for_pointer.borrow_mut().render_cpu_if_needed(
                width,
                height,
                app.window().scale_factor(),
            ) {
                Ok(Some(frame)) => apply_cpu_frame(&app, frame),
                Ok(None) => {}
                Err(error) => app.set_render_status(UiMessage::detail(
                    "Blitz software selection render failed: {}",
                    error,
                )),
            }
        }
        // Extracting the complete selected string on every drag sample is
        // linear in the selection size. The DOM highlight still repaints on
        // moves; clipboard state is published once the gesture finishes.
        if selection_finished || kind.as_str() == "down" {
            update_email_selection(&app, &renderer_for_pointer);
        }
        if selection_changed {
            app.window().request_redraw();
        }
    });

    let app_weak = app.as_weak();
    let renderer_for_scroll = Rc::clone(email_renderer);
    let use_wgpu_for_scroll = use_wgpu;
    app.on_email_scroll(move |scroll_y, viewport_height| {
        let needs_frame = renderer_for_scroll
            .borrow_mut()
            .set_visible_region(scroll_y, viewport_height);
        if !needs_frame {
            return;
        }
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        if !use_wgpu_for_scroll {
            let (width, height) = email_viewport_size(&app);
            match renderer_for_scroll.borrow_mut().render_cpu_if_needed(
                width,
                height,
                app.window().scale_factor(),
            ) {
                Ok(Some(frame)) => apply_cpu_frame(&app, frame),
                Ok(None) => {}
                Err(error) => app.set_render_status(UiMessage::detail(
                    "Blitz software tile render failed: {}",
                    error,
                )),
            }
        }
        app.window().request_redraw();
    });

    let app_weak = app.as_weak();
    let renderer_for_keyboard = Rc::clone(email_renderer);
    let use_wgpu_for_keyboard = use_wgpu;
    app.on_email_key_event(move |text, pressed, repeat, control, shift, alt, meta| {
        let copied = renderer_for_keyboard.borrow_mut().handle_key_event(
            text.as_str(),
            pressed,
            repeat,
            InputModifiers::new(control, shift, alt, meta),
        );
        let needs_repaint = renderer_for_keyboard.borrow().needs_repaint();
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        if !use_wgpu_for_keyboard && needs_repaint {
            let (width, height) = email_viewport_size(&app);
            match renderer_for_keyboard.borrow_mut().render_cpu_if_needed(
                width,
                height,
                app.window().scale_factor(),
            ) {
                Ok(Some(frame)) => apply_cpu_frame(&app, frame),
                Ok(None) => {}
                Err(error) => app.set_render_status(UiMessage::detail(
                    "Blitz software selection render failed: {}",
                    error,
                )),
            }
        }
        if needs_repaint {
            update_email_selection(&app, &renderer_for_keyboard);
        }
        if let Some(text) = copied {
            app.set_clipboard_request(text.into());
            app.set_clipboard_request_id(app.get_clipboard_request_id() + 1);
            app.set_render_status(UiMessage::plain("Copied selected text"));
        }
        if needs_repaint {
            app.window().request_redraw();
        }
    });

    let app_weak = app.as_weak();
    let renderer_for_select_all = Rc::clone(email_renderer);
    let use_wgpu_for_select_all = use_wgpu;
    app.on_select_email_text(move || {
        renderer_for_select_all.borrow_mut().select_all();
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        if !use_wgpu_for_select_all {
            let (width, height) = email_viewport_size(&app);
            match renderer_for_select_all.borrow_mut().render_cpu_if_needed(
                width,
                height,
                app.window().scale_factor(),
            ) {
                Ok(Some(frame)) => apply_cpu_frame(&app, frame),
                Ok(None) => {}
                Err(error) => app.set_render_status(UiMessage::detail(
                    "Blitz software selection render failed: {}",
                    error,
                )),
            }
        }
        update_email_selection(&app, &renderer_for_select_all);
        app.window().request_redraw();
    });
}
