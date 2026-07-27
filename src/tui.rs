//! `agent-wall --tui`: a ratatui dashboard with one panel per live tmux pane,
//! each panel replaying that pane's real colored screen.
//!
//! Panes, not sessions: a session with three windows is three separate live
//! terminals, and rendering only the session's active pane silently hides the
//! rest of the work the user asked to watch.

use crate::tmux::{self, PaneInfo};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Paragraph, Widget};
use ratatui::{DefaultTerminal, Frame};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tui_term::widget::{Cursor, PseudoTerminal};

/// How long a frame blocks waiting for a keypress. Also the input latency
/// ceiling: quitting feels instant because we never block longer than this.
const TICK: Duration = Duration::from_millis(100);

/// Gap between tmux re-reads. Deliberately far longer than [`TICK`] — every
/// refresh forks one `list-panes` plus one `capture-pane` per pane, so driving
/// it per frame would spawn dozens of processes a second for no visible gain.
const REFRESH: Duration = Duration::from_millis(750);

const WAYBAR_PANEL_COLS: u16 = 32;
const WAYBAR_ROWS: u16 = 10;

/// Column count for `n` panels. Wide-and-short beats tall-and-thin here: a
/// terminal screen is ~80x24, so panels must stay wide enough to avoid
/// re-wrapping the captured text into nonsense.
fn columns_for(n: usize) -> u16 {
    match n {
        0 => 0,
        1 => 1,
        2..=4 => 2,
        5..=9 => 3,
        _ => 4,
    }
}

/// One slice of an even split: the `index`-th of `total` slices of `len`
/// starting at `start`.
///
/// Remainder cells go to the leftmost/topmost slices one apiece, so the slices
/// tile `len` *exactly* — no gap, no overlap, nothing lost to rounding. Done in
/// `u32` so the intermediate product cannot wrap a `u16`.
fn span(start: u16, len: u16, total: u32, index: u32) -> (u16, u16) {
    let total = total.max(1);
    let len = u32::from(len);
    let (base, remainder) = (len / total, len % total);
    let offset = base * index + index.min(remainder);
    let size = base + u32::from(index < remainder);
    let clamp = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    (start.saturating_add(clamp(offset)), clamp(size))
}

/// Split `area` into exactly `n` row-major cells.
///
/// Always returns exactly `n` rects, every one inside `area` and none
/// overlapping. When `area` is smaller than the grid it implies, cells degrade
/// to zero width/height rather than spilling outside — a caller rendering into
/// a 3x2 terminal gets useless but *safe* rects, never a panic.
fn grid(area: Rect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let cols = u32::from(columns_for(n)).max(1);
    let rows = u32::try_from(n.div_ceil(cols as usize)).unwrap_or(u32::MAX);

    (0..n)
        .map(|i| {
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            let (x, width) = span(area.x, area.width, cols, i % cols);
            let (y, height) = span(area.y, area.height, rows, i / cols);
            Rect {
                x,
                y,
                width,
                height,
            }
        })
        .collect()
}

fn row_grid(area: Rect, n: usize) -> Vec<Rect> {
    let total = u32::try_from(n).unwrap_or(u32::MAX);
    (0..total)
        .map(|index| {
            let (x, width) = span(area.x, area.width, total, index);
            Rect {
                x,
                y: area.y,
                width,
                height: area.height,
            }
        })
        .collect()
}

/// One pane's decoded screen, ready to render.
struct PaneView {
    parser: vt100::Parser,
    title: String,
    attached: bool,
}

/// Decode one pane's captured bytes into a screen sized to that pane.
///
/// The parser is built fresh per snapshot on purpose. `capture-pane` hands back
/// a whole screen dump every tick, not an incremental byte stream; replaying
/// dumps into a surviving parser would accumulate stale cursor position and
/// scrollback that the real pane never had.
///
/// `Parser::new` takes ROWS BEFORE COLS, the transpose of tmux's `WIDTHxHEIGHT`
/// and of [`PaneInfo`]'s field order. Feeding it a pane's real geometry is what
/// makes wrapping land where the user actually sees it; a swap here renders
/// plausible-looking garbage rather than failing loudly.
fn decode_capture(capture: &[u8], rows: u16, cols: u16) -> vt100::Parser {
    let mut parser = vt100::Parser::new(rows.max(1), cols.max(1), 0);
    parser.process(capture);
    parser
}

