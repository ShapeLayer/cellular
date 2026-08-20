//! Drawing the terminal interface.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::FIELDS;

use super::commands;
use super::editor::ArrayEditor;
use super::{Anchor, App, Focus, JobKind, LogKind, Mode, PROMPT_WIDTH};

const ACCENT: Color = Color::Cyan;
const SELECTED_BG: Color = Color::Indexed(236);
const DIM: Color = Color::DarkGray;
/// The running-job badge, in a colour no log line uses, so it reads as status
/// rather than as another line of output.
const BUSY: Color = Color::Magenta;

/// Rows the config list may occupy, before the 45% cap applies.
const CONFIG_ROWS: usize = 10;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 8 || area.height < 3 {
        return;
    }

    // The array editor takes the whole screen.
    if let Mode::EditArray(editor) = &mut app.mode {
        let inner_height = area.height.saturating_sub(5) as usize;
        editor.scroll_into_view(inner_height);
        let editor = editor.clone();
        render_array_editor(frame, area, &editor);
        return;
    }

    let regions = Regions::compute(area);
    if let Some(config_area) = regions.config {
        render_config_box(frame, config_area, app);
    }
    render_log(frame, regions.log, app);
    render_completion(frame, regions.log, app);
    // Last, so neither the log nor the candidate list can cover it.
    render_job_badge(frame, regions.log, app);
    render_input(frame, regions.input, app);
    render_guide(frame, regions.guide, app);
}

/// Where each region sits. Regions disappear from the top down as the terminal
/// shrinks, so the command line and the key guide always survive.
struct Regions {
    config: Option<Rect>,
    log: Rect,
    input: Rect,
    guide: Rect,
}

impl Regions {
    fn compute(area: Rect) -> Self {
        let height = area.height as usize;
        let guide_height = 1;
        let input_height = 3usize.min(height.saturating_sub(guide_height));
        let remaining = height.saturating_sub(guide_height + input_height);

        // (*5) The config list never takes more than 45% of the screen.
        let rows = ((height * 45) / 100).clamp(1, CONFIG_ROWS);
        // Two borders plus the "Now opened at" line.
        let wanted = rows + 3;
        let config_height = if remaining >= wanted + 2 {
            wanted
        } else if remaining >= 6 {
            remaining - 2
        } else {
            0
        };
        let log_height = remaining - config_height;

        let mut y = area.y;
        let config = if config_height >= 4 {
            let rect = Rect::new(area.x, y, area.width, config_height as u16);
            y += config_height as u16;
            Some(rect)
        } else {
            y += config_height as u16;
            None
        };
        let log = Rect::new(area.x, y, area.width, log_height as u16);
        y += log_height as u16;
        let input = Rect::new(area.x, y, area.width, input_height as u16);
        y += input_height as u16;
        let guide = Rect::new(area.x, y, area.width, guide_height as u16);

        Regions {
            config,
            log,
            input,
            guide,
        }
    }
}

// ------------------------------------------------------------ config box --

