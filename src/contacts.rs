//! Contact directory view-model and Slint model projection.

use crate::{AppWindow, ContactAccountRow, ContactRow, I18n, paged_visible_count};
use chrono::{Datelike, Local, TimeZone};
use flectar_mail_core::models::{ContactRecord, ContactRecordCursor, ContactRecordPage};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub(crate) struct ContactDirectoryState {
    pub(crate) contacts: Vec<ContactRecord>,
    pub(crate) rows: Rc<VecModel<ContactRow>>,
    pub(crate) selected_id: Option<i64>,
    pub(crate) query: String,
    pub(crate) scope: String,
    pub(crate) page: usize,
    pub(crate) next_cursor: Option<ContactRecordCursor>,
    pub(crate) matching_count: usize,
    pub(crate) total_count: usize,
    pub(crate) favorite_count: usize,
    pub(crate) account_counts: HashMap<i64, usize>,
    pub(crate) editing_new: bool,
    pub(crate) using_core: bool,
}

impl ContactDirectoryState {
    pub(crate) fn new(contacts: Vec<ContactRecord>, using_core: bool) -> Self {
        Self {
            selected_id: contacts.first().map(|contact| contact.id),
            contacts,
            rows: Rc::new(VecModel::default()),
            query: String::new(),
            scope: "All contacts".to_owned(),
            page: 1,
            next_cursor: None,
            matching_count: 0,
            total_count: 0,
            favorite_count: 0,
            account_counts: HashMap::new(),
            editing_new: false,
            using_core,
        }
    }

    pub(crate) fn begin_core_query(&mut self) {
        self.contacts.clear();
        self.selected_id = None;
        self.page = 1;
        self.next_cursor = None;
        self.matching_count = 0;
        self.editing_new = false;
        self.using_core = true;
    }

    pub(crate) fn apply_core_page(
        &mut self,
        cursor: Option<&ContactRecordCursor>,
        page: ContactRecordPage,
    ) -> bool {
        if cursor.is_some() && self.next_cursor.as_ref() != cursor {
            return false;
        }
        if cursor.is_none() {
            self.contacts = page.records;
        } else {
            let mut known = self
                .contacts
                .iter()
                .map(|contact| contact.id)
                .collect::<std::collections::HashSet<_>>();
            self.contacts.extend(
                page.records
                    .into_iter()
                    .filter(|contact| known.insert(contact.id)),
            );
        }
        self.next_cursor = page.next_cursor;
        self.matching_count = page.matching_count;
        self.total_count = page.total_count;
        self.favorite_count = page.favorite_count;
        self.account_counts = page.account_counts.into_iter().collect();
        if self.selected_id.is_none() {
            self.selected_id = self.contacts.first().map(|contact| contact.id);
        }
        true
    }
}

fn contact_avatar_tone(contact: &ContactRecord) -> i32 {
    // FNV-1a gives each contact a stable, inexpensive palette slot. Using the
    // normalized email keeps the fallback consistent when a display name is
    // edited and avoids a visibly changing "random" color between launches.
    let mut hash = 0x811c_9dc5_u32;
    for byte in contact.email.trim().to_lowercase().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    (hash % 6) as i32
}

fn record_initials(contact: &ContactRecord) -> String {
    let source = if contact.name.trim().is_empty() {
        contact.email.as_str()
    } else {
        contact.name.as_str()
    };
    let mut initials = source
        .split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>();
    if initials.is_empty() {
        initials.push('@');
    }
    initials.to_uppercase()
}

fn contact_last_interacted(app: &AppWindow, value: Option<i64>) -> String {
    let Some(value) = value else {
        return app.global::<I18n>().invoke_no_contact_history().into();
    };
    Local
        .timestamp_millis_opt(value)
        .single()
        .map(|time| {
            app.global::<I18n>()
                .invoke_last_contacted(time.month() as i32, time.day() as i32, time.year())
                .into()
        })
        .unwrap_or_else(|| {
            app.global::<I18n>()
                .invoke_contact_history_available()
                .into()
        })
}

fn contact_matches(contact: &ContactRecord, query: &str) -> bool {
    let haystack = format!(
        "{} {} {} {} {} {} {} {}",
        contact.name,
        contact.email,
        contact.phone,
        contact.company,
        contact.job_title,
        contact.website,
        contact.tags,
        contact.postal_address,
    )
    .to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|token| haystack.contains(token))
}

fn scope_account_id(scope: &str) -> Option<i64> {
    scope.strip_prefix("Account:")?.parse().ok()
}

fn contact_belongs_to_account(contact: &ContactRecord, account_id: i64) -> bool {
    contact.is_managed || contact.account_ids.contains(&account_id)
}

