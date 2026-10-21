// SPDX-License-Identifier: GPL-3.0-only

//! The Open row of a context menu: icon buttons for the ways to open a selection, under the
//! quick-action bar. It shortens the list: an action the row carries leaves the list.
//!
//! [`items`] decides which buttons show for a selection. It is a pure function so the rules
//! can be tested without a window. [`view`] draws them with the quick bar's row widget.

use std::collections::HashMap;

use cosmic::widget::menu::key_bind::KeyBind;
use cosmic::{Element, widget};

use crate::app::Action;
use crate::fl;
use crate::quick_bar::{self, RowButton};
use crate::tab;

/// What the row needs to know about the selection the menu opened on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Items selected.
    pub selected: usize,
    /// How many of those are folders.
    pub selected_dirs: usize,
    /// The tab is the file manager itself, not a file dialog or the desktop. Only there can
    /// a folder open in a new tab or window.
    pub app_mode: bool,
}

/// The actions the row can carry, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenAction {
    NewWindow,
    NewTab,
    Terminal,
    /// Pick an application to open the item with.
    With,
}

impl OpenAction {
    /// The [`Action`] the menu list runs for the same entry.
    pub const fn action(self) -> Action {
        match self {
            Self::NewWindow => Action::OpenInNewWindow,
            Self::NewTab => Action::OpenInNewTab,
            Self::Terminal => Action::OpenTerminal,
            Self::With => Action::OpenWith,
        }
    }

    /// Icon names, best first. COSMIC's set has no tab icon, so new tab falls back to a
    /// window stack.
    const fn icon_names(self) -> &'static [&'static str] {
        match self {
            Self::NewWindow => &["window-new-symbolic"],
            Self::NewTab => &["tab-new-symbolic", "window-stack-symbolic"],
            Self::Terminal => &["utilities-terminal-symbolic"],
            Self::With => &[
                "view-app-grid-symbolic",
                "application-x-executable-symbolic",
            ],
        }
    }

    /// The tooltip name: the menu list's label without its ellipsis.
    pub fn label(self) -> String {
        let label = match self {
            Self::NewWindow => fl!("open-in-new-window"),
            Self::NewTab => fl!("open-in-new-tab"),
            Self::Terminal => fl!("open-in-terminal"),
            Self::With => fl!("menu-open-with"),
        };
        label
            .trim_end_matches("...")
            .trim_end_matches('…')
            .to_string()
    }
}

/// Which buttons the row shows, left to right, for `context`. Mirrors the conditions of the
/// matching list entries: new tab and new window need a selection of folders only, in the
/// file manager; terminal needs exactly one folder; open with needs exactly one item.
pub fn items(context: Context) -> Vec<OpenAction> {
    let mut items = Vec::with_capacity(4);
    let only_folders = context.selected > 0 && context.selected == context.selected_dirs;
    if only_folders && context.app_mode {
        items.push(OpenAction::NewWindow);
        items.push(OpenAction::NewTab);
    }
    if context.selected == 1 && context.selected_dirs == 1 {
        items.push(OpenAction::Terminal);
    }
    if context.selected == 1 {
        items.push(OpenAction::With);
    }
    items
}

/// Whether `items` has a button for `action`, so the list below can drop its own entry.
pub fn carries(items: &[OpenAction], action: OpenAction) -> bool {
    items.contains(&action)
}

/// The row as a menu row. Each press runs the same message as the list entry would.
pub fn view(
    items: &[OpenAction],
    key_binds: &HashMap<KeyBind, Action>,
) -> Element<'static, tab::Message> {
    let buttons = items
        .iter()
        .map(|item| {
            let mut names = item.icon_names().iter();
            let first = names.next().copied().unwrap_or_default();
            let fallback: Vec<_> = names.map(|name| (*name).into()).collect();
            let handle = widget::icon::from_name(first)
                .size(16)
                .fallback(Some(widget::icon::IconFallback::Names(fallback)))
                .handle();
            RowButton {
                icon: handle,
                tooltip: quick_bar::tooltip(item.label(), item.action(), key_binds),
                on_press: Some(tab::Message::ContextAction(item.action())),
            }
        })
        .collect();
    quick_bar::row_view(buttons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(selected: usize, selected_dirs: usize) -> Context {
        Context {
            selected,
            selected_dirs,
            app_mode: true,
        }
    }

    #[test]
    fn one_folder_shows_every_way_to_open_in_order() {
        assert_eq!(
            items(context(1, 1)),
            vec![
                OpenAction::NewWindow,
                OpenAction::NewTab,
                OpenAction::Terminal,
                OpenAction::With,
            ]
        );
    }

    #[test]
    fn one_file_only_opens_with_an_application() {
        assert_eq!(items(context(1, 0)), vec![OpenAction::With]);
    }

    #[test]
    fn several_folders_open_in_new_windows_or_tabs() {
        assert_eq!(
            items(context(3, 3)),
            vec![OpenAction::NewWindow, OpenAction::NewTab]
        );
    }

    #[test]
    fn mixed_selection_has_no_row() {
        assert!(items(context(3, 1)).is_empty());
    }

    #[test]
    fn nothing_selected_has_no_row() {
        assert!(items(context(0, 0)).is_empty());
    }

    #[test]
    fn dialogs_and_desktop_cannot_open_tabs_or_windows() {
        let dialog = Context {
            app_mode: false,
            ..context(1, 1)
        };
        assert_eq!(items(dialog), vec![OpenAction::Terminal, OpenAction::With]);
    }

    #[test]
    fn carries_reports_present_actions() {
        let row = items(context(1, 0));
        assert!(carries(&row, OpenAction::With));
        assert!(!carries(&row, OpenAction::Terminal));
    }

    #[test]
    fn labels_lose_the_ellipsis() {
        crate::localize::localize();
        assert_eq!(OpenAction::With.label(), "Open with");
        assert_eq!(OpenAction::NewTab.label(), "Open in new tab");
    }
}
