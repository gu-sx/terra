//! Draws the state: a header, the sidebar and the main list, the bottom bar
//! and, when open, the playlist picker or the help overlay.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Clear, Paragraph, Row, Table};
use terra_core::PlaylistEntry;

use super::app::{App, Browse, Focus, Item, Load, View, visible_offset};
use super::keymap::{Binding, Context, KEYMAP};
use super::playlist::{Modal, Picker};

const HEADER_HEIGHT: u16 = 3;
const FOOTER_HEIGHT: u16 = 5;
const SIDEBAR_WIDTH: u16 = 22;
/// Wide enough for `h:mm:ss`, `artist` and a year.
const TIME_WIDTH: u16 = 7;
/// The width of the help overlay's key column.
const HELP_KEYS_WIDTH: usize = 14;
const PICKER_WIDTH: u16 = 50;
/// The most playlists the picker shows at once.
const PICKER_ROWS: usize = 10;

const ACCENT: Color = Color::Cyan;

/// Where each part of the screen goes.
struct Areas {
    header: Rect,
    sidebar: Rect,
    list: Rect,
    footer: Rect,
}

fn layout(area: Rect) -> Areas {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Fill(1),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .areas(area);
    let [sidebar, list] =
        Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Fill(1)]).areas(body);
    Areas {
        header,
        sidebar,
        list,
        footer,
    }
}

/// How many rows of the main list fit on a terminal of this size: the list
/// pane less its borders and its column header.
pub fn list_height(width: u16, height: u16) -> usize {
    let list = layout(Rect::new(0, 0, width, height)).list;
    usize::from(list.height.saturating_sub(3))
}

/// Draws the whole screen.
pub fn render(app: &App, frame: &mut Frame) {
    let areas = layout(frame.area());
    render_header(app, frame, areas.header);
    render_sidebar(app, frame, areas.sidebar);
    render_list(app, frame, areas.list);
    render_footer(app, frame, areas.footer);
    if let Some(Modal::Picker(picker)) = &app.modal {
        render_picker(app, picker, frame, frame.area());
    }
    if app.help {
        render_help(frame, frame.area());
    }
}

/// A pane's border, highlighted when the pane has the focus.
fn pane(focused: bool) -> Block<'static> {
    if focused {
        Block::bordered()
            .border_type(BorderType::Thick)
            .border_style(Style::new().fg(ACCENT))
    } else {
        Block::bordered().border_style(Style::new().fg(Color::DarkGray))
    }
}

/// The style of the selected row: reversed in the focused pane, bold in the
/// other.
fn selected_style(focused: bool) -> Style {
    if focused {
        Style::new()
            .fg(ACCENT)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    }
}