fn is_opencode_cost_line(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.ends_with("spent") && trimmed.contains('$')
}

fn tail_parser(screen: &vt100::Screen, rows: u16) -> vt100::Parser {
    let (_, cols) = screen.size();
    let plain: Vec<String> = screen.rows(0, cols).collect();
    let end = plain.iter().enumerate().fold(0, |end, (index, line)| {
        if line.trim().is_empty() || is_opencode_cost_line(line) {
            end
        } else {
            index + 1
        }
    });
    let start = end.saturating_sub(usize::from(rows));
    let mut parser = vt100::Parser::new(rows.max(1), cols.max(1), 0);
    for (index, (line, text)) in screen
        .rows_formatted(0, cols)
        .zip(&plain)
        .enumerate()
        .take(end)
        .skip(start)
    {
        if is_opencode_cost_line(text) {
            parser.process(b" ");
        } else {
            parser.process(&line);
        }
        if index + 1 < end {
            parser.process(b"\r\n");
        }
    }
    parser
}

fn decode(pane: &PaneInfo, rows: Option<u16>) -> vt100::Parser {
    let parser = decode_capture(
        &tmux::capture_pane_styled(&pane.pane_id),
        pane.rows,
        pane.cols,
    );
    match rows {
        Some(rows) => tail_parser(parser.screen(), rows),
        None => parser,
    }
}

fn title_for(pane: &PaneInfo, session: Option<&tmux::Meta>, now: i64) -> String {
    let state = if session.is_some_and(|s| s.attached) {
        "attached"
    } else {
        "detached"
    };
    let idle = session.map_or_else(|| "--:--".to_string(), |s| s.idle_label(now));
    format!(
        "{state} {}:{}.{} {} idle {idle}",
        pane.session_name, pane.window_index, pane.pane_index, pane.command
    )
}

fn snapshot(now: i64, rows: Option<u16>) -> Vec<PaneView> {
    let sessions = tmux::list_sessions();
    let by_name: HashMap<&str, &tmux::Meta> =
        sessions.iter().map(|s| (s.name.as_str(), s)).collect();

    tmux::list_panes()
        .iter()
        .map(|pane| {
            let session = by_name.get(pane.session_name.as_str()).copied();
            PaneView {
                parser: decode(pane, rows),
                title: title_for(pane, session, now),
                attached: session.is_some_and(|s| s.attached),
            }
        })
        .collect()
}

fn render(frame: &mut Frame, panes: &[PaneView]) {
    let area = frame.area();
    render_buffer(area, panes, frame.buffer_mut());
}

fn render_buffer(area: Rect, panes: &[PaneView], buffer: &mut Buffer) {
    if panes.is_empty() {
        render_empty(area, buffer);
        return;
    }
    render_cells(grid(area, panes.len()), panes, buffer);
}

fn render_cells(cells: Vec<Rect>, panes: &[PaneView], buffer: &mut Buffer) {
    for (cell, view) in cells.into_iter().zip(panes) {
        let color = if view.attached {
            Color::Green
        } else {
            Color::DarkGray
        };
        let block = Block::bordered()
            .border_style(Style::default().fg(color))
            .title(view.title.as_str());
        PseudoTerminal::new(view.parser.screen())
            .block(block)
            .cursor(Cursor::default().visibility(false))
            .render(cell, buffer);
    }
}

fn render_empty(area: Rect, buffer: &mut Buffer) {
    let block = Block::bordered().title("agent-wall");
    let inner = block.inner(area);
    block.render(area, buffer);
    let text = Paragraph::new("No tmux sessions are running.\nStart one with `tmux new`, then press r.\n\nq / Esc / Ctrl-C to quit.")
        .alignment(Alignment::Center);
    let height = inner.height.min(4);
    text.render(
        Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(height) / 2,
            width: inner.width,
            height,
        },
        buffer,
    );
}

pub fn render_waybar_to_buffer() -> Buffer {
    let panes = snapshot(tmux::now_secs(), Some(WAYBAR_ROWS.saturating_sub(2)));
    let pane_count = u16::try_from(panes.len()).unwrap_or(u16::MAX);
    let width = WAYBAR_PANEL_COLS.saturating_mul(pane_count.max(1));
    let area = Rect::new(0, 0, width, WAYBAR_ROWS);
    let mut buffer = Buffer::empty(area);
    if panes.is_empty() {
        render_empty(area, &mut buffer);
    } else {
        let cells = row_grid(area, panes.len());
        render_cells(cells.clone(), &panes, &mut buffer);
        compact_panel_rows(&mut buffer, &cells);
    }
    buffer
}

