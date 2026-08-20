//! Key handling for every mode of the terminal interface.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::Config;

use super::commands;
use super::editor::ArrayEditor;
use super::{App, ConfirmAction, Focus, Mode};

pub fn handle(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && handle_control(app, key.code) {
        return;
    }
    // Shortcuts this interface does not use must not fall through and type
    // their letter: ctrl+w would otherwise insert a `w`.
    if matches!(key.code, KeyCode::Char(_)) && is_shortcut(key.modifiers) {
        return;
    }

    match &app.mode {
        Mode::Confirm { .. } => confirm_key(app, key),
        Mode::EditArray(_) => array_key(app, key),
        Mode::EditScalar { .. } => scalar_key(app, key),
        Mode::Normal => match app.focus {
            Focus::ConfigList => list_key(app, key),
            Focus::CommandLine => command_key(app, key),
        },
    }
}

/// Modifier combinations that make a character key a shortcut rather than
/// text. Shift is excluded: it only produces capitals.
fn is_shortcut(modifiers: KeyModifiers) -> bool {
    modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

/// Returns true when the key was a control shortcut and is fully handled.
fn handle_control(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('c') => {
            match &app.mode {
                Mode::EditScalar { .. } | Mode::EditArray(_) | Mode::Confirm { .. } => {
                    app.mode = Mode::Normal;
                }
                Mode::Normal => app.request_quit(),
            }
            true
        }
        KeyCode::Char('n') => {
            app.dismiss_first_notice();
            true
        }
        KeyCode::Char('s') => {
            if let Mode::EditArray(editor) = &app.mode {
                // Handed over as a list: joining on commas would split any
                // element that contains one.
                let value = editor.value();
                let field = editor.field;
                commit_list_field(app, field, value);
            }
            true
        }
        _ => false,
    }
}

// ----------------------------------------------------------- config list --

fn list_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('w') => {
            app.selected = app.selected.saturating_sub(1);
            app.horizontal_scroll = 0;
        }
        KeyCode::Down | KeyCode::Char('s') => {
            app.selected = (app.selected + 1).min(crate::config::FIELDS.len() - 1);
            app.horizontal_scroll = 0;
        }
        KeyCode::Left | KeyCode::Char('a') => {
            app.horizontal_scroll = app.horizontal_scroll.saturating_sub(1);
        }
        KeyCode::Right | KeyCode::Char('d') => {
            app.horizontal_scroll += 1;
        }
        KeyCode::Home => {
            app.selected = 0;
            app.horizontal_scroll = 0;
        }
        KeyCode::End => {
            app.selected = crate::config::FIELDS.len() - 1;
            app.horizontal_scroll = 0;
        }
        KeyCode::Enter => begin_edit(app),
        KeyCode::Esc | KeyCode::Tab => app.focus = Focus::CommandLine,
        _ => {}
    }
}

fn begin_edit(app: &mut App) {
    let field = app.selected_field();
    if Config::is_list_field(field) {
        let items = app
            .config
            .list_value(field)
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        app.mode = Mode::EditArray(ArrayEditor::new(field, &items));
    } else {
        let buffer = raw_scalar_value(&app.config, field);
        let cursor = buffer.chars().count();
        app.mode = Mode::EditScalar {
            field,
            buffer,
            cursor,
        };
    }
}

/// The editable text of a single-valued field, without any decoration.
pub fn raw_scalar_value(config: &Config, field: &str) -> String {
    match field {
        "index_depth" => config
            .index_depth
            .map(|value| value.to_string())
            .unwrap_or_default(),
        "select_date_query_result_is_multiple" => {
            config.select_date_query_result_is_multiple.to_string()
        }
        "select_date_query_result_is_none" => config.select_date_query_result_is_none.to_string(),
        _ => config.display_value(field).unwrap_or_default(),
    }
}

fn commit_list_field(app: &mut App, field: &'static str, value: Vec<String>) {
    match commands::apply_list_value(app, field, value) {
        Ok(()) => {
            let shown = app.config.display_value(field).unwrap_or_default();
            app.success(format!("{field} = {shown} (unsaved)"));
            app.mode = Mode::Normal;
        }
        Err(error) => app.error(format!("{error:#}")),
    }
}

fn commit_field(app: &mut App, field: &'static str, value: &str) {
    match commands::apply_field_value(app, field, value) {
        Ok(()) => {
            let shown = app.config.display_value(field).unwrap_or_default();
            app.success(format!("{field} = {shown} (unsaved)"));
            app.mode = Mode::Normal;
        }
        // Stay in the editor so the user can correct the value.
        Err(error) => app.error(format!("{error:#}")),
    }
}