fn render_config_box(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if app.focus == Focus::ConfigList {
            ACCENT
        } else {
            DIM
        }))
        .title("─── Cellular ───")
        // A blank column on each side, so text never touches the border.
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let width = inner.width as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled("Now opened at: ", Style::default().fg(DIM)),
        Span::raw(clip(
            &app.project_root.display().to_string(),
            0,
            width.saturating_sub(15),
        )),
    ])];

    let visible_rows = inner.height.saturating_sub(1) as usize;
    let (rows, owners) = config_rows(app, width);
    scroll_list_into_view(app, &owners, visible_rows);
    lines.extend(
        rows.into_iter()
            .skip(app.list_scroll)
            .take(visible_rows.max(1)),
    );

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Build the config list, one or two display lines per field, and report which
/// field each display line belongs to.
fn config_rows(app: &App, width: usize) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut lines = Vec::new();
    let mut owners = Vec::new();

    for (index, field) in FIELDS.iter().enumerate() {
        let selected = index == app.selected;
        let editing =
            matches!(&app.mode, Mode::EditScalar { field: editing, .. } if editing == field);

        let marker = if selected && app.focus == Focus::ConfigList {
            '▶'
        } else {
            '●'
        };
        let value = match &app.mode {
            Mode::EditScalar {
                field: name,
                buffer,
                ..
            } if name == field => buffer.clone(),
            _ => app.config.display_value(field).unwrap_or_default(),
        };
        let head = format!("{marker} {field}: ");
        let origin_full = commands::describe_origin(app, field);
        let origin_short = commands::describe_origin_short(app, field);

        let row_style = if selected {
            Style::default().bg(SELECTED_BG)
        } else {
            Style::default()
        };
        let value_style = if editing {
            row_style.add_modifier(Modifier::UNDERLINED).fg(ACCENT)
        } else {
            row_style
        };
        let origin_style = row_style.fg(DIM);

        // (*4) Full path, then a project-relative path, then a wrapped line.
        let with_full = format!("{head}{value} ({origin_full})");
        let with_short = format!("{head}{value} ({origin_short})");
        let (origin_inline, wrap_origin) = if display_width(&with_full) <= width {
            (Some(origin_full.clone()), false)
        } else if display_width(&with_short) <= width {
            (Some(origin_short.clone()), false)
        } else {
            (None, true)
        };

        let scroll = if selected { app.horizontal_scroll } else { 0 };
        let value_width =
            width
                .saturating_sub(head.chars().count())
                .saturating_sub(match &origin_inline {
                    Some(origin) => origin.chars().count() + 3,
                    None => 0,
                });

        let shown_value = clip(&value, scroll, value_width);
        let mut used = display_width(&head) + display_width(&shown_value);
        let mut spans = vec![
            Span::styled(head.clone(), row_style),
            Span::styled(shown_value, value_style),
        ];
        if let Some(origin) = origin_inline {
            let text = format!(" ({origin})");
            used += display_width(&text);
            spans.push(Span::styled(text, origin_style));
        }
        // The highlight has to reach the end of the row, so pad it out.
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used)),
            row_style,
        ));
        lines.push(Line::from(spans));
        owners.push(index);

        if wrap_origin {
            let text = format!("  ↳ {origin_short}");
            let padding = width.saturating_sub(display_width(&text));
            lines.push(Line::from(vec![
                Span::styled(text, origin_style),
                Span::styled(" ".repeat(padding), row_style),
            ]));
            owners.push(index);
        }
    }

    (lines, owners)
}

/// Keep every display line of the selected field on screen.
fn scroll_list_into_view(app: &mut App, owners: &[usize], visible: usize) {
    if visible == 0 || owners.is_empty() {
        return;
    }
    let first = owners
        .iter()
        .position(|owner| *owner == app.selected)
        .unwrap_or(0);
    let last = owners
        .iter()
        .rposition(|owner| *owner == app.selected)
        .unwrap_or(first);

    if first < app.list_scroll {
        app.list_scroll = first;
    } else if last >= app.list_scroll + visible {
        app.list_scroll = last + 1 - visible;
    }
    app.list_scroll = app.list_scroll.min(owners.len().saturating_sub(1));
}

// ------------------------------------------------------------- log area --

fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // (*3) Notices stay pinned above the log until dismissed.
    for notice in &app.notices {
        lines.push(Line::from(Span::styled(
            clip(&format!("! {} (ctrl+n: dismiss)", notice.text), 0, width),
            Style::default().fg(Color::Yellow),
        )));
    }

    let room = (area.height as usize).saturating_sub(lines.len());
    let start = app.log.len().saturating_sub(room);
    for entry in &app.log[start..] {
        let style = match entry.kind {
            LogKind::Info => Style::default(),
            LogKind::Command => Style::default().fg(ACCENT),
            LogKind::Warn => Style::default().fg(Color::Yellow),
            LogKind::Error => Style::default().fg(Color::Red),
            LogKind::Success => Style::default().fg(Color::Green),
        };
        lines.push(Line::from(Span::styled(clip(&entry.text, 0, width), style)));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// A running job is announced in the bottom-right of the log area, directly
/// above the command line. A build reports each commit only as it finishes, so
/// without this a slow commit leaves the log still and gives no sign the job is
/// alive at all.
fn render_job_badge(frame: &mut Frame, log_area: Rect, app: &App) {
    let Some(job) = app
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::Build)
        .or_else(|| app.jobs.first())
    else {
        return;
    };
    if log_area.height == 0 {
        return;
    }

    let project = app
        .project_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| app.project_root.display().to_string());
    let text = match job.kind {
        // The count is the whole job, reused commits included, so it reaches
        // the total even on a build that measures nothing.
        JobKind::Build => match job.progress {
            Some((done, total)) => format!("● building: {project} ({done}/{total})"),
            None => format!("● building: {project}"),
        },
        JobKind::Viewer => format!("● starting viewer: {project}"),
    };

    // A blank column keeps the badge off whatever log line it sits beside.
    let width = display_width(&text) + 1;
    if width > log_area.width as usize {
        return;
    }
    let area = Rect::new(
        log_area.x + log_area.width - width as u16,
        log_area.y + log_area.height - 1,
        width as u16,
        1,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(text, Style::default().fg(BUSY).add_modifier(Modifier::BOLD)),
        ])),
        area,
    );
}

