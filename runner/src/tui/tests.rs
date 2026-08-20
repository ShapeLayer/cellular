//! Rendering and interaction tests driven through the real widget tree.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::config::{Config, Threads};

use super::{App, Focus, Mode, keys, ui};

fn app() -> App {
    let root = std::env::current_dir().expect("a working directory");
    let mut config = Config {
        index_depth: Some(2),
        ..Config::default()
    };
    config.index_detect_as_module = vec!["src/index".to_string()];
    App::new(root.clone(), root, config)
}

/// Draw the interface and return the screen as text, one line per row.
fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    terminal
        .draw(|frame| ui::render(frame, app))
        .expect("the interface draws");
    let buffer = terminal.backend().buffer().clone();
    let area = buffer.area;
    (0..area.height)
        .map(|y| {
            // A wide character owns two cells; the second is a placeholder that
            // must not be read back, or the row looks twice as wide as it is.
            let mut row = String::new();
            let mut x = 0;
            while x < area.width {
                let symbol = buffer[(x, y)].symbol();
                row.push_str(symbol);
                x += u16::try_from(ui::display_width(symbol).max(1)).unwrap_or(1);
            }
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(app: &mut App, code: KeyCode) {
    keys::handle(app, KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, KeyCode::Char(character));
    }
}

#[test]
fn the_normal_screen_shows_every_region() {
    let mut app = app();
    app.info("hello from the log");
    let output = screen(&mut app, 100, 30);

    assert!(output.contains("Cellular"), "{output}");
    assert!(output.contains("Now opened at:"), "{output}");
    assert!(output.contains("index_depth: 2"), "{output}");
    assert!(output.contains("hello from the log"), "{output}");
    assert!(output.contains('❯'), "{output}");
    assert!(output.contains("tab: Move to config list"), "{output}");
}

#[test]
fn a_short_screen_drops_the_config_box_but_keeps_the_command_line() {
    let mut app = app();
    let output = screen(&mut app, 60, 5);
    assert!(!output.contains("Now opened at:"), "{output}");
    assert!(output.contains('❯'), "{output}");
}

#[test]
fn tab_on_an_empty_command_line_moves_to_the_config_list() {
    let mut app = app();
    assert_eq!(app.focus, Focus::CommandLine);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::ConfigList);

    let output = screen(&mut app, 100, 30);
    assert!(output.contains("▶ index_depth"), "{output}");

    // esc hands focus back and restores the unselected marker.
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::CommandLine);
    let output = screen(&mut app, 100, 30);
    assert!(output.contains("● index_depth"), "{output}");
}

#[test]
fn typing_shows_candidates_over_the_log() {
    let mut app = app();
    for index in 0..20 {
        app.info(format!("log line {index}"));
    }
    type_text(&mut app, "h");
    let output = screen(&mut app, 100, 30);
    assert!(output.contains("help"), "{output}");
    assert!(
        output.contains("Show commands and their descriptions"),
        "{output}"
    );

    // Candidates cover the log lines directly above the command line.
    let lines: Vec<&str> = output.lines().collect();
    let input_row = lines
        .iter()
        .position(|line| line.starts_with('❯'))
        .expect("the command line is on screen");
    assert!(lines[input_row - 2].contains("help"), "{output}");
}

#[test]
fn candidates_are_filtered_by_prefix_and_accepted_with_tab() {
    let mut app = app();
    type_text(&mut app, "c");
    let completion = app.completion.as_ref().expect("candidates for `c`");
    let labels: Vec<&str> = completion
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    assert_eq!(labels, vec!["cd", "clear"]);

    // Nothing is picked yet, so enter would run the line as typed.
    assert!(completion.selected.is_none());
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.input, "clear ");
}