// ------------------------------------------------------- scalar editing ---

fn scalar_key(app: &mut App, key: KeyEvent) {
    let Mode::EditScalar {
        field,
        buffer,
        cursor,
    } = &mut app.mode
    else {
        return;
    };
    let field = *field;

    match key.code {
        KeyCode::Char(character) => {
            let at = byte_index(buffer, *cursor);
            buffer.insert(at, character);
            *cursor += 1;
        }
        KeyCode::Backspace => {
            // Only the value is editable: at its start, backspace does nothing
            // so the field name can never be deleted.
            if *cursor > 0 {
                let at = byte_index(buffer, *cursor - 1);
                buffer.remove(at);
                *cursor -= 1;
            }
        }
        KeyCode::Delete => {
            if *cursor < buffer.chars().count() {
                let at = byte_index(buffer, *cursor);
                buffer.remove(at);
            }
        }
        KeyCode::Left => *cursor = cursor.saturating_sub(1),
        KeyCode::Right => *cursor = (*cursor + 1).min(buffer.chars().count()),
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = buffer.chars().count(),
        KeyCode::Enter => {
            let value = buffer.clone();
            commit_field(app, field, &value);
        }
        KeyCode::Esc => app.mode = Mode::Normal,
        _ => {}
    }
}

fn byte_index(text: &str, column: usize) -> usize {
    text.char_indices()
        .nth(column)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

// -------------------------------------------------------- array editing ---

fn array_key(app: &mut App, key: KeyEvent) {
    let Mode::EditArray(editor) = &mut app.mode else {
        return;
    };
    match key.code {
        KeyCode::Char(character) => editor.insert_char(character),
        KeyCode::Backspace => editor.backspace(),
        KeyCode::Delete => editor.delete(),
        // Enter inserts a line here; ctrl+s is what keeps the change.
        KeyCode::Enter => editor.newline(),
        KeyCode::Up => editor.move_up(),
        KeyCode::Down => editor.move_down(),
        KeyCode::Left => editor.move_left(),
        KeyCode::Right => editor.move_right(),
        KeyCode::Home => editor.move_home(),
        KeyCode::End => editor.move_end(),
        KeyCode::Esc => app.mode = Mode::Normal,
        _ => {}
    }
}

// -------------------------------------------------------- command line ----

fn command_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char(character) => {
            let at = byte_index(&app.input, app.cursor);
            app.input.insert(at, character);
            app.cursor += 1;
            refresh_completion(app);
        }
        KeyCode::Backspace => {
            if app.cursor > 0 {
                let at = byte_index(&app.input, app.cursor - 1);
                app.input.remove(at);
                app.cursor -= 1;
            }
            refresh_completion(app);
        }
        KeyCode::Delete => {
            if app.cursor < app.input.chars().count() {
                let at = byte_index(&app.input, app.cursor);
                app.input.remove(at);
            }
            refresh_completion(app);
        }
        KeyCode::Left => app.cursor = app.cursor.saturating_sub(1),
        KeyCode::Right => app.cursor = (app.cursor + 1).min(app.input.chars().count()),
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.cursor = app.input.chars().count(),
        KeyCode::Up => {
            if app.completion.is_some() {
                move_candidate(app, -1);
            } else {
                recall_history(app, -1);
            }
        }
        KeyCode::Down => {
            if app.completion.is_some() {
                move_candidate(app, 1);
            } else {
                recall_history(app, 1);
            }
        }
        KeyCode::Tab => {
            if app.completion.is_some() {
                // Tab is the completion key: with nothing picked it takes the
                // first candidate.
                accept_completion(app, true);
            } else if app.input.trim().is_empty() {
                app.focus = Focus::ConfigList;
            } else {
                refresh_completion(app);
                if app.completion.is_some() {
                    accept_completion(app, true);
                }
            }
        }
        KeyCode::Enter => {
            // Enter inserts a candidate only once one has been picked;
            // otherwise it runs what was typed.
            if app
                .completion
                .as_ref()
                .is_some_and(|completion| completion.selected.is_some())
            {
                accept_completion(app, false);
                return;
            }
            let line = std::mem::take(&mut app.input);
            app.cursor = 0;
            app.history_position = None;
            // The candidate list belongs to the line that just ran.
            app.completion = None;
            if !line.trim().is_empty() {
                app.history.push(line.trim().to_string());
            }
            commands::execute(app, &line);
        }
        KeyCode::Esc => {
            // A running build is stopped with `index cancel build`, never with
            // a key press: esc only clears what is on the command line.
            if app.completion.is_some() {
                app.completion = None;
            } else {
                app.input.clear();
                app.cursor = 0;
            }
        }
        _ => {}
    }
}