// ---------------------------------------------------------- completion ----

/// (*6) Candidates are drawn over the log area, right above the command line,
/// with blank rows above them so the two do not run together.
fn render_completion(frame: &mut Frame, log_area: Rect, app: &mut App) {
    let Some(completion) = &app.completion else {
        return;
    };
    if log_area.height == 0 || completion.items.is_empty() {
        return;
    }

    let height = log_area.height as usize;
    let gap = ((height * 20) / 100).clamp(1, 3);
    let cap = ((height * 50) / 100).max(1);
    let list_height = completion
        .items
        .len()
        .min(cap)
        .min(height.saturating_sub(gap))
        .max(1);

    let label_width = completion
        .items
        .iter()
        .map(|item| display_width(&item.label))
        .max()
        .unwrap_or(0)
        .clamp(4, 24);
    let content_width = completion
        .items
        .iter()
        .map(|item| label_width + 2 + display_width(&item.description))
        .max()
        .unwrap_or(20)
        .min(log_area.width as usize);

    // Line the candidates up with the cursor as it is actually drawn, without
    // letting the block leave the screen.
    let anchor_column = match completion.anchor {
        Anchor::LeftEdge => 0,
        Anchor::Cursor => {
            let width = log_area.width as usize;
            let scroll = input_scroll(app, width);
            let before: String = app
                .input
                .chars()
                .skip(scroll)
                .take(app.cursor.saturating_sub(scroll))
                .collect();
            PROMPT_WIDTH + display_width(&before)
        }
    };
    let anchor = anchor_column.min((log_area.width as usize).saturating_sub(content_width));

    let total = gap + list_height;
    let top = log_area.y + log_area.height.saturating_sub(total as u16);
    // The candidate block is narrower than the log lines it covers, so clearing
    // only the block would leave the tail of a log line running out of both of
    // its sides. Every row the block touches is wiped edge to edge first.
    let band = Rect::new(log_area.x, top, log_area.width, total as u16);
    let list = Rect::new(
        log_area.x + anchor as u16,
        top + gap as u16,
        content_width as u16,
        list_height as u16,
    );

    frame.render_widget(Clear, band);

    let selected = completion.selected;
    let scroll = selected
        .unwrap_or(0)
        .saturating_sub(list_height.saturating_sub(1));
    let lines: Vec<Line<'static>> = completion
        .items
        .iter()
        .enumerate()
        .skip(scroll)
        .take(list_height)
        .map(|(index, item)| {
            let style = if Some(index) == selected {
                Style::default().fg(ACCENT).add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let label = clip(&item.label, 0, label_width);
            let padding = " ".repeat(label_width.saturating_sub(display_width(&label)));
            let text = format!("{label}{padding}  {}", item.description);
            Line::from(Span::styled(clip(&text, 0, content_width), style))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), list);
}

// ---------------------------------------------------------- command line --

/// How far the command line is scrolled, in characters, so the cursor stays on
/// screen. Measured in columns, so wide characters shift it correctly.
fn input_scroll(app: &App, width: usize) -> usize {
    let visible = width.saturating_sub(PROMPT_WIDTH).max(1);
    let characters: Vec<char> = app.input.chars().collect();
    let cursor = app.cursor.min(characters.len());
    let mut scroll = cursor;
    let mut used = 0;
    while scroll > 0 {
        let columns = char_width(characters[scroll - 1]);
        if used + columns > visible - 1 {
            break;
        }
        used += columns;
        scroll -= 1;
    }
    scroll
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let rule = Line::from(Span::styled("─".repeat(width), Style::default().fg(DIM)));

    let (prompt, body, style) = match &app.mode {
        // The answer hint must survive the clip, so only the question is cut.
        Mode::Confirm { question, .. } => {
            let room = width.saturating_sub(PROMPT_WIDTH + 6);
            (
                "? ",
                format!("{} [y/n]", clip(question, 0, room)),
                Style::default().fg(Color::Yellow),
            )
        }
        _ => ("❯ ", app.input.clone(), Style::default()),
    };

    // The command line scrolls with the cursor instead of being cut short.
    let visible = width.saturating_sub(PROMPT_WIDTH).max(1);
    let scroll = match app.mode {
        Mode::Confirm { .. } => 0,
        _ => input_scroll(app, width),
    };
    let shown = window(&body, scroll, visible);

    let mut lines = vec![rule.clone()];
    lines.push(Line::from(vec![
        Span::styled(prompt, Style::default().fg(ACCENT)),
        Span::styled(shown, style),
    ]));
    if area.height >= 3 {
        lines.push(rule);
    }
    frame.render_widget(Paragraph::new(lines), area);

    if matches!(app.mode, Mode::Normal) && app.focus == Focus::CommandLine && area.height >= 2 {
        let before: String = app
            .input
            .chars()
            .skip(scroll)
            .take(app.cursor.saturating_sub(scroll))
            .collect();
        let column = (PROMPT_WIDTH + display_width(&before)).min(width.saturating_sub(1));
        frame.set_cursor_position((area.x + column as u16, area.y + 1));
    }
}

// ------------------------------------------------------------ key guide --

fn render_guide(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let text = guide_text(app);
    let width = area.width as usize;
    // (*8) When the guide does not fit, it scrolls one character at a time.
    let shown = if display_width(&text) <= width {
        text
    } else {
        let padded: Vec<char> = format!("{text}    ").chars().collect();
        let offset = app.guide_offset % padded.len();
        let rotated: String = padded[offset..].iter().chain(&padded[..offset]).collect();
        clip(&rotated, 0, width)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(shown, Style::default().fg(DIM)))),
        area,
    );
}

