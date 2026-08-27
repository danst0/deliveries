mod dhl;
mod hermes;

use async_channel::Sender;
use dhl::{TrackingDetails, TrackingEvent};
use gtk4::glib::{self, clone, ControlFlow};
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Button, CallbackAction, Entry, Label, ListBox, ListBoxRow,
    Orientation, ScrolledWindow, Shortcut, ShortcutController, ShortcutTrigger,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;

fn get_data_file() -> PathBuf {
    let mut path = glib::user_data_dir();
    path.push("deliveries-tracker");
    if !path.exists() {
        let _ = fs::create_dir_all(&path);
    }
    path.push("deliveries.json");
    path
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrackingItem {
    code: String,
    #[serde(default)]
    name: Option<String>,
    /// Recipient postcode; DHL releases the uncensored detail view in exchange for it.
    #[serde(default)]
    zip: Option<String>,
    events: Vec<TrackingEvent>,
    last_status: String,
    #[serde(default)]
    plz_required: bool,
}

#[derive(Debug)]
struct LookupMessage {
    code: String,
    result: Result<TrackingDetails, String>,
}

type SharedState = Rc<RefCell<Vec<TrackingItem>>>;

fn main() {
    let app = Application::builder()
        .application_id("me.dumke.deliveries")
        .build();

    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Deliveries")
        .default_width(520)
        .default_height(480)
        .build();

    let controller = ShortcutController::new();
    let quit_shortcut = Shortcut::builder()
        .trigger(&ShortcutTrigger::parse_string("<Control>q").unwrap())
        .action(&CallbackAction::new(clone!(@weak app => @default-return false, move |_, _| {
            app.quit();
            true
        })))
        .build();
    let close_shortcut = Shortcut::builder()
        .trigger(&ShortcutTrigger::parse_string("<Control>w").unwrap())
        .action(&CallbackAction::new(clone!(@weak window => @default-return false, move |_, _| {
            window.close();
            true
        })))
        .build();
    controller.add_shortcut(quit_shortcut);
    controller.add_shortcut(close_shortcut);
    window.add_controller(controller);

    let container = gtk4::Box::new(Orientation::Vertical, 8);
    container.set_margin_top(12);
    container.set_margin_bottom(12);
    container.set_margin_start(12);
    container.set_margin_end(12);

    let input_row = gtk4::Box::new(Orientation::Horizontal, 6);
    let paste_button = Button::builder()
        .icon_name("edit-paste-symbolic")
        .tooltip_text("Paste from clipboard")
        .build();
    let entry = Entry::builder()
        .placeholder_text("Add tracking number")
        .hexpand(true)
        .build();

    paste_button.connect_clicked(clone!(@weak entry => move |_| {
        let clipboard = entry.clipboard();
        clipboard.read_text_async(None::<&gtk4::gio::Cancellable>, clone!(@weak entry => move |res| {
            if let Ok(Some(text)) = res {
                entry.set_text(text.as_str().trim());
            }
        }));
    }));

    let add_button = Button::with_label("Add");
    input_row.append(&paste_button);
    input_row.append(&entry);
    input_row.append(&add_button);

    let refresh_all = Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh all active tracking numbers")
        .build();
    input_row.append(&refresh_all);

    let status_label = Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .margin_top(6)
        .build();
    status_label.add_css_class("dim-label");

    let listbox = ListBox::new();
    listbox.set_vexpand(true);
    let scrolled = ScrolledWindow::builder()
        .child(&listbox)
        .vexpand(true)
        .build();

    container.append(&input_row);
    container.append(&scrolled);
    container.append(&status_label);

    window.set_child(Some(&container));
    window.present();

    let (sender, receiver) = async_channel::unbounded::<LookupMessage>();
    let state: SharedState = Rc::new(RefCell::new(load_from_file()));

    // Refresh all on startup
    {
        let count = refresh_all_items(&state, &sender);
        if count > 0 {
            status_label.set_text(&format!("Refreshing {} items...", count));
        }
    }
    rebuild_list(&listbox, &state, &status_label, &sender);

    let sender_for_add = sender.clone();
    let state_for_add = Rc::clone(&state);
    add_button.connect_clicked(clone!(@weak entry, @weak status_label, @weak listbox => move |_| {
        add_tracking_number(&entry, &status_label, &listbox, &sender_for_add, &state_for_add);
    }));

    let sender_for_entry = sender.clone();
    let state_for_entry = Rc::clone(&state);
    entry.connect_activate(clone!(@weak entry, @weak status_label, @weak listbox => move |_| {
        add_tracking_number(&entry, &status_label, &listbox, &sender_for_entry, &state_for_entry);
    }));

    let sender_for_refresh = sender.clone();
    let state_for_refresh = Rc::clone(&state);
    refresh_all.connect_clicked(clone!(@weak status_label => move |_| {
        let count = refresh_all_items(&state_for_refresh, &sender_for_refresh);
        if count > 0 {
            status_label.set_text(&format!("Refreshing {} items...", count));
        }
    }));

    let sender_row = sender.clone();
    listbox.connect_row_activated(clone!(@weak state, @weak listbox as lb, @weak status_label => move |_lb, row| {
        let index = row.index();
        if index >= 0 {
            let code = {
                let items = state.borrow();
                items.get(index as usize).map(|it| it.code.clone())
            };
            if let Some(code) = code {
                show_detail_window(&state, code, &lb, &status_label, &sender_row);
            }
        }
    }));

    let state_for_task = Rc::clone(&state);
    let sender_for_task = sender.clone();
    glib::MainContext::default().spawn_local(clone!(@weak status_label, @weak listbox => async move {
        let state = state_for_task;
        let sender = sender_for_task;
        while let Ok(message) = receiver.recv().await {
            let _result_text = apply_lookup_message(&state, message);
            rebuild_list(&listbox, &state, &status_label, &sender);
            if receiver.is_empty() {
                glib::timeout_add_seconds_local(5, clone!(@weak status_label => @default-return ControlFlow::Break, move || {
                    status_label.set_text("");
                    ControlFlow::Break
                }));
            }
        }
    }));

    let status_label_weak = status_label.downgrade();
    let state_timer = Rc::clone(&state);
    let sender_timer = sender.clone();
    glib::timeout_add_seconds_local(3600, move || {
        let count = refresh_all_items(&state_timer, &sender_timer);
        if count > 0 {
            if let Some(label) = status_label_weak.upgrade() {
                label.set_text(&format!("Automatic refresh for {} tracking numbers...", count));
            }
        }
        ControlFlow::Continue
    });
}

fn add_tracking_number(entry: &Entry, status_label: &Label, listbox: &ListBox, sender: &Sender<LookupMessage>, state: &SharedState) {
    let code = entry.text().trim().to_string();

    if code.is_empty() {
        status_label.set_text("Please enter a tracking number.");
        return;
    }

    {
        let items = state.borrow();
        if items.iter().any(|item| item.code == code) {
            status_label.set_text("Tracking number is already in the list.");
            return;
        }
    }

    entry.set_text("");
    {
        let mut items = state.borrow_mut();
        items.push(TrackingItem {
            code: code.clone(),
            name: None,
            zip: None,
            events: Vec::new(),
            last_status: "Pending first check...".to_string(),
            plz_required: false,
        });
    }

    save_to_file(state);
    rebuild_list(listbox, state, status_label, sender);
    start_lookup_for_code(code, None, sender);
    status_label.set_text("Fetching latest status...");
}

fn save_to_file(state: &SharedState) {
    let items = state.borrow();
    if let Ok(json) = serde_json::to_string_pretty(&*items) {
        let _ = fs::write(get_data_file(), json);
    }
}

fn load_from_file() -> Vec<TrackingItem> {
    if let Ok(data) = fs::read_to_string(get_data_file()) {
        if let Ok(items) = serde_json::from_str(&data) {
            return items;
        }
    }
    Vec::new()
}

fn refresh_all_items(state: &SharedState, sender: &Sender<LookupMessage>) -> usize {
    let items: Vec<(String, Option<String>)> = state
        .borrow()
        .iter()
        .map(|it| (it.code.clone(), it.zip.clone()))
        .collect();
    for (code, zip) in &items {
        start_lookup_for_code(code.clone(), zip.clone(), sender);
    }
    items.len()
}

fn start_lookup_for_code(code: String, zip: Option<String>, sender: &Sender<LookupMessage>) {
    let sender = sender.clone();
    thread::spawn(move || {
        let result = if code.starts_with('H') {
            hermes::fetch_tracking_events(&code).map(|events| TrackingDetails {
                events,
                ..Default::default()
            })
        } else {
            dhl::fetch_tracking_events(&code, zip.as_deref())
        }
        .map_err(|e| e.to_string());
        let _ = sender.send_blocking(LookupMessage { code, result });
    });
}

fn apply_lookup_message(state: &SharedState, message: LookupMessage) -> String {
    let res = {
        let mut items = state.borrow_mut();
        if let Some(item) = items.iter_mut().find(|it| it.code == message.code) {
            match message.result {
                Ok(mut details) => {
                    item.plz_required = details.censored || details.plz_required;
                    item.events = std::mem::take(&mut details.events);
                    item.last_status = summarize(item, &details);
                    format!("Updated {}", item.code)
                }
                Err(err) => {
                    item.events.clear();
                    item.last_status = format!("Error: {}", err);
                    format!("Error while updating {}", item.code)
                }
            }
        } else {
            "Received update for an unknown tracking number".to_string()
        }
    };
    save_to_file(state);
    res
}

/// Condense what DHL reported into the single line shown under the shipment name.
fn summarize(item: &TrackingItem, details: &TrackingDetails) -> String {
    let mut parts = Vec::new();

    if let Some(status) = details.short_status.as_deref() {
        parts.push(status.to_string());
    } else if details.delivered {
        parts.push("Delivered".to_string());
    }

    if let Some((step, total)) = details.progress {
        parts.push(format!("step {}/{}", step, total));
    }

    if let Some((from, to)) = details.delivery_window.as_ref() {
        parts.push(if from == to {
            format!("delivery {}", from)
        } else {
            format!("delivery {} - {}", from, to)
        });
    }

    if details.versanddatum_required && item.events.is_empty() {
        parts.push("shipping date required".to_string());
    } else if item.plz_required {
        parts.push(match item.zip.as_deref() {
            Some(_) => "postcode did not unlock more".to_string(),
            None => "enter recipient postcode for more".to_string(),
        });
    }

    if let Some(err) = details.zip_error.as_deref() {
        parts.push(format!("postcode rejected: {}", err));
    }

    if parts.is_empty() {
        return format!("{} events available", item.events.len());
    }

    format!("{} ({} events)", parts.join(" - "), item.events.len())
}

fn show_detail_window(state: &SharedState, code: String, listbox_main: &ListBox, status_label_main: &Label, sender: &Sender<LookupMessage>) {
    let item_opt = state.borrow().iter().find(|it| it.code == code).cloned();
    let Some(item) = item_opt else { return; };

    // A modal window without a transient parent blocks input on the main window while
    // the compositor is free to stack the modal behind it, which reads as a frozen app.
    let parent = listbox_main
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok());

    let window = gtk4::Window::builder()
        .title(format!("Details: {}", item.code))
        .default_width(400)
        .default_height(500)
        .modal(true)
        .destroy_with_parent(true)
        .build();

    if let Some(parent) = parent.as_ref() {
        window.set_transient_for(Some(parent));
    }

    let controller = ShortcutController::new();
    let close_shortcut = Shortcut::builder()
        .trigger(&ShortcutTrigger::parse_string("<Control>w").unwrap())
        .action(&CallbackAction::new(clone!(@weak window => @default-return false, move |_, _| {
            window.close();
            true
        })))
        .build();
    controller.add_shortcut(close_shortcut);
    window.add_controller(controller);

    let container = gtk4::Box::new(Orientation::Vertical, 12);
    container.set_margin_top(12);
    container.set_margin_bottom(12);
    container.set_margin_start(12);
    container.set_margin_end(12);

    let code_label = Label::builder()
        .label(&format!("Tracking Number: {}", item.code))
        .xalign(0.0)
        .build();
    code_label.add_css_class("title-4");

    let name_entry = Entry::builder()
        .placeholder_text("Custom Name")
        .text(item.name.as_deref().unwrap_or(""))
        .build();

    let zip_entry = Entry::builder()
        .placeholder_text("Recipient postcode - press Enter to look up")
        .text(item.zip.as_deref().unwrap_or(""))
        .max_length(5)
        .build();

    let zip_hint = Label::builder()
        .label(if item.plz_required {
            "DHL withholds details for this shipment until you enter the recipient postcode."
        } else {
            "DHL reports nothing withheld here, so a postcode adds nothing."
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    zip_hint.add_css_class("dim-label");
    zip_hint.add_css_class("caption");

    let history_label = Label::builder()
        .label("Tracking History:")
        .xalign(0.0)
        .margin_top(12)
        .build();
    history_label.add_css_class("title-4");

    let events_list = gtk4::Box::new(Orientation::Vertical, 6);
    if item.events.is_empty() {
        let placeholder = Label::builder()
            .label("No raw status events available yet.")
            .xalign(0.0)
            .build();
        events_list.append(&placeholder);
    } else {
        for ev in item.events.iter().rev() {
            events_list.append(&event_row(ev));
        }
    }

    let scrolled = ScrolledWindow::builder()
        .child(&events_list)
        .vexpand(true)
        .build();

    container.append(&code_label);
    container.append(&name_entry);
    container.append(&zip_entry);
    container.append(&zip_hint);
    container.append(&history_label);
    container.append(&scrolled);

    let code_for_zip = code.clone();
    zip_entry.connect_changed(clone!(@weak zip_entry, @weak state => move |_| {
        let raw = zip_entry.text().trim().to_string();
        let zip_opt = if raw.is_empty() { None } else { Some(raw) };

        {
            let mut items = state.borrow_mut();
            if let Some(it) = items.iter_mut().find(|it| it.code == code_for_zip) {
                it.zip = zip_opt;
            }
        }
        save_to_file(&state);
    }));

    let sender_zip = sender.clone();
    let code_for_lookup = code.clone();
    zip_entry.connect_activate(clone!(@weak status_label_main => move |entry| {
        let zip = entry.text().trim().to_string();
        if zip.is_empty() {
            status_label_main.set_text("Enter a postcode first.");
            return;
        }
        start_lookup_for_code(code_for_lookup.clone(), Some(zip), &sender_zip);
        status_label_main.set_text("Fetching details with postcode...");
    }));

    let sender_save = sender.clone();
    name_entry.connect_changed(clone!(@weak name_entry, @weak state, @weak listbox_main, @weak status_label_main => move |_| {
        let new_name = name_entry.text().trim().to_string();
        let name_opt = if new_name.is_empty() { None } else { Some(new_name) };
        
        {
            let mut items = state.borrow_mut();
            if let Some(it) = items.iter_mut().find(|it| it.code == code) {
                it.name = name_opt;
            }
        }
        save_to_file(&state);
        rebuild_list(&listbox_main, &state, &status_label_main, &sender_save);
    }));

    window.set_child(Some(&container));
    window.present();
}

fn rebuild_list(listbox: &ListBox, state: &SharedState, status_label: &Label, sender: &Sender<LookupMessage>) {
    while let Some(child) = listbox.first_child() {
        listbox.remove(&child);
    }

    let items = state.borrow();
    if items.is_empty() {
        let row = ListBoxRow::new();
        let label = Label::builder()
            .label("No tracking numbers yet. Add one above.")
            .xalign(0.0)
            .wrap(true)
            .build();
        row.set_child(Some(&label));
        listbox.append(&row);
        return;
    }

    for item in items.iter() {
        listbox.append(&tracking_row(item, state, status_label, sender, listbox));
    }
}

fn tracking_row(item: &TrackingItem, state: &SharedState, status_label: &Label, sender: &Sender<LookupMessage>, listbox: &ListBox) -> ListBoxRow {
    let display_name = item.name.as_deref().unwrap_or(&item.code);
    let name_label = Label::builder()
        .label(display_name)
        .xalign(0.0)
        .build();
    name_label.add_css_class("title-4");

    let latest_status = item.events.last()
        .map(|ev| ev.description.clone())
        .unwrap_or_else(|| item.last_status.clone());

    let status = Label::builder()
        .label(&latest_status)
        .xalign(0.0)
        .wrap(true)
        .build();
    status.add_css_class("dim-label");
    status.add_css_class("caption");

    let header = gtk4::Box::new(Orientation::Vertical, 2);
    header.set_hexpand(true);
    header.append(&name_label);
    header.append(&status);

    let archive_btn = Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Archive/Remove")
        .build();
    let code_clone = item.code.clone();
    let state_clone = Rc::clone(state);
    let sender_clone = sender.clone();
    archive_btn.connect_clicked(clone!(@weak listbox, @weak status_label => move |_| {
        archive_tracking(&code_clone, &state_clone);
        rebuild_list(&listbox, &state_clone, &status_label, &sender_clone);
    }));

    let actions = gtk4::Box::new(Orientation::Horizontal, 4);
    actions.set_valign(gtk4::Align::Center);
    actions.append(&archive_btn);

    let row_box = gtk4::Box::new(Orientation::Horizontal, 8);
    row_box.set_margin_top(4);
    row_box.set_margin_bottom(4);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);
    row_box.append(&header);
    row_box.append(&actions);

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}

fn archive_tracking(code: &str, state: &SharedState) {
    let mut items = state.borrow_mut();
    if let Some(pos) = items.iter().position(|it| it.code == code) {
        items.remove(pos);
    }
    drop(items);
    save_to_file(state);
}

fn event_row(event: &TrackingEvent) -> gtk4::Box {
    let meta = Label::builder()
        .label(format!("{} — {}", event.timestamp, event.location))
        .xalign(0.0)
        .wrap(true)
        .build();
    meta.add_css_class("dim-label");

    let desc = Label::builder()
        .label(&event.description)
        .xalign(0.0)
        .wrap(true)
        .build();

    let row_box = gtk4::Box::new(Orientation::Vertical, 4);
    row_box.append(&meta);
    row_box.append(&desc);
    row_box
}
