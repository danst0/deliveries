mod dhl;

use async_channel::Sender;
use dhl::{fetch_tracking_events, TrackingEvent};
use gtk4::glib::{self, clone, ControlFlow};
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Button, Entry, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow};
use std::cell::RefCell;
use std::rc::Rc;
use std::thread;

#[derive(Clone, Debug)]
struct TrackingItem {
    code: String,
    events: Vec<TrackingEvent>,
    last_status: String,
}

#[derive(Debug)]
struct LookupMessage {
    code: String,
    result: Result<Vec<TrackingEvent>, String>,
}

type SharedState = Rc<RefCell<Vec<TrackingItem>>>;

fn main() {
    let app = Application::builder()
        .application_id("com.example.deliveries")
        .build();

    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("DHL Deliveries")
        .default_width(520)
        .default_height(480)
        .build();

    let container = gtk4::Box::new(Orientation::Vertical, 8);
    container.set_margin_top(12);
    container.set_margin_bottom(12);
    container.set_margin_start(12);
    container.set_margin_end(12);

    let input_row = gtk4::Box::new(Orientation::Horizontal, 6);
    let entry = Entry::builder()
        .placeholder_text("Add DHL tracking number")
        .hexpand(true)
        .build();
    let add_button = Button::with_label("Add");
    input_row.append(&entry);
    input_row.append(&add_button);

    let refresh_all = Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh all active tracking numbers")
        .build();

    let status_label = Label::builder()
        .label("Enter a DHL tracking number to start.")
        .wrap(true)
        .xalign(0.0)
        .build();

    let listbox = ListBox::new();
    listbox.set_vexpand(true);
    let scrolled = ScrolledWindow::builder()
        .child(&listbox)
        .vexpand(true)
        .build();

    container.append(&input_row);
    container.append(&status_label);
    container.append(&refresh_all);
    container.append(&scrolled);

    window.set_child(Some(&container));
    window.show();

    let (sender, receiver) = async_channel::unbounded::<LookupMessage>();
    let state: SharedState = Rc::new(RefCell::new(Vec::new()));

    let sender_clone = sender.clone();
    let state_clone = Rc::clone(&state);
    add_button.connect_clicked(clone!(@weak entry, @weak status_label, @weak listbox => move |_| {
        add_tracking_number(&entry, &status_label, &listbox, &sender_clone, &state_clone);
    }));

    let sender_clone = sender.clone();
    let state_clone = Rc::clone(&state);
    entry.connect_activate(clone!(@weak entry, @weak status_label, @weak listbox => move |_| {
        add_tracking_number(&entry, &status_label, &listbox, &sender_clone, &state_clone);
    }));

    let sender_clone = sender.clone();
    let state_clone = Rc::clone(&state);
    refresh_all.connect_clicked(clone!(@weak status_label => move |_| {
        let count = refresh_all_items(&state_clone, &sender_clone);
        status_label.set_text(&format!("Refreshing {} tracking numbers...", count));
    }));

    let state_for_task = Rc::clone(&state);
    let sender_for_task = sender.clone();
    glib::MainContext::default().spawn_local(clone!(@weak status_label, @weak listbox => async move {
        let state = state_for_task;
        let sender = sender_for_task;
        while let Ok(message) = receiver.recv().await {
            let result_text = apply_lookup_message(&state, message);
            status_label.set_text(&result_text);
            rebuild_list(&listbox, &state, &status_label, &sender);
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
            events: Vec::new(),
            last_status: "Pending first check...".to_string(),
        });
    }

    rebuild_list(listbox, state, status_label, sender);
    start_lookup_for_code(code, sender);
    status_label.set_text("Fetching latest status...");
}

fn refresh_all_items(state: &SharedState, sender: &Sender<LookupMessage>) -> usize {
    let codes: Vec<String> = state.borrow().iter().map(|it| it.code.clone()).collect();
    for code in &codes {
        start_lookup_for_code(code.clone(), sender);
    }
    codes.len()
}

fn start_lookup_for_code(code: String, sender: &Sender<LookupMessage>) {
    let sender = sender.clone();
    thread::spawn(move || {
        let result = fetch_tracking_events(&code).map_err(|e| e.to_string());
        let _ = sender.send_blocking(LookupMessage { code: code.clone(), result });
    });
}

fn apply_lookup_message(state: &SharedState, message: LookupMessage) -> String {
    let mut items = state.borrow_mut();
    if let Some(item) = items.iter_mut().find(|it| it.code == message.code) {
        match message.result {
            Ok(events) => {
                item.events = events;
                item.last_status = format!("{} events available", item.events.len());
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
    let code_label = Label::builder()
        .label(format!("{}", item.code))
        .xalign(0.0)
        .wrap(true)
        .build();
    code_label.add_css_class("title-4");

    let status = Label::builder()
        .label(item.last_status.clone())
        .xalign(0.0)
        .wrap(true)
        .build();
    status.add_css_class("dim-label");

    let header = gtk4::Box::new(Orientation::Horizontal, 6);
    header.set_hexpand(true);
    header.append(&code_label);

    let refresh_btn = Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh this tracking number now")
        .build();
    let code_clone = item.code.clone();
    let sender_clone = sender.clone();
    refresh_btn.connect_clicked(clone!(@weak status_label => move |_| {
        status_label.set_text(&format!("Refreshing {}...", code_clone));
        start_lookup_for_code(code_clone.clone(), &sender_clone);
    }));
    header.append(&refresh_btn);

    let archive_btn = Button::with_label("Archive");
    let code_clone = item.code.clone();
    let state_clone = Rc::clone(state);
    let sender_clone = sender.clone();
    archive_btn.connect_clicked(clone!(@weak listbox, @weak status_label => move |_| {
        archive_tracking(&code_clone, &state_clone);
        status_label.set_text(&format!("Archived {}", code_clone));
        rebuild_list(&listbox, &state_clone, &status_label, &sender_clone);
    }));
    header.append(&archive_btn);

    let body = gtk4::Box::new(Orientation::Vertical, 4);
    body.append(&status);

    if item.events.is_empty() {
        let placeholder = Label::builder()
            .label("No events yet.")
            .xalign(0.0)
            .wrap(true)
            .build();
        body.append(&placeholder);
    } else {
        for ev in &item.events {
            body.append(&event_row(ev));
        }
    }

    let row_box = gtk4::Box::new(Orientation::Vertical, 8);
    row_box.append(&header);
    row_box.append(&body);

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}

fn archive_tracking(code: &str, state: &SharedState) {
    let mut items = state.borrow_mut();
    if let Some(pos) = items.iter().position(|it| it.code == code) {
        items.remove(pos);
    }
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