fn guide_text(app: &App) -> String {
    let mut parts: Vec<String> = Vec::new();
    match &app.mode {
        Mode::Confirm { .. } => {
            parts.push("y: yes".into());
            parts.push("n: no".into());
            parts.push("esc: cancel".into());
        }
        Mode::EditScalar { .. } => {
            parts.push("enter: apply".into());
            parts.push("esc: cancel".into());
            parts.push("←/→: move".into());
        }
        Mode::EditArray(_) => {}
        Mode::Normal => match app.focus {
            Focus::ConfigList => {
                parts.push("w/s or ↑/↓: select".into());
                parts.push("a/d or ←/→: scroll".into());
                parts.push("enter: edit".into());
                parts.push("tab/esc: command line".into());
            }
            Focus::CommandLine => {
                if app.completion.is_some() {
                    parts.push("↑/↓: choose".into());
                    parts.push("enter/tab: accept".into());
                    parts.push("esc: dismiss".into());
                } else {
                    if app.input.trim().is_empty() {
                        parts.push("tab: Move to config list".into());
                    }
                    parts.push("enter: run".into());
                    parts.push("↑/↓: history".into());
                }
                if app.build_running() {
                    parts.insert(0, "`index cancel build`: cancel the build".into());
                }
            }
        },
    }
    if !app.notices.is_empty() {
        parts.push("ctrl+n: dismiss notice".into());
    }
    if !app.dirty.is_empty() {
        parts.push(format!(
            "{} unsaved change(s), `save` writes them",
            app.dirty.len()
        ));
    }
    parts.push("ctrl+c: quit".into());
    parts.join(", ")
}

// --------------------------------------------------------- array editor ---