#[test]
fn arguments_complete_from_the_config_fields() {
    let mut app = app();
    type_text(&mut app, "set ind");
    let labels: Vec<String> = app
        .completion
        .as_ref()
        .expect("field candidates")
        .items
        .iter()
        .map(|item| item.label.clone())
        .collect();
    assert_eq!(
        labels,
        vec!["index_depth", "index_exclude", "index_detect_as_module"]
    );
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.input, "set index_depth ");
}

#[test]
fn editing_a_scalar_field_marks_it_unsaved() {
    let mut app = app();
    press(&mut app, KeyCode::Tab); // focus the config list
    press(&mut app, KeyCode::Enter); // edit index_depth
    assert!(matches!(app.mode, Mode::EditScalar { .. }));

    // Backspace stops at the start of the value, so the key is never eaten.
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    let output = screen(&mut app, 100, 30);
    assert!(output.contains("index_depth:"), "{output}");

    type_text(&mut app, "4");
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.config.index_depth, Some(4));
    assert!(app.dirty.contains("index_depth"));

    let output = screen(&mut app, 100, 30);
    assert!(output.contains("index_depth: 4 (unsaved)"), "{output}");
}

#[test]
fn escaping_a_scalar_edit_restores_the_old_value() {
    let mut app = app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "9");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.config.index_depth, Some(2));
    assert!(app.dirty.is_empty());
}

#[test]
fn the_thread_count_is_set_from_the_command_line() {
    let mut app = app();
    type_text(&mut app, "set threads 4");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.config.threads, Threads::Fixed(4));
    assert!(app.dirty.contains("threads"));

    // Zero threads is not a build, so the value is refused and the old one
    // stands.
    type_text(&mut app, "set threads 0");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.config.threads, Threads::Fixed(4));

    type_text(&mut app, "set threads auto");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.config.threads, Threads::Auto);

    let output = screen(&mut app, 100, 24);
    assert!(output.contains("threads: auto"), "{output}");
}

#[test]
fn a_bad_value_keeps_the_editor_open() {
    let mut app = app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "x");
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::EditScalar { .. }));
    assert!(
        app.log
            .iter()
            .any(|line| line.text.contains("must be a number"))
    );
}

#[test]
fn list_fields_open_the_full_screen_editor() {
    let mut app = app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Down); // index_exclude
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::EditArray(_)));

    let output = screen(&mut app, 60, 12);
    assert!(output.contains("index_exclude"), "{output}");
    assert!(output.contains(".git*"), "{output}");
    assert!(output.contains('~'), "{output}");
    assert!(output.contains("esc: cancel, ctrl+s: save"), "{output}");

    // Enter is a newline here, not a commit.
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "target");
    assert!(matches!(app.mode, Mode::EditArray(_)));

    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.config.index_exclude, vec![".git*", "target"]);
    assert!(app.dirty.contains("index_exclude"));
}

#[test]
fn long_values_scroll_sideways_with_markers() {
    let mut app = app();
    app.config.ignoring_files = (0..40).map(|index| format!("file-{index}.txt")).collect();
    press(&mut app, KeyCode::Tab);
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    assert_eq!(app.selected_field(), "ignoring_files");

    let output = screen(&mut app, 70, 30);
    assert!(output.contains('…'), "{output}");

    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Right);
    let scrolled = screen(&mut app, 70, 30);
    let row = scrolled
        .lines()
        .find(|line| line.contains("ignoring_files"))
        .expect("the row is on screen");
    // Both edges are cut once the row has been scrolled.
    assert_eq!(row.matches('…').count(), 2, "{row}");
}

#[test]
fn the_candidate_block_wipes_every_row_it_covers() {
    let mut app = app();
    for index in 0..10 {
        app.info(format!("log line {index} {}", "x".repeat(80)));
    }
    // The candidates follow the cursor, so log text sits on both sides of them.
    type_text(&mut app, "viewer ");
    assert!(app.completion.is_some());

    let output = screen(&mut app, 100, 30);
    let covered: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("Stop the running viewer web server."))
        .collect();
    assert_eq!(covered.len(), 1, "{output}");
    assert!(!covered[0].contains('x'), "{output}");
}

