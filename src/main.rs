mod dhl;

use async_channel::Sender;
use dhl::{fetch_tracking_events, TrackingEvent};
use gtk4::glib::{self, clone};
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Button, Entry, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow};
use std::thread;

type LookupResult = Result<(String, Vec<TrackingEvent>), String>;

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
        .placeholder_text("DHL tracking number")
        .hexpand(true)
        .build();
    let button = Button::with_label("Check");
    input_row.append(&entry);
    input_row.append(&button);

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
    container.append(&scrolled);

    window.set_child(Some(&container));
    window.show();

    let (sender, receiver) = async_channel::unbounded::<LookupResult>();

    let sender_clone = sender.clone();
    button.connect_clicked(clone!(@weak entry, @weak status_label, @weak listbox, @weak button => move |_| {
        start_lookup(&entry, &status_label, &listbox, &button, &sender_clone);
    }));

    let sender_clone = sender.clone();
    entry.connect_activate(clone!(@weak entry, @weak status_label, @weak listbox, @weak button => move |_| {
        start_lookup(&entry, &status_label, &listbox, &button, &sender_clone);
    }));

    glib::MainContext::default().spawn_local(clone!(@weak status_label, @weak listbox, @weak button, @weak entry => async move {
        while let Ok(result) = receiver.recv().await {
            match result {
                Ok((code, events)) => {
                    status_label.set_text(&format!("{} events for {}", events.len(), code));
                    populate_events(&listbox, &events);
                }
                Err(err) => {
                    status_label.set_text(&format!("Error: {}", err));
                    populate_events(&listbox, &[]);
                }
            }

            button.set_sensitive(true);
            entry.set_sensitive(true);
        }
    }));
}

fn start_lookup(entry: &Entry, status_label: &Label, listbox: &ListBox, button: &Button, sender: &Sender<LookupResult>) {
    let code = entry.text().trim().to_string();

    if code.is_empty() {
        status_label.set_text("Please enter a tracking number.");
        return;
    }

    status_label.set_text("Fetching latest status...");
    button.set_sensitive(false);
    entry.set_sensitive(false);
    populate_events(listbox, &[]);

    let sender = sender.clone();
    thread::spawn(move || {
        let result = fetch_tracking_events(&code).map(|events| (code.clone(), events)).map_err(|e| e.to_string());
        let _ = sender.send_blocking(result);
    });
}

fn populate_events(listbox: &ListBox, events: &[TrackingEvent]) {
    while let Some(child) = listbox.first_child() {
        listbox.remove(&child);
    }

    if events.is_empty() {
        let row = ListBoxRow::new();
        let label = Label::builder()
            .label("No events to display.")
            .xalign(0.0)
            .wrap(true)
            .build();
        row.set_child(Some(&label));
        listbox.append(&row);
        return;
    }

    for event in events {
        listbox.append(&event_row(event));
    }
}

fn event_row(event: &TrackingEvent) -> ListBoxRow {
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

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
