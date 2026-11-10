//! The keymap: which key does what, in which context. The help overlay and
//! the key hints read the same table.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{Action, App};
use super::playlist::Modal;

/// A key and the modifiers held with it. Shift is not part of a character
/// key: `G` is `Char('G')` without modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl Key {
    const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    const fn char(c: char) -> Self {
        Self::plain(KeyCode::Char(c))
    }

    const fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    /// The key as a terminal event reports it, with Shift dropped from
    /// character keys, whose case already shows it.
    pub fn from_event(event: &KeyEvent) -> Self {
        let mut modifiers = event.modifiers;
        if matches!(event.code, KeyCode::Char(_)) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        Self {
            code: event.code,
            modifiers,
        }
    }

    /// The key's name in the help overlay and the hints.
    pub fn name(self) -> String {
        let code = match self.code {
            KeyCode::Char(' ') => "Space".to_owned(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Up => "↑".to_owned(),
            KeyCode::Down => "↓".to_owned(),
            KeyCode::Left => "←".to_owned(),
            KeyCode::Right => "→".to_owned(),
            KeyCode::Enter => "Enter".to_owned(),
            KeyCode::Backspace => "Backspace".to_owned(),
            KeyCode::Esc => "Esc".to_owned(),
            KeyCode::Tab => "Tab".to_owned(),
            KeyCode::Home => "Home".to_owned(),
            KeyCode::End => "End".to_owned(),
            other => other.to_string(),
        };
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            format!("Ctrl-{code}")
        } else {
            code
        }
    }
}

/// When a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// Moving through the sidebar and the main list.
    Browse,
    /// Typing a search query. Characters without a binding go into the
    /// query.
    Search,
    /// Typing a playlist's name, or filtering the playlist picker.
    /// Characters without a binding go into the text.
    Prompt,
    /// Answering a yes or no question.
    Confirm,
    /// The help overlay is open.
    Help,
}

impl Context {
    /// The contexts in the order the help overlay shows them.
    pub const ALL: [Self; 5] = [
        Self::Browse,
        Self::Search,
        Self::Prompt,
        Self::Confirm,
        Self::Help,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Browse => "BROWSE",
            Self::Search => "SEARCH",
            Self::Prompt => "PROMPT",
            Self::Confirm => "CONFIRM",
            Self::Help => "HELP",
        }
    }

    /// The context the state is in.
    pub fn of(app: &App) -> Self {
        if app.help {
            Self::Help
        } else if let Some(modal) = &app.modal {
            match modal {
                Modal::Name { .. } | Modal::Picker(_) => Self::Prompt,
                Modal::Delete(_) => Self::Confirm,
            }
        } else if app.editing_search().is_some() {
            Self::Search
        } else {
            Self::Browse
        }
    }
}

/// One entry of the keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub context: Context,
    pub keys: &'static [Key],
    pub action: Action,
    pub description: &'static str,
    /// The word the key hints use, for the bindings they show.
    pub hint: Option<&'static str>,
}

