//! Slint projection for mailbox fixtures and rendered message frames.

use super::*;

#[cfg(test)]
pub(super) fn fixture_mailboxes(messages: &[MailMessage]) -> Vec<MailboxEntry> {
    const STANDARD_FOLDERS: [&str; 7] = [
        "Inbox", "Starred", "Sent", "Archive", "Spam", "Trash", "Drafts",
    ];

    let mut accounts = Vec::new();
    for message in messages {
        if !accounts.contains(&message.account) {
            accounts.push(message.account.clone());
        }
    }

    let mut entries = Vec::new();
    for account in accounts {
        entries.push(MailboxEntry {
            account_id: 0,
            folder_id: -1,
            parent_folder_id: -1,
            depth: 0,
            has_children: false,
            is_standard: false,
            label: account.clone(),
            scope: account.clone(),
            context: account.clone(),
            detail: "Account folders".to_owned(),
            avatar: avatar_initials(&account),
            is_account: true,
            count: messages
                .iter()
                .filter(|message| message.account == account)
                .count()
                .to_string(),
        });

        for label in STANDARD_FOLDERS {
            entries.push(MailboxEntry {
                account_id: 0,
                folder_id: -1,
                parent_folder_id: -1,
                depth: 0,
                has_children: false,
                is_standard: true,
                label: label.to_owned(),
                scope: format!("{account} / {label}"),
                context: account.clone(),
                detail: String::new(),
                avatar: String::new(),
                is_account: false,
                count: messages
                    .iter()
                    .filter(|message| {
                        message.account == account
                            && if label == "Starred" {
                                message.starred
                            } else {
                                message.folder == label
                            }
                    })
                    .count()
                    .to_string(),
            });
        }

        let mut custom_folders = Vec::new();
        for message in messages.iter().filter(|message| message.account == account) {
            if !STANDARD_FOLDERS.contains(&message.folder.as_str())
                && !custom_folders.contains(&message.folder)
            {
                custom_folders.push(message.folder.clone());
            }
        }
        custom_folders.sort_by_key(|folder| folder.to_lowercase());
        for folder in custom_folders {
            entries.push(MailboxEntry {
                account_id: 0,
                folder_id: -1,
                parent_folder_id: -1,
                depth: 0,
                has_children: false,
                is_standard: false,
                label: folder.clone(),
                scope: format!("{account} / {folder}"),
                context: account.clone(),
                detail: String::new(),
                avatar: String::new(),
                is_account: false,
                count: messages
                    .iter()
                    .filter(|message| message.account == account && message.folder == folder)
                    .count()
                    .to_string(),
            });
        }
    }
    entries
}

pub(super) fn avatar_initials(label: &str) -> String {
    let mut words = label
        .split_whitespace()
        .filter_map(|word| word.chars().next());
    let Some(first) = words.next() else {
        return "?".to_owned();
    };
    let second = words.next();
    second.map_or_else(
        || first.to_uppercase().collect(),
        |second| format!("{}{}", first.to_uppercase(), second.to_uppercase()),
    )
}

pub(super) fn update_email_selection(
    app: &AppWindow,
    email_renderer: &Rc<RefCell<GpuEmailRenderer>>,
) {
    let renderer = email_renderer.borrow();
    let selected_text = renderer.selected_text().unwrap_or_default();
    app.set_selected_text(selected_text.into());
    app.set_has_selection(renderer.has_selection());
}

pub(super) fn email_viewport_size(app: &AppWindow) -> (u32, u32) {
    let width = app.get_email_viewport_width().max(1.0).ceil() as u32;
    let height = app.get_email_viewport_height().max(1.0).ceil() as u32;
    // Before Slint's first layout pass these output properties can still be
    // zero. Use the fallback canvas only for that initial frame.
    if width <= 1 || height <= 1 {
        (520, 900)
    } else {
        (width, height)
    }
}