fn visible_contact_ids(directory: &ContactDirectoryState) -> Vec<i64> {
    if directory.using_core {
        return directory
            .contacts
            .iter()
            .map(|contact| contact.id)
            .collect();
    }
    directory
        .contacts
        .iter()
        .filter(|contact| {
            if directory.scope == "Favorites" {
                contact.is_favorite
            } else if let Some(account_id) = scope_account_id(&directory.scope) {
                contact_belongs_to_account(contact, account_id)
            } else {
                true
            }
        })
        .filter(|contact| contact_matches(contact, &directory.query))
        .map(|contact| contact.id)
        .collect()
}

pub(crate) fn apply_contact_rows(app: &AppWindow, state: &Rc<RefCell<ContactDirectoryState>>) {
    let directory = state.borrow();
    let visible = visible_contact_ids(&directory);
    let shown_count = if directory.using_core {
        visible.len()
    } else {
        paged_visible_count(directory.page, visible.len())
    };
    let rows = visible
        .iter()
        .take(shown_count)
        .filter_map(|id| directory.contacts.iter().find(|contact| contact.id == *id))
        .filter_map(|contact| {
            Some(ContactRow {
                id: i32::try_from(contact.id).ok()?,
                name: contact.name.clone().into(),
                email: contact.email.clone().into(),
                initials: record_initials(contact).into(),
                avatar_tone: contact_avatar_tone(contact),
                company: contact.company.clone().into(),
                detail: match (contact.company.is_empty(), contact.job_title.is_empty()) {
                    (false, false) => format!("{} · {}", contact.company, contact.job_title),
                    (false, true) => contact.company.clone(),
                    (true, false) => contact.job_title.clone(),
                    (true, true) => String::new(),
                }
                .into(),
                last_contacted: contact_last_interacted(app, contact.last_interacted).into(),
                favorite: contact.is_favorite,
                selected: directory.selected_id == Some(contact.id) && !directory.editing_new,
            })
        })
        .collect::<Vec<_>>();
    crate::reconcile_model_rows(&directory.rows, rows, |row| row.id);
    let connected_accounts = app.get_connected_accounts();
    let mut selected_scope_label = directory.scope.clone();
    let account_rows = (0..connected_accounts.row_count())
        .filter_map(|index| connected_accounts.row_data(index))
        .map(|account| {
            let selected = directory.scope == format!("Account:{}", account.id);
            if selected {
                selected_scope_label = if account.name.is_empty() {
                    account.email.to_string()
                } else {
                    account.name.to_string()
                };
            }
            let count = if directory.using_core {
                directory
                    .account_counts
                    .get(&i64::from(account.id))
                    .copied()
                    .unwrap_or_default()
            } else {
                directory
                    .contacts
                    .iter()
                    .filter(|contact| contact_belongs_to_account(contact, i64::from(account.id)))
                    .count()
            };
            ContactAccountRow {
                id: account.id,
                name: account.name,
                email: account.email,
                initials: account.initials,
                avatar: account.avatar_small,
                has_avatar: account.has_avatar,
                count: if count == 0 {
                    "".into()
                } else {
                    count.to_string().into()
                },
                selected,
            }
        })
        .collect::<Vec<_>>();
    app.set_contact_accounts(ModelRc::new(VecModel::from(account_rows)));
    app.set_contact_scope(directory.scope.clone().into());
    app.set_contact_scope_label(selected_scope_label.into());
    app.set_contact_search_query(directory.query.clone().into());
    let total_count = if directory.using_core {
        directory.total_count
    } else {
        directory.contacts.len()
    };
    let favorite_count = if directory.using_core {
        directory.favorite_count
    } else {
        directory
            .contacts
            .iter()
            .filter(|contact| contact.is_favorite)
            .count()
    };
    let visible_count = if directory.using_core {
        directory.matching_count
    } else {
        visible.len()
    };
    app.set_contact_total_count(total_count.min(i32::MAX as usize) as i32);
    app.set_contact_favorite_count(favorite_count.min(i32::MAX as usize) as i32);
    app.set_contact_visible_count(visible_count.min(i32::MAX as usize) as i32);
    app.set_contact_shown_count(shown_count as i32);
    app.set_contact_can_load_more(if directory.using_core {
        directory.next_cursor.is_some()
    } else {
        shown_count < visible.len()
    });
}