pub fn refresh_completion(app: &mut App) {
    if app.input.is_empty() {
        app.completion = None;
        return;
    }
    app.completion = commands::complete(app, &app.input.clone(), app.cursor);
}

fn move_candidate(app: &mut App, delta: isize) {
    let Some(completion) = &mut app.completion else {
        return;
    };
    let count = completion.items.len() as isize;
    if count == 0 {
        return;
    }
    let next = match completion.selected {
        Some(current) => (current as isize + delta).rem_euclid(count),
        // The first move enters the list from whichever end it came from.
        None if delta < 0 => count - 1,
        None => 0,
    };
    completion.selected = Some(next as usize);
}

fn accept_completion(app: &mut App, allow_first: bool) {
    let Some(completion) = app.completion.take() else {
        return;
    };
    let index = match completion.selected {
        Some(index) => index,
        None if allow_first => 0,
        None => return,
    };
    let Some(item) = completion.items.get(index) else {
        return;
    };

    let characters: Vec<char> = app.input.chars().collect();
    let head: String = characters[..completion.replace_from.min(characters.len())]
        .iter()
        .collect();
    let tail: String = characters[app.cursor.min(characters.len())..]
        .iter()
        .collect();
    // A directory candidate stays open so its children can be completed next.
    let inserted = if item.label.ends_with('/') {
        item.label.clone()
    } else {
        format!("{} ", item.label)
    };

    app.cursor = head.chars().count() + inserted.chars().count();
    app.input = format!("{head}{inserted}{tail}");
    refresh_completion(app);
}

fn recall_history(app: &mut App, delta: isize) {
    if app.history.is_empty() {
        return;
    }
    let last = app.history.len() - 1;
    let next = match (app.history_position, delta) {
        (None, -1) => Some(last),
        (None, _) => None,
        (Some(position), -1) => Some(position.saturating_sub(1)),
        (Some(position), _) if position >= last => None,
        (Some(position), _) => Some(position + 1),
    };
    app.history_position = next;
    app.input = next
        .and_then(|position| app.history.get(position).cloned())
        .unwrap_or_default();
    app.cursor = app.input.chars().count();
    app.completion = None;
}

// -------------------------------------------------------- confirmations ---

fn confirm_key(app: &mut App, key: KeyEvent) {
    let accepted = match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => true,
        KeyCode::Char('n') | KeyCode::Char('N') => false,
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            return;
        }
        _ => return,
    };

    let Mode::Confirm { action, .. } = app.mode.clone() else {
        return;
    };
    app.mode = Mode::Normal;

    match action {
        ConfirmAction::QuitWithUnsaved => {
            if accepted {
                match commands::save_config(app, false) {
                    Ok(path) => app.success(format!("saved to {}", path.display())),
                    Err(error) => {
                        app.error(format!("could not save: {error:#}"));
                        return; // Stay so the user can decide what to do.
                    }
                }
            }
            app.quit = true;
        }
        ConfirmAction::StartBuild { resolved, force } => {
            if accepted {
                app.spawn_build(resolved, force);
            } else {
                app.info("the build was not started");
            }
        }
        ConfirmAction::RebuildDamaged { specs } => {
            if !accepted {
                app.info("keeping the index as it is; `index build` rebuilds it later");
                return;
            }
            if specs.is_empty() {
                commands::execute(app, "index clear");
            } else {
                app.start_build(specs, true);
            }
        }
    }
}

/// Bracketed paste, routed to whatever is being edited.
pub fn paste(app: &mut App, text: &str) {
    match &mut app.mode {
        Mode::EditArray(editor) => editor.insert_str(text),
        Mode::EditScalar { buffer, cursor, .. } => {
            let at = byte_index(buffer, *cursor);
            buffer.insert_str(at, text);
            *cursor += text.chars().count();
        }
        Mode::Normal if app.focus == Focus::CommandLine => {
            let at = byte_index(&app.input, app.cursor);
            app.input.insert_str(at, text);
            app.cursor += text.chars().count();
            refresh_completion(app);
        }
        _ => {}
    }
}