pub(super) fn apply_email(
    app: &AppWindow,
    email: MailMessage,
    email_renderer: &Rc<RefCell<GpuEmailRenderer>>,
    use_wgpu: bool,
    allow_remote_images: bool,
) -> Result<(), String> {
    app.set_selected_sender(email.sender.clone().into());
    app.set_selected_address(email.address.clone().into());
    app.set_selected_subject(email.subject.clone().into());
    app.set_selected_time(email.time.clone().into());
    app.set_selected_to(email.to.clone().into());
    app.set_selected_label(email.label.clone().into());
    app.set_selected_is_draft(email.folder == "Drafts" || email.label == "DRAFT");
    app.set_selected_initials(email.initials.clone().into());
    app.set_selected_starred(email.starred);
    app.set_selected_unread(email.unread);
    app.set_selected_sender_verification(email.sender_verification.clone().into());
    app.set_email_scroll_y(0.0);

    let preview = display_preview(&email.preview);
    let fallback_html = format!(
        "<html><body style=\"font-family:Arial,sans-serif;padding:32px;line-height:1.6;color:#303348\"><p>{}</p></body></html>",
        preview
    );
    let html = email.html.as_deref().unwrap_or(&fallback_html);
    app.set_selected_source(html.into());
    app.set_remote_images_blocked(renderer::has_remote_images(html) && !allow_remote_images);
    let mut prepared = email_renderer
        .borrow()
        .prepare_email_html(html, allow_remote_images)?;
    // Slint owns the plain-text accessibility/copy view after this point.
    // Do not retain a second full copy inside the Blitz document wrapper.
    let plain_text = std::mem::take(&mut prepared.plain_text);

    if !use_wgpu {
        // Release the previous tile model before allocating the first
        // viewport-sized tile set for the replacement message. Rendering is
        // synchronous here, so no intermediate frame is exposed.
        app.set_email_tiles(ModelRc::new(VecModel::default()));
        let mut renderer = email_renderer.borrow_mut();
        renderer.set_email(prepared);
        let (width, height) = email_viewport_size(app);
        let rendered = renderer
            .render_cpu_if_needed(width, height, app.window().scale_factor())?
            .ok_or_else(|| "software email renderer did not produce a frame".to_owned())?;
        drop(renderer);
        app.set_selected_plain_text(plain_text.into());
        app.set_selected_text("".into());
        app.set_has_selection(false);
        apply_cpu_frame(app, rendered);
        // The old image and Vello scratch storage are both gone now. Return
        // any unused pages left after the new retained frame to the OS.
        renderer::trim_unused_heap();
        return Ok(());
    }

    let links = prepared.links.clone();
    email_renderer.borrow_mut().set_email(prepared);
    app.set_email_tiles(ModelRc::new(VecModel::default()));
    app.set_email_links(ModelRc::new(VecModel::from(
        links.into_iter().map(slint_email_link).collect::<Vec<_>>(),
    )));
    app.set_selected_plain_text(plain_text.into());
    app.set_selected_text("".into());
    app.set_has_selection(false);
    app.set_render_status(UiMessage::plain("Message ready."));
    Ok(())
}

pub(super) fn slint_email_link(link: renderer::EmailLink) -> EmailLink {
    EmailLink {
        x: link.x,
        y: link.y,
        width: link.width,
        height: link.height,
        url: link.url.into(),
    }
}

pub(super) fn slint_email_tile(tile: renderer::RenderedEmailTile) -> EmailTile {
    EmailTile {
        image: tile.image,
        y: tile.y,
        height: tile.height,
    }
}

#[cfg(feature = "gpu-renderer")]
pub(super) fn apply_gpu_frame(app: &AppWindow, rendered: RenderedEmail) {
    app.set_email_content_aspect(rendered.height as f32 / rendered.width.max(1) as f32);
    app.set_email_tiles(ModelRc::new(VecModel::from(
        rendered
            .tiles
            .into_iter()
            .map(slint_email_tile)
            .collect::<Vec<_>>(),
    )));
    app.set_email_links(ModelRc::new(VecModel::from(
        rendered
            .links
            .into_iter()
            .map(slint_email_link)
            .collect::<Vec<_>>(),
    )));
    app.set_render_status(UiMessage::plain("Message ready."));
}

pub(super) fn apply_cpu_frame(app: &AppWindow, rendered: RenderedEmail) {
    app.set_email_content_aspect(rendered.height as f32 / rendered.width.max(1) as f32);
    app.set_email_tiles(ModelRc::new(VecModel::from(
        rendered
            .tiles
            .into_iter()
            .map(slint_email_tile)
            .collect::<Vec<_>>(),
    )));
    app.set_email_links(ModelRc::new(VecModel::from(
        rendered
            .links
            .into_iter()
            .map(slint_email_link)
            .collect::<Vec<_>>(),
    )));
    app.set_render_status(UiMessage::plain("Message ready."));
}

pub(super) fn open_email_link(url: &str) -> Result<(), String> {
    let Some((scheme, _)) = url.trim().split_once(':') else {
        return Err("only absolute links can be opened".into());
    };
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" | "mailto" | "tel" => {
            webbrowser::open(url.trim()).map_err(|error| error.to_string())?;
            Ok(())
        }
        _ => Err("unsupported link type".into()),
    }
}