fn marker(selected: bool) -> &'static str {
    if selected { "> " } else { "  " }
}

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::bordered().border_style(Style::new().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let title = Line::from(vec![
        Span::styled("◉ terra", Style::new().fg(ACCENT).bold()),
        Span::raw("  "),
        Span::raw(app.library.clone()),
    ]);
    let title_width = u16::try_from(title.width()).unwrap_or(u16::MAX);
    let [title_area, _, breadcrumb_area] = Layout::horizontal([
        Constraint::Length(title_width),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(title), title_area);
    let breadcrumb = tail(&app.breadcrumb(), usize::from(breadcrumb_area.width));
    let breadcrumb = Line::from(breadcrumb).bold().right_aligned();
    frame.render_widget(Paragraph::new(breadcrumb), breadcrumb_area);
}

/// The end of `text` that fits in `width` columns, after an ellipsis when
/// the start is cut.
fn tail(text: &str, width: usize) -> String {
    if Line::from(text).width() <= width {
        return text.to_owned();
    }
    let mut start = text.char_indices().map(|(index, _)| index);
    loop {
        let rest = match start.next() {
            Some(index) => &text[index..],
            None => return String::new(),
        };
        let cut = format!("…{rest}");
        if Line::from(cut.as_str()).width() <= width {
            return cut;
        }
    }
}

fn render_sidebar(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Sidebar;
    let block = pane(focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let heading = |text: &'static str| Line::from(text).fg(Color::DarkGray).bold();
    let entry = |index: usize, label: String, active: bool| {
        let selected = app.sidebar == index;
        let mut style = if active {
            Style::new().fg(ACCENT)
        } else {
            Style::new()
        };
        if selected {
            style = style.patch(selected_style(focused));
        }
        Line::styled(format!("{}{label}", marker(selected)), style)
    };

    let mut lines = vec![heading("BROWSE")];
    let mut cursor_line = 0;
    for (index, browse) in Browse::ALL.into_iter().enumerate() {
        if app.sidebar == index {
            cursor_line = lines.len();
        }
        lines.push(entry(
            index,
            browse.label().to_owned(),
            browse.view() == app.root_view(),
        ));
    }
    lines.push(Line::default());
    lines.push(heading("PLAYLISTS"));
    match &app.playlists {
        Load::Loading => lines.push(Line::from("  Loading…").dim()),
        Load::Failed(error) => lines.push(Line::from(format!("  {error}")).red()),
        Load::Ready(playlists) if playlists.is_empty() => {
            lines.push(Line::from("  No playlists").dim());
        }
        Load::Ready(playlists) => {
            for (offset, playlist) in playlists.iter().enumerate() {
                let index = Browse::ALL.len() + offset;
                if app.sidebar == index {
                    cursor_line = lines.len();
                }
                let active = View::Playlist(playlist.id) == app.root_view();
                lines.push(entry(index, playlist.name.clone(), active));
            }
        }
    }

    let height = usize::from(inner.height);
    let skip = (cursor_line + 1).saturating_sub(height);
    let visible: Vec<Line> = lines.into_iter().skip(skip).take(height).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

/// The column headers of the main list.
fn columns(app: &App) -> (&'static str, &'static str) {
    if app.searching() {
        return ("RESULT", "");
    }
    match app.top().view {
        View::Tracks(_) | View::Playlist(_) => ("TITLE / ARTIST", "TIME"),
        View::Artists => ("ARTIST", ""),
        View::Albums(_) | View::ArtistAlbums(_) => ("ALBUM / ARTIST", "YEAR"),
    }
}

/// The two cells of a row: its name and its right-hand column.
fn item_cells(item: &Item, searching: bool) -> (String, String) {
    match item {
        Item::Artist(artist) => {
            let kind = if searching { "artist" } else { "" };
            (artist.name.clone(), kind.to_owned())
        }
        Item::Album(album) => {
            let right = if searching {
                "album".to_owned()
            } else {
                album.year.map(|year| year.to_string()).unwrap_or_default()
            };
            (
                format!("{} / {}", album.title, album.album_artist.name),
                right,
            )
        }
        Item::Track(track) | Item::Entry(PlaylistEntry { track, .. }) => (
            format!("{} / {}", track.title, track.artist.name),
            format_duration(track.duration),
        ),
    }
}

/// Formats a duration as `mm:ss`, or `h:mm:ss` from an hour on.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn render_list(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::List;
    let block = pane(focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (name, right) = columns(app);
    let header = Row::new([
        Cell::from(format!("  {name}")),
        Cell::from(Line::from(right).right_aligned()),
    ])
    .style(Style::new().fg(Color::DarkGray).bold());
    let widths = [Constraint::Fill(1), Constraint::Length(TIME_WIDTH)];
    let [header_area, rows_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(Table::new([header], widths), header_area);

    let searching = app.searching();
    let items = match app.rows() {
        Load::Loading => {
            frame.render_widget(Paragraph::new("  Loading…").dim(), rows_area);
            return;
        }
        Load::Failed(error) => {
            let text = format!("  Could not load the list: {error}");
            frame.render_widget(Paragraph::new(text).red(), rows_area);
            return;
        }
        Load::Ready(items) if items.is_empty() => {
            let text = match &app.top().search {
                Some(search) if search.is_active() => {
                    format!("  No matches for “{}”", search.query)
                }
                _ => "  Nothing here yet".to_owned(),
            };
            frame.render_widget(Paragraph::new(text).dim(), rows_area);
            return;
        }
        Load::Ready(items) => items,
    };

    // Only the rows that fit are turned into widgets.
    let height = usize::from(rows_area.height);
    let cursor = app.cursor();
    let offset = visible_offset(cursor.selected, cursor.offset, height);
    let rows = items
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, item)| {
            let selected = index == cursor.selected;
            let (name, right) = item_cells(item, searching);
            let row = Row::new([
                Cell::from(format!("{}{name}", marker(selected))),
                Cell::from(Line::from(right).right_aligned()),
            ]);
            if selected {
                row.style(selected_style(focused))
            } else {
                row
            }
        });
    frame.render_widget(Table::new(rows, widths), rows_area);
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::bordered().border_style(Style::new().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [playing, progress, hints] = Layout::vertical([Constraint::Length(1); 3]).areas(inner);

    frame.render_widget(Paragraph::new("■ Nothing playing").dim(), playing);
    let bar = "░".repeat(usize::from(progress.width));
    frame.render_widget(Paragraph::new(bar).fg(Color::DarkGray), progress);

    let context = Context::of(app);
    let prompt = match (&app.modal, app.editing_search()) {
        (Some(Modal::Name { renaming, text }), _) => {
            let label = match renaming {
                Some(playlist) => format!("Rename “{}”: ", playlist.name),
                None => "New playlist: ".to_owned(),
            };
            Some(text_prompt(label, text))
        }
        (Some(Modal::Delete(playlist)), _) => Some(Line::from(vec![
            Span::styled("Delete ", Style::new().fg(ACCENT).bold()),
            Span::raw(format!("“{}”?", playlist.name)),
        ])),
        (Some(Modal::Picker(_)), _) => None,
        (None, Some(search)) => Some(text_prompt("/".to_owned(), &search.query)),
        (None, None) => None,
    };
    if let Some(prompt) = prompt {
        frame.render_widget(Paragraph::new(prompt), hints);
        frame.render_widget(Paragraph::new(hint_line(context).right_aligned()), hints);
    } else if let Some(status) = &app.status {
        let style = if status.error {
            Style::new().fg(Color::Red)
        } else {
            Style::new()
        };
        frame.render_widget(Paragraph::new(status.text.clone()).style(style), hints);
    } else {
        frame.render_widget(Paragraph::new(hint_line(context)), hints);
    }
}

/// A label, the text typed after it and a cursor.
fn text_prompt(label: String, text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::new().fg(ACCENT).bold()),
        Span::raw(text.to_owned()),
        Span::styled("█", Style::new().fg(ACCENT)),
    ])
}

