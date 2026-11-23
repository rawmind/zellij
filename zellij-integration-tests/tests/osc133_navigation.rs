#![cfg(unix)]

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::engine::Engine as _;
use zellij_integration_tests::{
    keys, FakePtyHandle, GridSnapshot, TestRunner, TestSession, TERMINAL_SIZE,
};

const NAVIGATION_CONFIG: &str = concat!(
    "mouse_mode true\n",
    "copy_clipboard \"system\"\n",
    "keybinds {\n",
    " normal {\n",
    "  bind \"Ctrl y\" { ScrollToPreviousPrompt; }\n",
    "  bind \"Ctrl u\" { ScrollToNextPrompt; }\n",
    "  bind \"Ctrl v\" { SelectCommandAtScrollPosition; }\n",
    "  bind \"Ctrl x\" { CopyLastCommandOutput; }\n",
    " }\n",
    " scroll {\n",
    "  bind \"Ctrl y\" { Copy; }\n",
    " }\n",
    "}\n",
);

const COPY_CONFIG: &str = concat!("mouse_mode true\n", "copy_clipboard \"system\"\n");

const MOUSE_CONFIG: &str = concat!("mouse_mode true\n", "advanced_mouse_actions true\n");

const MARKED_COMMAND: &[u8] = b"\x1b]133;A\x07$ \x1b]133;B\x07echo hi\x1b]133;C\x07\r\nhello world\r\nsecond line\x1b]133;D;0\x07\r\n\x1b]133;A\x07$ ";

const COMMAND_AND_OUTPUT: &str = "echo hi\nhello world\nsecond line";
const LAST_COMMAND_OUTPUT: &str = "hello world\nsecond line";

const ALT_WHEEL_UP: u8 = 72;
const ALT_WHEEL_DOWN: u8 = 73;

const SCROLL_MODE_HINT: &str = "PgDn|PgUp";

fn start_with_config(config: &str) -> TestSession {
    TestRunner::new(TERMINAL_SIZE).with_config(config).start()
}

fn claim_terminal_with(zellij: &TestSession, pane_output: &[u8], last_line: &str) -> FakePtyHandle {
    let terminal = zellij.expect_pty_spawn();
    terminal.output(pane_output);
    zellij.wait_until("marked pane output rendered", |grid_snapshot| {
        grid_snapshot.tab_bar_appears()
            && grid_snapshot.status_bar_appears()
            && grid_snapshot.contains(last_line)
    });
    terminal
}

fn marked_commands(count: usize) -> Vec<u8> {
    let mut content = String::new();
    for command_index in 0..count {
        content.push_str(&format!(
            "\u{1b}]133;A\u{7}$ \u{1b}]133;B\u{7}cmd{:02}\u{1b}]133;C\u{7}\r\nout{:02}\u{1b}]133;D;0\u{7}\r\n",
            command_index, command_index
        ));
    }
    content.into_bytes()
}

fn top_content_line(grid_snapshot: &GridSnapshot) -> String {
    grid_snapshot
        .lines()
        .get(1)
        .map(|line| line.trim_end().to_owned())
        .unwrap_or_default()
}

fn enter_scroll_mode(zellij: &TestSession) {
    zellij.send_stdin(&keys::ctrl('s'));
    zellij.wait_until("scroll mode active", |grid_snapshot| {
        grid_snapshot.contains(SCROLL_MODE_HINT)
    });
}

fn sgr_mouse_report(column: usize, line: usize, button: u8) -> Vec<u8> {
    format!("\u{1b}[<{};{};{}M", button, column, line).into_bytes()
}

fn osc52_copy_of(text: &str) -> Vec<u8> {
    format!("\u{1b}]52;c;{}\u{1b}\\", BASE64_STANDARD.encode(text)).into_bytes()
}