#[test]
fn the_command_history_is_recalled_with_the_arrow_keys() {
    let mut app = app();
    // The candidate list is open, but nothing is picked, so enter runs `help`.
    type_text(&mut app, "help");
    assert!(app.completion.is_some());
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.history, vec!["help"]);
    assert!(app.log.iter().any(|line| line.text.contains("❯ help")));

    press(&mut app, KeyCode::Up);
    assert_eq!(app.input, "help");
    press(&mut app, KeyCode::Down);
    assert_eq!(app.input, "");
}

#[test]
fn leaving_with_unsaved_changes_asks_first() {
    let mut clean = app();
    clean.request_quit();
    assert!(clean.quit);

    let mut app = app();
    app.dirty.insert("index_depth".to_string());
    app.request_quit();
    assert!(!app.quit);
    assert!(matches!(app.mode, Mode::Confirm { .. }));

    let output = screen(&mut app, 100, 30);
    assert!(output.contains("[y/n]"), "{output}");

    press(&mut app, KeyCode::Char('n'));
    assert!(app.quit);
}

#[test]
fn tab_takes_the_first_candidate_without_picking_one() {
    let mut app = app();
    type_text(&mut app, "vie");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.input, "viewer ");
}

#[test]
fn unknown_commands_are_reported() {
    let mut app = app();
    type_text(&mut app, "nope");
    // Nothing completes `nope`, so enter runs it straight away.
    assert!(app.completion.is_none());
    press(&mut app, KeyCode::Enter);
    assert!(
        app.log
            .iter()
            .any(|line| line.text.contains("unknown command"))
    );
}

// --- regression tests for defects found in review ---

#[test]
fn control_keys_do_not_type_letters() {
    let mut app = app();
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
    );
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.input, "", "control keys leaked into the command line");
}

#[test]
fn the_command_line_follows_a_long_cursor() {
    let mut app = app();
    type_text(&mut app, "index build ");
    for index in 0..60 {
        type_text(&mut app, &format!("{index},"));
    }
    assert!(app.completion.is_none());
    let output = screen(&mut app, 40, 12);
    let row = output
        .lines()
        .find(|line| line.starts_with('❯'))
        .expect("the command line is on screen")
        .to_string();
    assert!(
        row.contains("59,"),
        "the end of the line the cursor sits on is off screen: {row:?}"
    );
}

#[test]
fn list_elements_survive_a_comma() {
    let mut app = app();
    app.config.ignoring_files = vec!["a.txt".to_string()];
    press(&mut app, KeyCode::Tab);
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    assert_eq!(app.selected_field(), "ignoring_files");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "weird, name.txt");
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert_eq!(
        app.config.ignoring_files,
        vec!["a.txt", "weird, name.txt"],
        "an element containing a comma was split apart"
    );
}

#[test]
fn the_selected_row_is_highlighted_across_the_whole_row() {
    let mut app = app();
    press(&mut app, KeyCode::Tab);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 30)).expect("a terminal");
    terminal
        .draw(|frame| ui::render(frame, &mut app))
        .expect("draws");
    let buffer = terminal.backend().buffer().clone();

    // Row 2 of the screen is the first config row (border, path, then fields).
    // Columns 0 and 79 are the border, 1 and 78 the padding, so the content
    // runs from 2 to 77.
    let row = 2u16;
    let first = buffer[(2, row)].style().bg;
    let last = buffer[(77, row)].style().bg;
    assert_eq!(
        first, last,
        "the highlight stops before the end of the selected row"
    );
}

#[test]
fn wide_characters_stay_inside_their_region() {
    // Korean text is two columns per character, so counting characters would
    // overflow whatever region the text was measured for.
    for width in 1..12 {
        let clipped = ui::clip("가나다라마바사", 0, width);
        assert!(
            ui::display_width(&clipped) <= width,
            "{clipped:?} takes {} columns, not {width}",
            ui::display_width(&clipped)
        );
    }
    assert_eq!(ui::clip("가나다라마바사", 0, 7), "가나다…");
    assert_eq!(ui::clip("가나다라마바사", 2, 7), "…다라…");
}