/// (*5) The list editor: a bordered screen with the key guide on the bottom
/// three lines, and vi-style `~` past the end of the content.
fn render_array_editor(frame: &mut Frame, area: Rect, editor: &ArrayEditor) {
    let width = area.width as usize;
    let box_height = area.height.saturating_sub(3);
    let box_area = Rect::new(area.x, area.y, area.width, box_height);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(format!("─── {} ───", editor.field))
        .padding(Padding::horizontal(1));
    let inner = block.inner(box_area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, box_area);

    let rows = inner.height as usize;
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        match editor.lines.get(editor.scroll + row) {
            Some(text) => lines.push(Line::from(clip(text, 0, inner.width as usize))),
            None => lines.push(Line::from(Span::styled(
                "~",
                Style::default().fg(Color::Blue),
            ))),
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);

    let rule = Line::from(Span::styled("─".repeat(width), Style::default().fg(DIM)));
    let guide = Rect::new(area.x, area.y + box_height, area.width, 3);
    frame.render_widget(
        Paragraph::new(vec![
            rule.clone(),
            Line::from(Span::styled(
                "esc: cancel, ctrl+s: save",
                Style::default().fg(DIM),
            )),
            rule,
        ]),
        guide,
    );

    if editor.row >= editor.scroll && editor.row < editor.scroll + rows {
        let before: String = editor.lines[editor.row]
            .chars()
            .take(editor.column)
            .collect();
        let column = display_width(&before).min(inner.width.saturating_sub(1) as usize);
        frame.set_cursor_position((
            inner.x + column as u16,
            inner.y + (editor.row - editor.scroll) as u16,
        ));
    }
}

// -------------------------------------------------------------- helpers ---

/// Terminal columns a string occupies. Korean and other wide characters take
/// two, so counting characters would overflow every region they land in.
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn char_width(character: char) -> usize {
    UnicodeWidthChar::width(character).unwrap_or(0)
}

/// Cut `text` down to `width` terminal columns, starting `offset` characters
/// in, marking each cut edge with `…`.
pub fn clip(text: &str, offset: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let characters: Vec<char> = text.chars().collect();
    let offset = offset.min(characters.len());
    let rest: String = characters[offset..].iter().collect();
    let lead = usize::from(offset > 0);

    if lead + display_width(&rest) <= width {
        let mut out = String::with_capacity(rest.len() + 4);
        if lead == 1 {
            out.push('…');
        }
        out.push_str(&rest);
        return out;
    }
    // There is not enough room even for one character plus both markers.
    if width <= lead + 1 {
        return "…".to_string();
    }

    let budget = width - lead - 1;
    let mut out = String::new();
    if lead == 1 {
        out.push('…');
    }
    let mut used = 0;
    for character in &characters[offset..] {
        let columns = char_width(*character);
        if used + columns > budget {
            break;
        }
        out.push(*character);
        used += columns;
    }
    out.push('…');
    out
}

/// Take as many characters from `offset` as fit in `width` columns, without
/// any marker. Used where a marker would move the cursor, such as the command
/// line.
fn window(text: &str, offset: usize, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for character in text.chars().skip(offset) {
        let columns = char_width(character);
        if used + columns > width {
            break;
        }
        out.push(character);
        used += columns;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_marks_both_cut_edges() {
        assert_eq!(clip("abcdef", 0, 10), "abcdef");
        assert_eq!(clip("abcdef", 0, 4), "abc…");
        // The offset hides "ab"; the leading marker takes one column.
        assert_eq!(clip("abcdef", 2, 4), "…cd…");
        assert_eq!(clip("abcdef", 3, 10), "…def");
        assert_eq!(clip("abcdef", 0, 0), "");
    }

    #[test]
    fn regions_keep_the_command_line_on_tiny_screens() {
        let small = Regions::compute(Rect::new(0, 0, 40, 5));
        assert!(small.config.is_none());
        assert_eq!(small.input.height, 3);
        assert_eq!(small.guide.height, 1);

        let large = Regions::compute(Rect::new(0, 0, 100, 40));
        let config = large.config.expect("the config box fits");
        // The list itself stays within 45% of the screen height.
        assert!(config.height as usize <= (40 * 45) / 100 + 3);
        assert!(large.log.height > 0);
    }

    #[test]
    fn the_config_list_shrinks_with_the_screen() {
        let regions = Regions::compute(Rect::new(0, 0, 80, 20));
        let config = regions.config.expect("the config box fits");
        // 45% of 20 rows is 9 list rows, plus two borders and the path line.
        assert_eq!(config.height, 12);
    }
}
