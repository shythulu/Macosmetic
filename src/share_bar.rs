// SPDX-License-Identifier: GPL-3.0-only

//! The share row of a context menu: Open, Share, AirDrop and Reveal in Finder as icon
//! buttons, under the Open row. The list drops the entries the row carries.
//!
//! Share, AirDrop and Reveal in Finder exist only on macOS. Elsewhere the row would hold
//! Open alone, so it is left out and Open stays in the list.
//!
//! [`items`] decides which buttons show. It is a pure function so the rules can be tested
//! without a window. [`view`] draws them with the quick bar's row widget.

use std::collections::HashMap;

use cosmic::widget;
use cosmic::widget::menu::key_bind::KeyBind;

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
    /// The platform can share, AirDrop and reveal items in its file manager: macOS.
    pub sharing: bool,
}

impl Context {
    /// For the selection on this platform.
    pub fn new(selected: usize, selected_dirs: usize) -> Self {
        Self {
            selected,
            selected_dirs,
            sharing: cfg!(target_os = "macos"),
        }
    }
}

/// The actions the row can carry, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareAction {
    Open,
    Share,
    AirDrop,
    Reveal,
}

impl ShareAction {
    /// The [`Action`] the menu list runs for the same entry.
    pub const fn action(self) -> Action {
        match self {
            Self::Open => Action::Open,
            Self::Share => Action::Share,
            Self::AirDrop => Action::AirDrop,
            Self::Reveal => Action::RevealInFinder,
        }
    }

    const fn icon_name(self) -> &'static str {
        match self {
            Self::Open => "document-open-symbolic",
            Self::Share => "emblem-shared-symbolic",
            Self::AirDrop => "network-wireless-symbolic",
            Self::Reveal => "system-file-manager-symbolic",
        }
    }

    /// The tooltip name: the menu list's label without its ellipsis.
    pub fn label(self) -> String {
        let label = match self {
            Self::Open => fl!("open"),
            Self::Share => fl!("share"),
            Self::AirDrop => fl!("airdrop"),
            Self::Reveal => fl!("reveal-in-finder"),
        };
        label
            .trim_end_matches("...")
            .trim_end_matches('…')
            .to_string()
    }
}

/// Which buttons the row shows, left to right, for `context`. Open follows the list entry's
/// rule: one folder, or files only. The other three need only a selection.
pub fn items(context: Context) -> Vec<ShareAction> {
    if !context.sharing || context.selected == 0 {
        return Vec::new();
    }
    let mut items = Vec::with_capacity(4);
    if context.selected_dirs == 0 || (context.selected == 1 && context.selected_dirs == 1) {
        items.push(ShareAction::Open);
    }
    items.extend([
        ShareAction::Share,
        ShareAction::AirDrop,
        ShareAction::Reveal,
    ]);
    items
}

/// Whether `items` has a button for `action`, so the list below can drop its own entry.
pub fn carries(items: &[ShareAction], action: ShareAction) -> bool {
    items.contains(&action)
}

/// The row as a menu row. Each press runs the same message as the list entry would.
pub fn row(items: &[ShareAction], key_binds: &HashMap<KeyBind, Action>) -> quick_bar::Row {
    let buttons = items
        .iter()
        .map(|item| RowButton {
            icon: widget::icon::from_name(item.icon_name()).size(16).handle(),
            tooltip: quick_bar::tooltip(item.label(), item.action(), key_binds),
            on_press: Some(tab::Message::ContextAction(item.action())),
        })
        .collect();
    quick_bar::Row::new(buttons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(selected: usize, selected_dirs: usize) -> Context {
        Context {
            selected,
            selected_dirs,
            sharing: true,
        }
    }

    #[test]
    fn one_file_shows_all_four_in_order() {
        assert_eq!(
            items(mac(1, 0)),
            vec![
                ShareAction::Open,
                ShareAction::Share,
                ShareAction::AirDrop,
                ShareAction::Reveal,
            ]
        );
    }

    #[test]
    fn several_folders_cannot_open_but_can_share() {
        assert_eq!(
            items(mac(2, 2)),
            vec![
                ShareAction::Share,
                ShareAction::AirDrop,
                ShareAction::Reveal
            ]
        );
    }

    #[test]
    fn several_files_open_together() {
        assert_eq!(items(mac(3, 0))[0], ShareAction::Open);
    }

    #[test]
    fn nothing_selected_has_no_row() {
        assert!(items(mac(0, 0)).is_empty());
    }

    #[test]
    fn without_sharing_there_is_no_row_and_open_stays_in_the_list() {
        let linux = Context {
            sharing: false,
            ..mac(1, 0)
        };
        assert!(items(linux).is_empty());
    }

    #[test]
    fn labels_lose_the_ellipsis() {
        crate::localize::localize();
        assert_eq!(ShareAction::Share.label(), "Share");
        assert_eq!(ShareAction::Reveal.label(), "Reveal in Finder");
    }
}