#[test]
fn a_wide_project_path_does_not_overflow_the_config_box() {
    let mut app = app();
    app.project_root = std::path::PathBuf::from("/사용자/문서/저장소/아주긴이름의프로젝트폴더");
    app.config.ignoring_files = vec!["한글파일이름.txt".to_string(); 8];
    let output = screen(&mut app, 46, 20);
    for line in output.lines() {
        assert!(
            ui::display_width(line) <= 46,
            "{line:?} is {} columns wide",
            ui::display_width(line)
        );
    }
}

#[test]
fn the_command_line_scrolls_with_wide_characters() {
    let mut app = app();
    type_text(&mut app, "set index_exclude ");
    for _ in 0..20 {
        type_text(&mut app, "한글");
    }
    let output = screen(&mut app, 30, 12);
    let row = output
        .lines()
        .find(|line| line.starts_with('❯'))
        .expect("the command line is on screen");
    assert!(ui::display_width(row) <= 30, "{row:?}");
    assert!(
        row.ends_with("한글"),
        "the cursor end is off screen: {row:?}"
    );
}

/// A running job has to be visible without reading the log, since a build that
/// is inside one slow commit prints nothing at all while it works.
#[test]
fn a_running_job_is_badged_above_the_command_line() {
    let mut app = app();
    assert!(!screen(&mut app, 80, 30).contains("building:"));

    let (_sender, receiver) = std::sync::mpsc::channel();
    app.jobs.push(super::Job {
        kind: super::JobKind::Build,
        label: "index build all".to_string(),
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        progress: Some((3, 10)),
        receiver,
    });
    for index in 0..40 {
        app.info(format!("log line {index}"));
    }

    let output = screen(&mut app, 80, 30);
    let rows: Vec<&str> = output.lines().collect();
    let badge = rows
        .iter()
        .position(|row| row.contains("● building:"))
        .expect("the badge is drawn");

    let project = app
        .project_root
        .file_name()
        .expect("the project has a name")
        .to_string_lossy()
        .to_string();
    assert!(rows[badge].ends_with(&format!("● building: {project} (3/10)")));
    // Bottom-right of the log area: the row under it opens the command line.
    assert!(rows[badge + 1].starts_with('─'));
    assert!(rows[badge + 2].starts_with('❯'));
    // The log keeps running underneath instead of being pushed aside.
    assert!(rows[badge].contains("log line 39"));
}

/// A build is stopped by a command, never by a key press: esc on the command
/// line only clears what is typed there.
#[test]
fn a_running_build_is_cancelled_by_command_and_not_by_esc() {
    use std::sync::atomic::Ordering;

    let mut app = app();
    let (_sender, receiver) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    app.jobs.push(super::Job {
        kind: super::JobKind::Build,
        label: "index build all".to_string(),
        cancel: std::sync::Arc::clone(&cancel),
        progress: Some((0, 1)),
        receiver,
    });

    type_text(&mut app, "still typing");
    press(&mut app, KeyCode::Esc);
    assert!(!cancel.load(Ordering::Relaxed), "esc cancelled the build");
    assert!(app.input.is_empty(), "esc did not clear the command line");

    super::commands::execute(&mut app, "index cancel build");
    assert!(cancel.load(Ordering::Relaxed), "the command did not cancel");
}

/// Without a build running the command explains itself instead of doing
/// nothing, and it only accepts `build` as its target.
#[test]
fn cancelling_without_a_build_reports_why() {
    let mut app = app();
    super::commands::execute(&mut app, "index cancel build");
    let output = screen(&mut app, 100, 30);
    assert!(output.contains("no index build is running"), "{output}");

    super::commands::execute(&mut app, "index cancel viewer");
    let output = screen(&mut app, 100, 30);
    assert!(output.contains("unknown cancel target"), "{output}");
}