impl Binding {
    /// The binding's keys as the help overlay shows them.
    pub fn keys_label(&self) -> String {
        self.keys
            .iter()
            .map(|key| key.name())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

const fn bind(
    context: Context,
    keys: &'static [Key],
    action: Action,
    description: &'static str,
    hint: Option<&'static str>,
) -> Binding {
    Binding {
        context,
        keys,
        action,
        description,
        hint,
    }
}

/// Every key binding.
pub const KEYMAP: &[Binding] = &[
    bind(
        Context::Browse,
        &[Key::char('j'), Key::plain(KeyCode::Down)],
        Action::MoveDown,
        "Move down",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('k'), Key::plain(KeyCode::Up)],
        Action::MoveUp,
        "Move up",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('g'), Key::plain(KeyCode::Home)],
        Action::Top,
        "Go to the top",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('G'), Key::plain(KeyCode::End)],
        Action::Bottom,
        "Go to the bottom",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::ctrl('d')],
        Action::HalfPageDown,
        "Half a page down",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::ctrl('u')],
        Action::HalfPageUp,
        "Half a page up",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::plain(KeyCode::Tab)],
        Action::ToggleFocus,
        "Switch focus",
        Some("focus"),
    ),
    bind(
        Context::Browse,
        &[Key::char('h'), Key::plain(KeyCode::Left)],
        Action::FocusSidebar,
        "Focus the sidebar",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('l'), Key::plain(KeyCode::Right)],
        Action::FocusList,
        "Focus the list",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::plain(KeyCode::Enter)],
        Action::Open,
        "Open",
        Some("open"),
    ),
    bind(
        Context::Browse,
        &[Key::plain(KeyCode::Backspace), Key::plain(KeyCode::Esc)],
        Action::Back,
        "Back / clear search",
        Some("back"),
    ),
    bind(
        Context::Browse,
        &[Key::char('/')],
        Action::StartSearch,
        "Search",
        Some("search"),
    ),
    bind(
        Context::Browse,
        &[Key::char('a')],
        Action::AddToPlaylist,
        "Add to a playlist",
        Some("add"),
    ),
    bind(
        Context::Browse,
        &[Key::char('n')],
        Action::NewPlaylist,
        "New playlist",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('r')],
        Action::RenamePlaylist,
        "Rename the playlist",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('D')],
        Action::DeletePlaylist,
        "Delete the playlist",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('x')],
        Action::RemoveEntry,
        "Remove the entry",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('J')],
        Action::MoveEntryDown,
        "Move the entry down",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('K')],
        Action::MoveEntryUp,
        "Move the entry up",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('?')],
        Action::ToggleHelp,
        "Help",
        Some("help"),
    ),
    bind(
        Context::Browse,
        &[Key::char('q'), Key::ctrl('c')],
        Action::Quit,
        "Quit",
        Some("quit"),
    ),
    bind(
        Context::Search,
        &[Key::plain(KeyCode::Enter)],
        Action::ConfirmSearch,
        "Keep the results",
        Some("keep"),
    ),
    bind(
        Context::Search,
        &[Key::plain(KeyCode::Esc)],
        Action::ClearSearch,
        "Clear the search",
        Some("clear"),
    ),
    bind(
        Context::Search,
        &[Key::plain(KeyCode::Backspace)],
        Action::SearchDeleteChar,
        "Delete a character",
        None,
    ),
    bind(
        Context::Search,
        &[Key::plain(KeyCode::Down)],
        Action::MoveDown,
        "Move down",
        None,
    ),
    bind(
        Context::Search,
        &[Key::plain(KeyCode::Up)],
        Action::MoveUp,
        "Move up",
        None,
    ),
    bind(
        Context::Search,
        &[Key::ctrl('c')],
        Action::Quit,
        "Quit",
        None,
    ),
    bind(
        Context::Prompt,
        &[Key::plain(KeyCode::Enter)],
        Action::PromptConfirm,
        "Accept",
        Some("ok"),
    ),
    bind(
        Context::Prompt,
        &[Key::plain(KeyCode::Esc)],
        Action::PromptCancel,
        "Cancel",
        Some("cancel"),
    ),
    bind(
        Context::Prompt,
        &[Key::plain(KeyCode::Backspace)],
        Action::PromptDeleteChar,
        "Delete a character",
        None,
    ),
    bind(
        Context::Prompt,
        &[Key::plain(KeyCode::Down)],
        Action::MoveDown,
        "Move down the picker",
        None,
    ),
    bind(
        Context::Prompt,
        &[Key::plain(KeyCode::Up)],
        Action::MoveUp,
        "Move up the picker",
        None,
    ),
    bind(
        Context::Prompt,
        &[Key::ctrl('c')],
        Action::Quit,
        "Quit",
        None,
    ),
    bind(
        Context::Confirm,
        &[Key::char('y')],
        Action::PromptConfirm,
        "Yes",
        Some("yes"),
    ),
    bind(
        Context::Confirm,
        &[Key::char('n'), Key::plain(KeyCode::Esc)],
        Action::PromptCancel,
        "No",
        Some("no"),
    ),
    bind(
        Context::Confirm,
        &[Key::ctrl('c')],
        Action::Quit,
        "Quit",
        None,
    ),
    bind(
        Context::Help,
        &[Key::char('?'), Key::plain(KeyCode::Esc), Key::char('q')],
        Action::ToggleHelp,
        "Close help",
        Some("close"),
    ),
];

/// The action a key performs in `context`.
pub fn action_for_key(context: Context, key: Key) -> Option<Action> {
    let bound = KEYMAP
        .iter()
        .find(|binding| binding.context == context && binding.keys.contains(&key))
        .map(|binding| binding.action);
    match (bound, key.code, context) {
        (Some(action), _, _) => Some(action),
        (None, KeyCode::Char(c), Context::Search) if key.modifiers.is_empty() => {
            Some(Action::SearchInput(c))
        }
        (None, KeyCode::Char(c), Context::Prompt) if key.modifiers.is_empty() => {
            Some(Action::PromptInput(c))
        }
        _ => None,
    }
}