fn compact_panel_rows(buffer: &mut Buffer, panels: &[Rect]) {
    for panel in panels {
        if panel.width <= 2 || panel.height <= 2 {
            continue;
        }
        let rows: Vec<Vec<_>> = (panel.y + 1..panel.bottom() - 1)
            .filter(|y| {
                (panel.x + 1..panel.right() - 1)
                    .any(|x| !buffer[(x, *y)].symbol().trim().is_empty())
            })
            .map(|y| {
                (panel.x + 1..panel.right() - 1)
                    .map(|x| buffer[(x, y)].clone())
                    .collect()
            })
            .collect();
        let empty_rows = usize::from(panel.height - 2).saturating_sub(rows.len());

        for (offset, y) in (panel.y + 1..panel.bottom() - 1).enumerate() {
            for (column, x) in (panel.x + 1..panel.right() - 1).enumerate() {
                if let Some(row) = offset
                    .checked_sub(empty_rows)
                    .and_then(|index| rows.get(index))
                {
                    buffer[(x, y)] = row[column].clone();
                } else {
                    buffer[(x, y)].reset();
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    fg: Color,
    bg: Color,
    modifiers: Modifier,
}

impl From<&ratatui::buffer::Cell> for CellStyle {
    fn from(cell: &ratatui::buffer::Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            modifiers: cell.modifier,
        }
    }
}

/// Serialize a ratatui buffer as escaped Pango markup, coalescing adjacent
/// cells that carry the same foreground, background, and modifiers.
pub fn buffer_to_pango(buffer: &Buffer) -> String {
    let area = *buffer.area();
    let mut output = String::new();
    for y in area.y..area.bottom() {
        let mut style = None;
        let mut text = String::new();
        for x in area.x..area.right() {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            let next = CellStyle::from(cell);
            if style.is_some_and(|current| current != next) {
                push_pango_run(&mut output, style, &text);
                text.clear();
            }
            style = Some(next);
            text.push_str(cell.symbol());
        }
        push_pango_run(&mut output, style, &text);
        if y + 1 < area.bottom() {
            output.push('\n');
        }
    }
    output
}

fn push_pango_run(output: &mut String, style: Option<CellStyle>, text: &str) {
    if text.is_empty() {
        return;
    }
    let Some(style) = style else {
        return;
    };
    let mut attributes = String::new();
    let (fg, bg) = if style.modifiers.contains(Modifier::REVERSED) {
        (style.bg, style.fg)
    } else {
        (style.fg, style.bg)
    };
    push_color_attribute(&mut attributes, "foreground", fg);
    push_color_attribute(&mut attributes, "background", bg);
    if style.modifiers.contains(Modifier::BOLD) {
        attributes.push_str(" weight=\"bold\"");
    }
    if style.modifiers.contains(Modifier::DIM) {
        attributes.push_str(" alpha=\"60%\"");
    }
    if style.modifiers.contains(Modifier::ITALIC) {
        attributes.push_str(" style=\"italic\"");
    }
    if style.modifiers.contains(Modifier::UNDERLINED) {
        attributes.push_str(" underline=\"single\"");
    }
    if style.modifiers.contains(Modifier::CROSSED_OUT) {
        attributes.push_str(" strikethrough=\"true\"");
    }
    if style.modifiers.contains(Modifier::HIDDEN) {
        attributes.push_str(" alpha=\"0\"");
    }

    if attributes.is_empty() {
        push_escaped(output, text);
    } else {
        output.push_str("<span");
        output.push_str(&attributes);
        output.push('>');
        push_escaped(output, text);
        output.push_str("</span>");
    }
}

fn push_color_attribute(output: &mut String, name: &str, color: Color) {
    if let Some((red, green, blue)) = color_rgb(color) {
        output.push(' ');
        output.push_str(name);
        output.push_str("=\"#");
        output.push_str(&format!("{red:02x}{green:02x}{blue:02x}"));
        output.push('"');
    }
}

fn color_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Reset => None,
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((205, 0, 0)),
        Color::Green => Some((0, 205, 0)),
        Color::Yellow => Some((205, 205, 0)),
        Color::Blue => Some((0, 0, 238)),
        Color::Magenta => Some((205, 0, 205)),
        Color::Cyan => Some((0, 205, 205)),
        Color::Gray => Some((229, 229, 229)),
        Color::DarkGray => Some((127, 127, 127)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((92, 92, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Rgb(red, green, blue) => Some((red, green, blue)),
        Color::Indexed(index) => Some(indexed_rgb(index)),
    }
}

fn indexed_rgb(index: u8) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => ANSI[usize::from(index)],
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let offset = index - 16;
            (
                LEVELS[usize::from(offset / 36)],
                LEVELS[usize::from((offset / 6) % 6)],
                LEVELS[usize::from(offset % 6)],
            )
        }
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            (level, level, level)
        }
    }
}