pub(crate) fn clear_contact_form(app: &AppWindow) {
    app.set_contact_id(0);
    app.set_contact_name("".into());
    app.set_contact_email("".into());
    app.set_contact_phone("".into());
    app.set_contact_company("".into());
    app.set_contact_job_title("".into());
    app.set_contact_website("".into());
    app.set_contact_birthday("".into());
    app.set_contact_address("".into());
    app.set_contact_notes("".into());
    app.set_contact_tags("".into());
    app.set_contact_favorite(false);
    app.set_contact_initials("?".into());
    app.set_contact_interactions(app.global::<I18n>().invoke_new_contact_entry());
    app.set_contact_last_interacted("".into());
}

fn apply_contact_form(app: &AppWindow, contact: &ContactRecord) {
    app.set_contact_id(i32::try_from(contact.id).unwrap_or(i32::MAX));
    app.set_contact_name(contact.name.clone().into());
    app.set_contact_email(contact.email.clone().into());
    app.set_contact_phone(contact.phone.clone().into());
    app.set_contact_company(contact.company.clone().into());
    app.set_contact_job_title(contact.job_title.clone().into());
    app.set_contact_website(contact.website.clone().into());
    app.set_contact_birthday(contact.birthday.clone().into());
    app.set_contact_address(contact.postal_address.clone().into());
    app.set_contact_notes(contact.notes.clone().into());
    app.set_contact_tags(contact.tags.clone().into());
    app.set_contact_favorite(contact.is_favorite);
    app.set_contact_initials(record_initials(contact).into());
    app.set_contact_interactions(
        app.global::<I18n>()
            .invoke_interaction_count(contact.interactions as i32),
    );
    app.set_contact_last_interacted(contact_last_interacted(app, contact.last_interacted).into());
    app.set_contact_has_selection(true);
}

pub(crate) fn apply_contact_directory(app: &AppWindow, state: &Rc<RefCell<ContactDirectoryState>>) {
    {
        let mut directory = state.borrow_mut();
        if !directory.editing_new {
            let visible = visible_contact_ids(&directory);
            if !directory
                .selected_id
                .is_some_and(|selected| visible.contains(&selected))
            {
                directory.selected_id = visible.first().copied();
            }
        }
    }
    apply_contact_rows(app, state);
    let directory = state.borrow();
    if directory.editing_new {
        app.set_contact_has_selection(true);
        return;
    }
    if let Some(contact) = directory
        .selected_id
        .and_then(|id| directory.contacts.iter().find(|contact| contact.id == id))
    {
        apply_contact_form(app, contact);
    } else {
        clear_contact_form(app);
        app.set_contact_has_selection(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(email: &str) -> ContactRecord {
        ContactRecord {
            id: 1,
            name: "Example Person".to_owned(),
            email: email.to_owned(),
            phone: String::new(),
            company: String::new(),
            job_title: String::new(),
            website: String::new(),
            birthday: String::new(),
            postal_address: String::new(),
            notes: String::new(),
            tags: String::new(),
            is_favorite: false,
            interactions: 0,
            last_interacted: None,
            account_ids: Vec::new(),
            is_managed: false,
        }
    }

    fn cursor(id: i64) -> ContactRecordCursor {
        ContactRecordCursor {
            is_favorite: false,
            sort_name: format!("Person {id}"),
            email: format!("person-{id}@example.com"),
            id,
        }
    }

    fn page(start: i64, count: i64, next_cursor: Option<ContactRecordCursor>) -> ContactRecordPage {
        ContactRecordPage {
            records: (start..start + count)
                .map(|id| {
                    let mut record = contact(&format!("person-{id}@example.com"));
                    record.id = id;
                    record
                })
                .collect(),
            next_cursor,
            matching_count: 61,
            total_count: 61,
            favorite_count: 0,
            account_counts: vec![(1, 61)],
        }
    }

    #[test]
    fn avatar_tone_is_stable_for_normalized_email() {
        let lower = contact("person@example.com");
        let mixed = contact("  Person@Example.COM  ");
        assert_eq!(contact_avatar_tone(&lower), contact_avatar_tone(&mixed));
        assert!((0..6).contains(&contact_avatar_tone(&lower)));
    }

    #[test]
    fn core_pages_append_only_at_the_expected_cursor() {
        let mut directory = ContactDirectoryState::new(Vec::new(), true);
        directory.begin_core_query();
        assert!(directory.apply_core_page(None, page(1, 25, Some(cursor(25)))));
        let first_cursor = directory.next_cursor.clone().unwrap();
        assert!(directory.apply_core_page(
            Some(&first_cursor),
            page(26, 25, Some(cursor(50)))
        ));
        assert_eq!(directory.contacts.len(), 50);
        assert_eq!(directory.next_cursor, Some(cursor(50)));
        assert!(!directory.apply_core_page(
            Some(&first_cursor),
            page(26, 25, Some(cursor(50)))
        ));
        assert_eq!(directory.contacts.len(), 50);
    }
}