/// The playlist picker: its filter, then the playlists that match it.
fn render_picker(app: &App, picker: &Picker, frame: &mut Frame, area: Rect) {
    // The picker keeps its size while the filter narrows it.
    let shown = app.playlist_list().len().clamp(1, PICKER_ROWS);
    let matches = picker.matches(app.playlist_list());
    let height = u16::try_from(shown + 3).unwrap_or(u16::MAX);
    let popup = centered(area, PICKER_WIDTH, height);
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(Style::new().fg(ACCENT))
        .title(format!(" Add “{}” to ", picker.track.title));
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    let [query_area, rows_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new(text_prompt("› ".to_owned(), &picker.query)),
        query_area,
    );

    if matches.is_empty() {
        let text = if picker.query.is_empty() {
            "  No playlists".to_owned()
        } else {
            format!("  No playlists match “{}”", picker.query)
        };
        frame.render_widget(Paragraph::new(text).dim(), rows_area);
        return;
    }
    let height = usize::from(rows_area.height);
    let offset = visible_offset(picker.selected, 0, height);
    let lines: Vec<Line> = matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, playlist)| {
            let selected = index == picker.selected;
            let line = Line::from(format!("{}{}", marker(selected), playlist.name));
            if selected {
                line.style(selected_style(true))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), rows_area);
}

/// The key hints of the bindings in `context` that have one.
fn hint_line(context: Context) -> Line<'static> {
    let mut spans = Vec::new();
    for binding in KEYMAP.iter().filter(|binding| binding.context == context) {
        let (Some(hint), Some(key)) = (binding.hint, binding.keys.first()) else {
            continue;
        };
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key.name(), Style::new().fg(ACCENT).bold()));
        spans.push(Span::raw(format!(" {hint}")).dim());
    }
    Line::from(spans)
}

/// One line of the help overlay.
pub fn help_line(binding: &Binding) -> String {
    format!(
        "{:<width$} {}",
        binding.keys_label(),
        binding.description,
        width = HELP_KEYS_WIDTH
    )
}

fn render_help(frame: &mut Frame, area: Rect) {
    let section = |context: Context| {
        let mut lines = vec![Line::from(context.title()).fg(ACCENT).bold()];
        let typing = match context {
            Context::Search => Some("Add to the query"),
            Context::Prompt => Some("Add to the text"),
            Context::Browse | Context::Confirm | Context::Help => None,
        };
        if let Some(typing) = typing {
            lines.push(Line::from(format!(
                "{:<HELP_KEYS_WIDTH$} {typing}",
                "(type)"
            )));
        }
        lines.extend(
            KEYMAP
                .iter()
                .filter(|binding| binding.context == context)
                .map(|binding| Line::from(help_line(binding))),
        );
        lines
    };
    // The overlay's own keys go on its bottom border, under the left column.
    let [first, rest @ .., help] = Context::ALL;
    let left = section(first);
    let mut right = Vec::new();
    for context in rest {
        if !right.is_empty() {
            right.push(Line::default());
        }
        right.extend(section(context));
    }

    let rows = u16::try_from(left.len().max(right.len())).unwrap_or(u16::MAX);
    let popup = centered(area, 78, rows.saturating_add(2));
    let mut block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(Style::new().fg(ACCENT))
        .title(format!(" {} ", help.title()));
    for binding in KEYMAP.iter().filter(|binding| binding.context == help) {
        block = block.title_bottom(help_line(binding));
    }
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(Paragraph::new(left), left_area);
    frame.render_widget(Paragraph::new(right), right_area);
}

/// A rectangle of at most `width` by `height` in the middle of `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