fn push_escaped(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

/// In raw mode the terminal stops translating Ctrl-C into SIGINT, so the
/// process never receives one — an unhandled Ctrl-C would simply be swallowed
/// and the user would think the app had wedged. It must be matched by hand.
fn is_quit(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        || (matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
            && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Restore the terminal *before* the default hook prints the panic. Skipping
/// this leaves the user in raw mode on the alternate screen with no echo and
/// no visible message — a shell that looks dead and needs a blind `reset`.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        previous(info);
    }));
}

fn event_loop(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    let mut panes = snapshot(tmux::now_secs(), None);
    let mut refreshed = Instant::now();
    let mut forced = false;

    loop {
        terminal.draw(|frame| render(frame, &panes))?;

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if is_quit(&key) {
                return Ok(());
            }
            forced = matches!(key.code, KeyCode::Char('r'));
        }

        if std::mem::take(&mut forced) || refreshed.elapsed() >= REFRESH {
            panes = snapshot(tmux::now_secs(), None);
            refreshed = Instant::now();
        }
    }
}

/// Run the dashboard, owning the terminal for the whole call: raw mode and the
/// alternate screen are entered here and always left here, including on error.
pub fn run() -> std::io::Result<()> {
    install_panic_hook();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal);
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 200,
        height: 60,
    };

    fn contains(outer: Rect, inner: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.right() <= outer.right()
            && inner.bottom() <= outer.bottom()
    }

    #[test]
    fn zero_panes_is_empty() {
        assert!(grid(AREA, 0).is_empty());
    }

    #[test]
    fn returns_exactly_n_rects() {
        for n in [1usize, 2, 5, 24] {
            assert_eq!(grid(AREA, n).len(), n, "wrong cell count for n={n}");
        }
    }

    #[test]
    fn one_pane_fills_the_area() {
        let cells = grid(AREA, 1);
        assert_eq!(cells, vec![AREA]);
    }

    #[test]
    fn two_panes_split_into_two_columns_side_by_side() {
        let cells = grid(AREA, 2);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].y, cells[1].y, "n=2 must be one row");
        assert!(cells[0].x < cells[1].x, "n=2 must read left to right");
    }

    #[test]
    fn no_cell_overflows_the_area() {
        for n in [1usize, 2, 5, 24] {
            for (i, cell) in grid(AREA, n).into_iter().enumerate() {
                assert!(contains(AREA, cell), "cell {i} of n={n} escaped: {cell:?}");
            }
        }
    }

    #[test]
    fn cells_are_row_major_and_deterministic() {
        let n = 5;
        let cells = grid(AREA, n);
        let cols = usize::from(columns_for(n));
        for (i, cell) in cells.iter().enumerate() {
            if i % cols != 0 {
                let left = cells[i - 1];
                assert!(cell.x > left.x, "cell {i} must sit right of its neighbour");
                assert_eq!(cell.y, left.y, "cell {i} must share its row's y");
            }
        }
        assert!(cells[cols].y > cells[0].y, "row 2 must sit below row 1");
        assert_eq!(cells, grid(AREA, n), "grid must be deterministic");
    }

    #[test]
    fn cells_do_not_overlap() {
        let cells = grid(AREA, 5);
        for (i, a) in cells.iter().enumerate() {
            for b in cells.iter().skip(i + 1) {
                let disjoint =
                    a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
                assert!(disjoint, "cells overlap: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn tiny_area_still_returns_every_cell() {
        let tiny = Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 2,
        };
        let cells = grid(tiny, 24);
        assert_eq!(cells.len(), 24);
        for cell in cells {
            assert!(contains(tiny, cell), "cell escaped tiny area: {cell:?}");
        }
    }

    #[test]
    fn column_thresholds() {
        assert_eq!(columns_for(0), 0);
        assert_eq!(columns_for(1), 1);
        assert_eq!(columns_for(2), 2);
        assert_eq!(columns_for(4), 2);
        assert_eq!(columns_for(5), 3);
        assert_eq!(columns_for(9), 3);
        assert_eq!(columns_for(10), 4);
        assert_eq!(columns_for(64), 4);
    }

    #[test]
    fn waybar_cells_share_one_row_without_gaps() {
        let cells = row_grid(AREA, 5);

        assert_eq!(cells.len(), 5);
        assert!(cells.iter().all(|cell| cell.y == AREA.y));
        assert!(cells.iter().all(|cell| cell.height == AREA.height));
        assert_eq!(cells.first().map(|cell| cell.x), Some(AREA.x));
        assert_eq!(cells.last().map(|cell| cell.right()), Some(AREA.right()));
        for pair in cells.windows(2) {
            assert_eq!(pair[0].right(), pair[1].x);
        }
    }

    #[test]
    fn waybar_panel_cells_leave_room_for_eight_content_lines() {
        let cells = row_grid(Rect::new(0, 0, WAYBAR_PANEL_COLS * 4, WAYBAR_ROWS), 4);

        assert!(cells.iter().all(|cell| cell.width == WAYBAR_PANEL_COLS));
        assert!(cells.iter().all(|cell| cell.height == WAYBAR_ROWS));
    }

    #[test]
    fn tail_parser_strips_opencode_cost_lines() {
        // Given
        let capture = b"work\r\n142,592 tokens\r\n74% used\r\n$32.06 spent\r\nlatest";

        // When
        let full = decode_capture(capture, 5, 32);
        let parser = tail_parser(full.screen(), 5);

        // Then
        let contents = parser.screen().contents();
        assert!(
            !contents.contains("spent"),
            "cost line leaked: {contents:?}"
        );
        assert!(contents.contains("latest"));
    }

    #[test]
    fn decoder_keeps_last_eight_lines_when_capture_is_taller() {
        // Given
        let capture =
            b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\neight\r\nnine\r\n\r\n\r\n";

        // When
        let full = decode_capture(capture, 12, 16);
        let parser = tail_parser(full.screen(), 8);

        // Then
        assert_eq!(
            parser.screen().contents(),
            "two\nthree\nfour\nfive\nsix\nseven\neight\nnine"
        );
    }

    #[test]
    fn waybar_places_latest_compacted_row_at_panel_bottom() {
        let panel = Rect::new(0, 0, 6, 6);
        let mut buffer = Buffer::empty(panel);
        buffer[(1, 1)].set_symbol("A");
        buffer[(1, 3)].set_symbol("B");

        compact_panel_rows(&mut buffer, &[panel]);

        assert_eq!(buffer[(1, 1)].symbol(), " ");
        assert_eq!(buffer[(1, 2)].symbol(), " ");
        assert_eq!(buffer[(1, 3)].symbol(), "A");
        assert_eq!(buffer[(1, 4)].symbol(), "B");
    }

    #[test]
    fn pango_escapes_markup_text() {
        // Given
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer[(0, 0)].set_symbol("&");
        buffer[(1, 0)].set_symbol("<");
        buffer[(2, 0)].set_symbol(">");

        // When
        let markup = buffer_to_pango(&buffer);

        // Then
        assert_eq!(markup, "&amp;&lt;&gt;");
    }

    #[test]
    fn pango_coalesces_matching_styles_and_splits_transitions() {
        // Given
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 4, 1));
        for (x, symbol) in ["a", "b", "c", "d"].into_iter().enumerate() {
            buffer[(u16::try_from(x).expect("test coordinate fits u16"), 0)].set_symbol(symbol);
        }
        buffer[(0, 0)].set_style(Style::default().fg(Color::Red).bold());
        buffer[(1, 0)].set_style(Style::default().fg(Color::Red).bold());
        buffer[(2, 0)].set_style(Style::default().bg(Color::Blue).italic());
        buffer[(3, 0)].set_style(Style::default().bg(Color::Blue).italic());

        // When
        let markup = buffer_to_pango(&buffer);

        // Then
        assert_eq!(
            markup,
            "<span foreground=\"#cd0000\" weight=\"bold\">ab</span>\
             <span background=\"#0000ee\" style=\"italic\">cd</span>"
        );
    }
}