/// Install a fake build job so the tests can drive the interface as if one
/// were running. The sender is returned because dropping it ends the job.
fn running_build(app: &mut App) -> std::sync::mpsc::Sender<super::JobMessage> {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.jobs.push(super::Job {
        kind: super::JobKind::Build,
        label: "index build all".to_string(),
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        progress: Some((3, 10)),
        receiver,
    });
    sender
}

/// The viewer startup has no dependency on the project index, so it must not
/// occupy the build slot or make `index cancel build` ambiguous.
#[test]
fn a_viewer_start_and_index_build_can_run_together() {
    let mut app = app();
    let build_sender = running_build(&mut app);
    let (_viewer_sender, receiver) = std::sync::mpsc::channel();
    app.jobs.push(super::Job {
        kind: super::JobKind::Viewer,
        label: "viewer start".to_string(),
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        progress: None,
        receiver,
    });

    assert!(app.build_running());
    assert!(app.viewer_starting());
    assert_eq!(app.jobs.len(), 2);

    super::commands::execute(&mut app, "index cancel build");
    assert!(
        app.jobs[0]
            .cancel
            .load(std::sync::atomic::Ordering::Relaxed),
        "the build remains individually cancellable"
    );
    drop(build_sender);
}

/// The build runs on its own thread, so the interface stays fully usable while
/// it works: the config list, the editor and the command line all respond.
#[test]
fn the_interface_keeps_working_while_a_build_runs() {
    let mut app = app();
    let _sender = running_build(&mut app);

    // The config list still takes focus, moves and opens the editor.
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::ConfigList);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.selected, 1);
    press(&mut app, KeyCode::Enter);
    assert!(!matches!(app.mode, Mode::Normal), "the editor did not open");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::CommandLine);

    // The command line still completes and runs commands.
    type_text(&mut app, "hel");
    assert!(app.completion.is_some(), "candidates were not offered");
    press(&mut app, KeyCode::Esc);
    app.input.clear();
    app.cursor = 0;

    super::commands::execute(&mut app, "set index_depth 5");
    assert_eq!(app.config.index_depth, Some(5));
    let output = screen(&mut app, 100, 30);
    assert!(output.contains("● building:"), "{output}");
}

/// Only the commands that would fight the build for the files it is writing
/// have to wait, and they say so instead of racing it. Exporting is not one of
/// them: every flush leaves a whole index behind, so it packages what has been
/// measured so far.
#[test]
fn commands_that_touch_the_index_wait_for_the_build() {
    let mut app = app();
    let _sender = running_build(&mut app);

    for line in ["index clear", "index build HEAD"] {
        super::commands::execute(&mut app, line);
    }
    let refused = app
        .log
        .iter()
        .filter(|line| {
            line.kind == super::LogKind::Error && line.text.contains("index cancel build")
        })
        .count();
    assert_eq!(refused, 2, "each blocked command has to explain itself");

    super::commands::execute(&mut app, "index export");
    assert!(
        !app.log
            .iter()
            .any(|line| line.text.contains("before exporting")),
        "exporting part way through a build is allowed"
    );
}

/// The badge counts the whole job, so a build that is part way through says so
/// without the user having to read the `[n/m]` notes back out of the log.
#[test]
fn the_build_badge_follows_the_commit_count() {
    let mut app = app();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.jobs.push(super::Job {
        kind: super::JobKind::Build,
        label: "index build all".to_string(),
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        progress: Some((0, 12)),
        receiver,
    });
    assert!(screen(&mut app, 80, 30).contains("(0/12)"));

    sender
        .send(super::JobMessage::Progress { done: 7, total: 12 })
        .expect("the job is still listening");
    app.poll_job();
    assert!(screen(&mut app, 80, 30).contains("(7/12)"));
}
