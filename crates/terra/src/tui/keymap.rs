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
    /// Controlling the player: these bindings apply in [`Browse`](Self::Browse)
    /// and [`Queue`](Self::Queue) too, where those have none for the key.
    Playback,
    /// The queue view is open.
    Queue,
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
    pub const ALL: [Self; 7] = [
        Self::Browse,
        Self::Playback,
        Self::Queue,
        Self::Search,
        Self::Prompt,
        Self::Confirm,
        Self::Help,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Browse => "BROWSE",
            Self::Playback => "PLAYBACK",
            Self::Queue => "QUEUE",
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
        } else if app.queue_view.is_some() {
            Self::Queue
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
        "Open / play",
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
        &[Key::char('e')],
        Action::QueueTrack,
        "Add to the queue",
        None,
    ),
    bind(
        Context::Browse,
        &[Key::char('E')],
        Action::PlayNext,
        "Play next",
        None,
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
        Context::Playback,
        &[Key::char(' ')],
        Action::TogglePause,
        "Pause / resume",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('>')],
        Action::NextTrack,
        "Next track",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('<')],
        Action::PreviousTrack,
        "Previous track",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char(']')],
        Action::Seek {
            forward: true,
            seconds: 5,
        },
        "Forward 5 s",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('[')],
        Action::Seek {
            forward: false,
            seconds: 5,
        },
        "Back 5 s",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('}')],
        Action::Seek {
            forward: true,
            seconds: 30,
        },
        "Forward 30 s",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('{')],
        Action::Seek {
            forward: false,
            seconds: 30,
        },
        "Back 30 s",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('+')],
        Action::VolumeUp,
        "Volume up",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('-')],
        Action::VolumeDown,
        "Volume down",
        None,
    ),
    bind(
        Context::Playback,
        &[Key::char('Q')],
        Action::ToggleQueue,
        "Show / hide the queue",
        None,
    ),
    bind(
        Context::Queue,
        &[Key::char('j'), Key::plain(KeyCode::Down)],
        Action::MoveDown,
        "Move down",
        None,
    ),
    bind(
        Context::Queue,
        &[Key::char('k'), Key::plain(KeyCode::Up)],
        Action::MoveUp,
        "Move up",
        None,
    ),
    bind(
        Context::Queue,
        &[Key::char('g'), Key::plain(KeyCode::Home)],
        Action::Top,
        "Go to the top",
        None,
    ),
    bind(
        Context::Queue,
        &[Key::char('G'), Key::plain(KeyCode::End)],
        Action::Bottom,
        "Go to the bottom",
        None,
    ),
    bind(
        Context::Queue,
        &[Key::char('x')],
        Action::RemoveEntry,
        "Remove from the queue",
        Some("remove"),
    ),
    bind(
        Context::Queue,
        &[Key::char('J')],
        Action::MoveEntryDown,
        "Move the item down",
        Some("down"),
    ),
    bind(
        Context::Queue,
        &[Key::char('K')],
        Action::MoveEntryUp,
        "Move the item up",
        Some("up"),
    ),
    bind(
        Context::Queue,
        &[Key::plain(KeyCode::Esc)],
        Action::ToggleQueue,
        "Close the queue",
        Some("close"),
    ),
    bind(
        Context::Queue,
        &[Key::ctrl('c')],
        Action::Quit,
        "Quit",
        None,
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
    bind(
        Context::Help,
        &[Key::char('j'), Key::plain(KeyCode::Down)],
        Action::MoveDown,
        "Scroll down",
        None,
    ),
    bind(
        Context::Help,
        &[Key::char('k'), Key::plain(KeyCode::Up)],
        Action::MoveUp,
        "Scroll up",
        None,
    ),
];

/// The action a key performs in `context`.
pub fn action_for_key(context: Context, key: Key) -> Option<Action> {
    let find = |context: Context| {
        KEYMAP
            .iter()
            .find(|binding| binding.context == context && binding.keys.contains(&key))
            .map(|binding| binding.action)
    };
    let bound = find(context).or_else(|| match context {
        Context::Browse | Context::Queue => find(Context::Playback),
        _ => None,
    });
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