fn occurrences_of(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

#[test]
fn bracket_keys_jump_between_prompts_in_scroll_mode() {
    let mut zellij = TestRunner::new(TERMINAL_SIZE).start();
    claim_terminal_with(&zellij, &marked_commands(12), "out11");
    enter_scroll_mode(&zellij);

    let before_jump = top_content_line(&zellij.snapshot());

    zellij.send_stdin(&keys::key('['));
    let after_first_jump = zellij.wait_until("jumped to the previous prompt", |grid_snapshot| {
        let top_line = top_content_line(grid_snapshot);
        top_line.starts_with("$ cmd") && top_line != before_jump
    });
    let first_prompt = top_content_line(&after_first_jump);

    zellij.send_stdin(&keys::key('['));
    zellij.wait_until("jumped to an earlier prompt", |grid_snapshot| {
        let top_line = top_content_line(grid_snapshot);
        top_line.starts_with("$ cmd") && top_line != first_prompt
    });

    zellij.send_stdin(&keys::key(']'));
    zellij.wait_until("jumped forward to the later prompt", |grid_snapshot| {
        top_content_line(grid_snapshot) == first_prompt
    });

    zellij.quit();
}

#[test]
fn a_prompt_jump_from_normal_mode_does_not_enter_scroll_mode() {
    let mut zellij = start_with_config(NAVIGATION_CONFIG);
    claim_terminal_with(&zellij, &marked_commands(12), "out11");

    let before_jump = top_content_line(&zellij.snapshot());

    zellij.send_stdin(&keys::ctrl('y'));
    let after_jump = zellij.wait_until(
        "jumped to the previous prompt from normal mode",
        |grid_snapshot| {
            let top_line = top_content_line(grid_snapshot);
            top_line.starts_with("$ cmd") && top_line != before_jump
        },
    );

    assert!(
        !after_jump.contains(SCROLL_MODE_HINT),
        "a prompt jump outside scroll mode must not switch modes"
    );

    zellij.quit();
}

#[test]
fn selecting_the_command_at_the_scroll_position_composes_with_copy() {
    let mut zellij = start_with_config(NAVIGATION_CONFIG);
    claim_terminal_with(&zellij, MARKED_COMMAND, "second line");
    enter_scroll_mode(&zellij);

    zellij.send_stdin(&keys::key('m'));
    zellij.send_stdin(&keys::ctrl('y'));

    let expected_copy = osc52_copy_of(COMMAND_AND_OUTPUT);
    zellij.wait_until_raw_output("the selected command and its output were copied", |bytes| {
        occurrences_of(bytes, &expected_copy) >= 1
    });

    zellij.quit();
}

#[test]
fn copying_the_last_command_output_flashes_it_and_leaves_scroll_mode() {
    let mut zellij = start_with_config(COPY_CONFIG);
    claim_terminal_with(&zellij, MARKED_COMMAND, "second line");

    let before_copy = zellij.snapshot();
    let output_row = before_copy
        .row_of_line("hello world")
        .expect("the command output must be rendered");
    let steady_foreground = before_copy.cell_foreground(0, output_row);

    enter_scroll_mode(&zellij);
    zellij.send_stdin(&keys::key('c'));

    let expected_copy = osc52_copy_of(LAST_COMMAND_OUTPUT);
    zellij.wait_until_raw_output("the last command output was copied", |bytes| {
        occurrences_of(bytes, &expected_copy) >= 1
    });

    zellij.wait_until("the copied output is highlighted", |grid_snapshot| {
        grid_snapshot.cell_foreground(0, output_row) != steady_foreground
    });
    zellij.wait_until("scroll mode was left after the copy", |grid_snapshot| {
        !grid_snapshot.contains(SCROLL_MODE_HINT)
    });
    zellij.wait_until("the highlight is cleared again", |grid_snapshot| {
        grid_snapshot.cell_foreground(0, output_row) == steady_foreground
    });

    zellij.quit();
}

#[test]
fn alt_wheel_jumps_between_prompts() {
    let mut zellij = start_with_config(MOUSE_CONFIG);
    claim_terminal_with(&zellij, &marked_commands(12), "out11");

    let before_jump = top_content_line(&zellij.snapshot());

    zellij.send_stdin(&sgr_mouse_report(20, 5, ALT_WHEEL_UP));
    zellij.wait_until(
        "alt wheel up jumped to the previous prompt",
        |grid_snapshot| {
            let top_line = top_content_line(grid_snapshot);
            top_line.starts_with("$ cmd") && top_line != before_jump
        },
    );

    zellij.send_stdin(&sgr_mouse_report(20, 5, ALT_WHEEL_DOWN));
    zellij.wait_until("alt wheel down jumped forward again", |grid_snapshot| {
        top_content_line(grid_snapshot) == before_jump
    });

    zellij.quit();
}

const DELETE_CONFIG: &str = concat!(
    "keybinds {\n",
    " normal {\n",
    "  bind \"Ctrl l\" { DeleteLastCommand; }\n",
    " }\n",
    " scroll {\n",
    "  bind \"x\" { DeleteLastCommand; }\n",
    " }\n",
    "}\n",
);

const FORM_FEED: u8 = 12;

fn marked_commands_and_prompt(count: usize) -> Vec<u8> {
    let mut content = marked_commands(count);
    content.extend_from_slice(b"\x1b]133;A\x07$ \x1b]133;B\x07typed");
    content
}

#[test]
fn each_press_deletes_one_command_and_keeps_the_typed_input() {
    let mut zellij = start_with_config(DELETE_CONFIG);
    let terminal = claim_terminal_with(&zellij, &marked_commands_and_prompt(3), "typed");

    zellij.send_stdin(&keys::ctrl('l'));
    zellij.wait_until("the last command was deleted", |grid_snapshot| {
        !grid_snapshot.contains("cmd02")
            && !grid_snapshot.contains("out02")
            && grid_snapshot.contains("out01")
            && grid_snapshot.contains("$ typed")
    });

    zellij.send_stdin(&keys::ctrl('l'));
    zellij.wait_until("the command before it was deleted", |grid_snapshot| {
        !grid_snapshot.contains("cmd01")
            && grid_snapshot.contains("out00")
            && grid_snapshot.contains("$ typed")
    });

    zellij.send_stdin(b"z");
    let received = terminal.wait_for_stdin("the marker key", |bytes| bytes.contains(&b'z'));
    assert!(
        !received.contains(&FORM_FEED),
        "a handled delete must not reach the program"
    );

    zellij.quit();
}

#[test]
fn the_delete_key_reaches_a_running_command() {
    let mut zellij = start_with_config(DELETE_CONFIG);
    let mut content = marked_commands(2);
    content.extend_from_slice(b"\x1b]133;A\x07$ \x1b]133;B\x07runs\x1b]133;C\x07\r\nstill running");
    let terminal = claim_terminal_with(&zellij, &content, "still running");

    zellij.send_stdin(&keys::ctrl('l'));
    terminal.wait_for_stdin("the forwarded key", |bytes| bytes.contains(&FORM_FEED));
    assert!(zellij.snapshot().contains("out01"));

    zellij.quit();
}

#[test]
fn the_delete_key_reaches_a_full_screen_program() {
    let mut zellij = start_with_config(DELETE_CONFIG);
    let mut content = marked_commands_and_prompt(2);
    content.extend_from_slice(b"\x1b[?1049h\x1b[Hfull screen");
    let terminal = claim_terminal_with(&zellij, &content, "full screen");

    zellij.send_stdin(&keys::ctrl('l'));
    terminal.wait_for_stdin("the forwarded key", |bytes| bytes.contains(&FORM_FEED));

    zellij.quit();
}

#[test]
fn the_delete_key_reaches_a_shell_without_markers() {
    let mut zellij = start_with_config(DELETE_CONFIG);
    let terminal = claim_terminal_with(&zellij, b"unmarked$ ", "unmarked$");

    zellij.send_stdin(&keys::ctrl('l'));
    terminal.wait_for_stdin("the forwarded key", |bytes| bytes.contains(&FORM_FEED));

    zellij.quit();
}

#[test]
fn a_delete_key_bound_in_scroll_mode_never_reaches_the_program() {
    let mut zellij = start_with_config(DELETE_CONFIG);
    let terminal = claim_terminal_with(&zellij, b"unmarked$ ", "unmarked$");
    enter_scroll_mode(&zellij);

    zellij.send_stdin(b"x");
    zellij.send_stdin(&keys::ctrl('c'));
    zellij.wait_until("scroll mode was left", |grid_snapshot| {
        !grid_snapshot.contains(SCROLL_MODE_HINT)
    });
    zellij.send_stdin(b"z");
    let received = terminal.wait_for_stdin("the marker key", |bytes| bytes.contains(&b'z'));
    assert!(!received.contains(&b'x'));

    zellij.quit();
}
