//! The right-click menu: callers hand it labels and what each one does. The
//! window places it where the click was.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ModelRc, VecModel};

use crate::ui::{self, AppWindow, MenuItem};

type Action = Box<dyn FnOnce(&AppWindow)>;

pub enum Entry {
    Item { label: String, enabled: bool, action: Action },
    Separator,
}

impl Entry {
    pub fn item(label: impl Into<String>, action: impl FnOnce(&AppWindow) + 'static) -> Self {
        Entry::Item {
            label: label.into(),
            enabled: true,
            action: Box::new(action),
        }
    }

    /// Shown greyed out.
    pub fn disabled(label: impl Into<String>) -> Self {
        Entry::Item {
            label: label.into(),
            enabled: false,
            action: Box::new(|_| {}),
        }
    }
}

thread_local! {
    static ACTIONS: RefCell<Vec<Option<Action>>> = const { RefCell::new(Vec::new()) };
}

pub fn init(window: &AppWindow) {
    window.on_menu_chosen(|index| {
        let action = ACTIONS.with_borrow_mut(|actions| actions.get_mut(index as usize).and_then(Option::take));
        let Some(window) = ui::window() else { return };
        window.set_menu_open(false);
        if let Some(action) = action {
            // May show another menu in the same place (a submenu).
            action(&window);
        }
    });
}

/// Shows the menu with these entries, where the last right-click was.
pub fn show(window: &AppWindow, entries: Vec<Entry>) {
    let mut items = Vec::new();
    let mut actions = Vec::new();
    for entry in entries {
        match entry {
            Entry::Item { label, enabled, action } => {
                items.push(MenuItem {
                    label: label.into(),
                    enabled,
                    separator: false,
                });
                actions.push(Some(action));
            }
            Entry::Separator => {
                items.push(MenuItem {
                    separator: true,
                    ..Default::default()
                });
                actions.push(None);
            }
        }
    }
    ACTIONS.with_borrow_mut(|a| *a = actions);
    window.set_menu_items(ModelRc::from(Rc::new(VecModel::from(items))));
    window.set_menu_open(true);
}

/// Closes the menu; true if it was open.
pub fn close(window: &AppWindow) -> bool {
    let open = window.get_menu_open();
    window.set_menu_open(false);
    open
}
