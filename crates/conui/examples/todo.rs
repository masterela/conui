//! A todo list, and the declarative layer under load.
//!
//! Where the snake example is the canvas layer — a fixed composition drawn at absolute
//! coordinates — this one is the opposite case, and the more common one: a screen assembled from
//! nested rows and columns that has to survive being any size at all. Nothing here computes a
//! coordinate. There is one `Column` of five children, one of which is a `Row`, and layout
//! decides the rest.
//!
//! It is also the smallest app that needs state a view cannot hold: which row is selected, and
//! where the cursor is in a half-typed line. Both live in the app struct as a [`Selection`] and an
//! [`Editor`], which `List` and `Input` borrow to draw. Note what is *not* here — no scroll
//! offset arithmetic, no cursor-visibility check, no focus registry. The list scrolls because it
//! knows its own height at render time; keys go to the editor because a `match` on the mode sends
//! them there.
//!
//! The list is clickable, which is the interesting half: a row number under the pointer means
//! nothing on its own, because the same cell is a different task depending on how far the list had
//! scrolled when it was drawn. So the list records where it landed in a [`Hits`] map while
//! composing, and the click is resolved against that — last frame's geometry is the only thing that
//! can answer the question. Clicking a task's tick ticks it; clicking its text selects it.
//!
//! Tasks are read from and written to `~/.conui-todo.md` as a markdown checklist, so the file
//! stays useful — and editable — outside this program. Override the location with
//! `CONUI_TODO_FILE`.
//!
//! ```text
//! cargo run -p conui --example todo
//! cargo run -p conui --example todo -- --dump   # one composed frame as text, touching no file
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use conui::state::{Editor, Selection};
use conui::view::{Column, Row, Spacer, View, ViewExt};
use conui::widget::{
    Border, Field, Gauge, Hints, Input, List, ListRow, Panel, Readout, Rule, Stat, Text,
};
use conui::{
    App, BarStyle, Buffer, Config, Event, Frame, Hits, KeyCode, MouseEvent, Padding, Pos, Role,
    Theme,
};

/// Below this there is no room for the sidebar and the list together.
const MIN_WIDTH: u16 = 62;
const MIN_HEIGHT: u16 = 16;
/// The sidebar is fixed: figures that jump about as the window resizes are hard to read.
const SIDEBAR: u16 = 24;
/// Width of one three-digit block-figure stat, from `Stat`'s own metrics.
const STAT_WIDTH: u16 = 11;
/// Columns the full key legend needs. Below this the footer shows the short one instead.
const FULL_LEGEND: u16 = 71;

fn main() -> io::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("todo — a conui example\n");
            println!("  --dump [width] [height]   print one composed frame as text and exit");
            println!("  --help                    this\n");
            println!("Tasks live in $CONUI_TODO_FILE, or ~/.conui-todo.md.");
            Ok(())
        }
        Some("--dump") => {
            let width = arguments.get(1).and_then(|value| value.parse().ok()).unwrap_or(84);
            let height = arguments.get(2).and_then(|value| value.parse().ok()).unwrap_or(22);
            dump(width, height);
            Ok(())
        }
        _ => run(),
    }
}

fn run() -> io::Result<()> {
    let mut todo = Todo::load(data_path());
    let config = Config::new()
        .theme(Theme::LAYA)
        // No animation on this screen, but a slow tick keeps the clock-free UI responsive to a
        // resize without burning a core on a 30fps redraw of a static list.
        .fps(20)
        .min_size(MIN_WIDTH, MIN_HEIGHT)
        .mouse(true);
    let mut app = App::with(config)?;

    while app.is_running() {
        for event in app.poll()? {
            if todo.handle(&event) == Flow::Quit {
                app.quit();
            }
        }
        app.draw(|frame| compose(frame, &todo))?;
    }
    // Leaving before the final save means a crash in `leave` cannot cost the user their edits.
    app.leave()?;
    todo.save();
    Ok(())
}

/// Render one frame into a plain buffer and print it, so the layout can be checked in CI or in a
/// pipe. The same [`compose`] the real app calls: there is no second rendering path to drift.
fn dump(width: u16, height: u16) {
    let mut todo = Todo::sample();
    todo.selection.set_selected(1);
    let mut buffer = Buffer::new(width, height);
    let mut frame = Frame::new(&mut buffer, Theme::LAYA);
    compose(&mut frame, &todo);
    for row in 0..height {
        println!("{}", buffer.row_text(row).trim_end());
    }
}

// ---- The screen -------------------------------------------------------------------------

fn compose(frame: &mut Frame<'_>, todo: &Todo) {
    // Last frame's geometry is gone; a stale hit would point at where a row used to be.
    todo.hits.clear();
    let (done, total) = (todo.done_count(), todo.tasks.len());

    let heading = format!("{done} of {total} done");
    let screen = Column::new()
        .child(
            Row::new()
                .child(Text::new("CONUI  /  TODO").accent().flex(1))
                .child(Text::new(heading).muted().right().flex(1))
                .length(1),
        )
        .child(Rule::new().length(1))
        .child(
            Row::new()
                .gap(2)
                .child(
                    Panel::new(format!("TASKS · {}", todo.filter.label()))
                        .border(Border::Line)
                        .padding(Padding::xy(1, 0))
                        .child(todo.task_list().hit(&todo.hits, Zone::List))
                        .flex(1),
                )
                .child(sidebar(todo).length(SIDEBAR))
                .flex(1),
        )
        .child(Rule::new().length(1))
        .child(footer(todo, frame.size().0.saturating_sub(4)).length(1))
        .padding(Padding::xy(2, 1));

    frame.render_full(&screen);
}

/// The right-hand column: counts, progress, and where the file is.
fn sidebar(todo: &Todo) -> impl View + '_ {
    let total = todo.tasks.len();
    let done = todo.done_count();
    let open = total - done;
    // An empty list is not 100% complete, and claiming it is would be the one dishonest number
    // on the screen.
    let ratio = if total == 0 { 0.0 } else { done as f32 / total as f32 };

    // Deliberately no decorative spacer between the gauge and the figures. Every row this column
    // asks for is a row it can lose to a short terminal, and losing the gauge costs more than the
    // breathing room is worth.
    Column::new()
        .child(
            Row::new()
                .gap(2)
                .child(Stat::new("DONE", done as i64).length(STAT_WIDTH))
                .child(Stat::new("OPEN", open as i64).role(Role::Warn).length(STAT_WIDTH))
                .length(4),
        )
        .child(
            Gauge::new(ratio)
                .label("PROGRESS")
                .label_width(9)
                .style(BarStyle::Shaded)
                .readout(Readout::Percent)
                .length(1),
        )
        .child(Field::new("TOTAL", total.to_string()).value_column(9).length(1))
        .child(Field::new("SHOWN", todo.visible().len().to_string()).value_column(9).length(1))
        .child(Field::new("FILE", todo.file_label()).value_column(9).length(1))
        .child(Spacer::new().flex(1))
        .child(Text::new(todo.status.clone()).role(todo.status_role).length(1))
}

/// One row at the bottom: the key legend, or the field you are typing into.
///
/// `width` is what the footer will actually get, so the legend can shed hints rather than having
/// them silently clipped — a legend that runs off the edge hides the one key a stuck user needs,
/// because `Q quit` is always last.
fn footer(todo: &Todo, width: u16) -> Box<dyn View + '_> {
    match todo.mode {
        Mode::Browse if width < FULL_LEGEND => Box::new(
            Hints::new()
                .emphasise_keys()
                .spacing(2)
                .key("↑/↓", "move")
                .key("SPACE", "toggle")
                .key("A", "add")
                .key("Q", "quit"),
        ),
        Mode::Browse => Box::new(
            // Two columns of spacing rather than three: the full legend only just fits.
            Hints::new()
                .emphasise_keys()
                .spacing(2)
                .key("↑/↓", "move")
                .key("SPACE", "toggle")
                .key("A", "add")
                .key("E", "edit")
                .key("D", "del")
                .key("F", "filter")
                .key("C", "clear")
                .key("Q", "quit"),
        ),
        Mode::Adding => Box::new(
            Input::new(&todo.editor)
                .prompt("+")
                .placeholder("new task — ENTER to add, ESC to cancel"),
        ),
        Mode::Editing(_) => Box::new(
            Input::new(&todo.editor).prompt("✎").placeholder("ENTER to save, ESC to discard"),
        ),
    }
}

// ---- State ------------------------------------------------------------------------------

struct Task {
    text: String,
    done: bool,
}

/// Which tasks the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Open,
    Done,
}

impl Filter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Open,
            Self::Open => Self::Done,
            Self::Done => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Open => "OPEN",
            Self::Done => "DONE",
        }
    }

    fn accepts(self, task: &Task) -> bool {
        match self {
            Self::All => true,
            Self::Open => !task.done,
            Self::Done => task.done,
        }
    }
}

/// What the keyboard is currently doing.
enum Mode {
    Browse,
    Adding,
    /// Editing the task at this index of `tasks`, held rather than re-derived so that a filter
    /// change mid-edit cannot retarget the write.
    Editing(usize),
}

/// Whether the event loop should keep going.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

/// The parts of the screen a click can land in.
///
/// One variant, because there is one thing here worth clicking. A [`Hits`] map is still the right
/// shape for it: the alternative is a bespoke `Cell<Rect>` per target, which is what this is
/// underneath and what stops scaling at two. The settings example has eight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    List,
}

struct Todo {
    tasks: Vec<Task>,
    filter: Filter,
    selection: Selection,
    editor: Editor,
    mode: Mode,
    path: Option<PathBuf>,
    status: String,
    status_role: Role,
    /// Where the list drew itself last frame, so a click can be turned into a row.
    hits: Hits<Zone>,
}

impl Todo {
    fn load(path: Option<PathBuf>) -> Self {
        let tasks = path.as_deref().map(read_tasks).unwrap_or_default();
        let status = match &path {
            Some(_) if tasks.is_empty() => String::new(),
            Some(_) => format!("{} loaded", tasks.len()),
            None => "no HOME: changes will not be saved".to_string(),
        };
        let status_role = if path.is_some() { Role::Muted } else { Role::Warn };
        Self {
            tasks,
            filter: Filter::All,
            selection: Selection::new(),
            editor: Editor::new(),
            mode: Mode::Browse,
            path,
            status,
            status_role,
            hits: Hits::new(),
        }
    }

    /// A fixed set of tasks for `--dump`, so the output is reproducible and no file is read.
    fn sample() -> Self {
        let tasks = [
            ("wire the focus layer", true),
            ("port the laya snake UI", true),
            ("ship List and Input", true),
            ("verify the windows backend on windows", false),
            ("set up CI for linux and windows", false),
            ("publish 0.1 to crates.io", false),
        ];
        Self {
            tasks: tasks
                .into_iter()
                .map(|(text, done)| Task { text: text.to_string(), done })
                .collect(),
            filter: Filter::All,
            selection: Selection::new(),
            editor: Editor::new(),
            mode: Mode::Browse,
            path: None,
            status: "6 loaded".to_string(),
            status_role: Role::Muted,
            hits: Hits::new(),
        }
    }

    /// Indices into `tasks` that the filter lets through, in order. The list's own row numbers
    /// index *this*, which is why every mutation goes back through it.
    fn visible(&self) -> Vec<usize> {
        self.tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| self.filter.accepts(task))
            .map(|(index, _)| index)
            .collect()
    }

    fn selected(&self) -> Option<usize> {
        self.visible().get(self.selection.selected()).copied()
    }

    fn done_count(&self) -> usize {
        self.tasks.iter().filter(|task| task.done).count()
    }

    /// The task list, built once and used twice: composed into the panel, and asked by the click
    /// handler where a row's text begins. Two constructions would drift the first time a mark
    /// changed width, and the drift would show up as a click ticking a task the user meant to
    /// select.
    fn task_list(&self) -> List<'_> {
        let rows: Vec<ListRow> = self
            .visible()
            .iter()
            .map(|&index| {
                let task = &self.tasks[index];
                if task.done {
                    // The tick carries the state as well as the colour does: a dim row alone is
                    // not something a colourblind user or a monochrome terminal can read.
                    ListRow::new(task.text.clone()).mark("✓", Role::Accent).role(Role::Dim)
                } else {
                    ListRow::new(task.text.clone()).mark("·", Role::Muted)
                }
            })
            .collect();

        List::new(rows).selection(&self.selection).highlight().empty(match self.filter {
            Filter::All => "nothing here yet — press A to add a task",
            Filter::Open => "no open tasks. everything is done",
            Filter::Done => "nothing finished yet",
        })
    }

    fn file_label(&self) -> String {
        match &self.path {
            Some(path) => path
                .file_name()
                .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into()),
            None => "—".to_string(),
        }
    }

    // ---- Input ------------------------------------------------------------------------

    fn handle(&mut self, event: &Event) -> Flow {
        match event {
            Event::Key(key) => match self.mode {
                Mode::Browse => return self.browse(key),
                _ => self.edit(key),
            },
            // Clicks and the wheel only mean something while browsing. Mid-edit the list is not
            // what the user is talking to, and quietly moving the selection under an open field
            // would commit the text to a different row than the one they were looking at.
            Event::Mouse(mouse) if matches!(self.mode, Mode::Browse) => self.point(mouse),
            // A paste into the field is text; a paste while browsing would be a stream of
            // shortcuts firing at once, which is never what the user meant.
            Event::Paste(text) if !matches!(self.mode, Mode::Browse) => {
                self.editor.insert_str(text);
            }
            _ => {}
        }
        Flow::Continue
    }

    fn browse(&mut self, key: &conui::KeyEvent) -> Flow {
        let length = self.visible().len();
        match key.code {
            KeyCode::Char('q' | 'Q') | KeyCode::Escape => return Flow::Quit,
            KeyCode::Up => self.selection.up(),
            KeyCode::Down => self.selection.down(length),
            KeyCode::Char('k') => self.selection.up(),
            KeyCode::Char('j') => self.selection.down(length),
            KeyCode::PageUp => self.selection.page_up(10),
            KeyCode::PageDown => self.selection.page_down(10, length),
            KeyCode::Home | KeyCode::Char('g') => self.selection.first(),
            KeyCode::End | KeyCode::Char('G') => self.selection.last(length),
            KeyCode::Char(' ' | '\t') | KeyCode::Enter => self.toggle(),
            KeyCode::Char('a' | 'A') => {
                self.editor.clear();
                self.mode = Mode::Adding;
            }
            KeyCode::Char('e' | 'E') => {
                if let Some(index) = self.selected() {
                    self.editor.set_value(self.tasks[index].text.clone());
                    self.mode = Mode::Editing(index);
                }
            }
            KeyCode::Char('d' | 'D') | KeyCode::Delete => self.remove(),
            KeyCode::Char('f' | 'F') => {
                self.filter = self.filter.next();
                // The row under the cursor has almost certainly changed identity; clamping at
                // least keeps it on a row that exists.
                self.selection.clamp(self.visible().len());
                self.note(format!("filter: {}", self.filter.label()), Role::Muted);
            }
            KeyCode::Char('c' | 'C') => self.clear_done(),
            _ => {}
        }
        Flow::Continue
    }

    /// Point at the list: the wheel scrolls it, a click picks a row, and a click on a row's mark
    /// ticks it.
    ///
    /// Everything here goes through the region the list recorded while drawing, because a row
    /// number on its own is meaningless: the same cell is a different task depending on how far
    /// the list had scrolled when the user was looking at it.
    fn point(&mut self, mouse: &MouseEvent) {
        let at = Pos::new(mouse.column, mouse.row);
        let Some(area) = self.hits.area_of(Zone::List) else { return };
        let length = self.visible().len();

        if let Some(delta) = mouse.kind.scroll() {
            // One row per notch, and only over the list. The cursor moves because the list's
            // window follows it — see `Selection::step` — and moving the cursor is harmless here:
            // nothing is written until SPACE or a click on a tick.
            if area.contains(at) {
                self.selection.step(delta, length);
            }
            return;
        }
        if !mouse.is_click() {
            return;
        }
        let Some(row) = self.selection.row_at(area, at, length) else { return };
        // Asked before the selection moves, because `text_column` depends on the marks the list
        // was drawn with, and one of those is about to change.
        let on_mark = self
            .hits
            .local(Zone::List, at)
            .is_some_and(|local| local.x < self.task_list().text_column());
        self.selection.set_selected(row);
        if on_mark {
            self.toggle();
        }
    }

    fn edit(&mut self, key: &conui::KeyEvent) {
        match key.code {
            KeyCode::Escape => {
                self.mode = Mode::Browse;
                self.editor.clear();
                self.note("cancelled", Role::Muted);
            }
            KeyCode::Enter => self.commit(),
            // Everything else the editor recognises is text; what it declines, we ignore.
            _ => {
                self.editor.handle(key);
            }
        }
    }

    fn commit(&mut self) {
        let text = self.editor.take().trim().to_string();
        let mode = std::mem::replace(&mut self.mode, Mode::Browse);
        if text.is_empty() {
            self.note("empty — nothing saved", Role::Warn);
            return;
        }
        match mode {
            Mode::Adding => {
                self.tasks.push(Task { text, done: false });
                // Put the cursor on what was just added, wherever the filter placed it.
                let index = self.tasks.len() - 1;
                if let Some(row) = self.visible().iter().position(|&task| task == index) {
                    self.selection.set_selected(row);
                }
                self.note("added", Role::Accent);
            }
            Mode::Editing(index) => {
                self.tasks[index].text = text;
                self.note("saved", Role::Accent);
            }
            Mode::Browse => {}
        }
        self.save();
    }

    fn toggle(&mut self) {
        let Some(index) = self.selected() else { return };
        self.tasks[index].done = !self.tasks[index].done;
        let done = self.tasks[index].done;
        // Under a filter the row has just disqualified itself and vanished from under the
        // cursor, so the selection has to be pulled back inside what remains.
        self.selection.clamp(self.visible().len());
        self.note(if done { "done" } else { "reopened" }, Role::Muted);
        self.save();
    }

    fn remove(&mut self) {
        let Some(index) = self.selected() else { return };
        let row = self.selection.selected();
        self.tasks.remove(index);
        self.selection.removed(row, self.visible().len());
        self.note("deleted", Role::Danger);
        self.save();
    }

    fn clear_done(&mut self) {
        let before = self.tasks.len();
        self.tasks.retain(|task| !task.done);
        let removed = before - self.tasks.len();
        if removed == 0 {
            self.note("nothing to clear", Role::Muted);
            return;
        }
        self.selection.clamp(self.visible().len());
        self.note(format!("cleared {removed}"), Role::Danger);
        self.save();
    }

    fn note(&mut self, status: impl Into<String>, role: Role) {
        self.status = status.into();
        self.status_role = role;
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else { return };
        if let Err(error) = write_tasks(&path, &self.tasks) {
            // A failed save is the one thing this app must never do quietly.
            self.note(format!("save failed: {error}"), Role::Danger);
        }
    }
}

// ---- The file ---------------------------------------------------------------------------

fn data_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CONUI_TODO_FILE") {
        return Some(PathBuf::from(explicit));
    }
    // `USERPROFILE` is the Windows spelling of the same idea, and a PowerShell session has no
    // `HOME` — without it the example would quietly run with nowhere to save to.
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".conui-todo.md"))
}

fn read_tasks(path: &Path) -> Vec<Task> {
    // A missing file is the first run, not an error.
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    text.lines().filter_map(parse_task).collect()
}

/// Parse one markdown checklist item. Anything else in the file — a heading, a blank line, a
/// note to self — is left alone on read and dropped on write, which is the honest tradeoff for a
/// format you can hand-edit.
fn parse_task(line: &str) -> Option<Task> {
    let item = line.trim().strip_prefix("- ").or_else(|| line.trim().strip_prefix("* "))?;
    let (done, text) = match item.as_bytes().first()? {
        b'[' if item.len() >= 3 => {
            let state = &item[1..2];
            let rest = &item[3..];
            match state {
                "x" | "X" => (true, rest),
                " " => (false, rest),
                _ => return None,
            }
        }
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| Task { text: text.to_string(), done })
}

fn write_tasks(path: &Path, tasks: &[Task]) -> io::Result<()> {
    let mut body = String::from("# todo\n\n");
    for task in tasks {
        body.push_str(if task.done { "- [x] " } else { "- [ ] " });
        body.push_str(&task.text);
        body.push('\n');
    }
    // Write beside the target and rename over it: a crash or a full disk mid-write then costs
    // the new version rather than the old one.
    let temporary = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name().map_or("conui-todo".into(), |name| name.to_string_lossy())
    ));
    fs::write(&temporary, body)?;
    fs::rename(&temporary, path)
}

// ---- Tests ------------------------------------------------------------------------------

/// An app built this way is testable without a terminal, which is the point of a [`Frame`]
/// owning nothing but a buffer. These drive the real key handler and assert on real state; the
/// rendering ones assert on the text of a rendered buffer. Run with:
///
/// ```text
/// cargo test -p conui --example todo
/// ```
#[cfg(test)]
mod tests {
    use super::*;
    use conui::{KeyEvent, Modifiers, MouseButton, MouseKind, Rect};

    /// A sample app with no path, so nothing here can touch the filesystem.
    fn app() -> Todo {
        Todo::sample()
    }

    fn press(todo: &mut Todo, code: KeyCode) -> Flow {
        todo.handle(&Event::Key(KeyEvent::plain(code)))
    }

    fn type_text(todo: &mut Todo, text: &str) {
        for character in text.chars() {
            press(todo, KeyCode::Char(character));
        }
    }

    fn screen(todo: &Todo, width: u16, height: u16) -> String {
        let mut buffer = Buffer::new(width, height);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        compose(&mut frame, todo);
        (0..height).map(|row| buffer.row_text(row)).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_task_can_be_added_from_the_keyboard() {
        let mut todo = app();
        press(&mut todo, KeyCode::Char('a'));
        type_text(&mut todo, "buy milk");
        press(&mut todo, KeyCode::Enter);
        assert_eq!(todo.tasks.last().map(|task| task.text.as_str()), Some("buy milk"));
        assert!(matches!(todo.mode, Mode::Browse), "committing returns to browsing");
        assert_eq!(todo.selected(), Some(todo.tasks.len() - 1), "the cursor follows the new task");
    }

    #[test]
    fn an_empty_add_is_refused_rather_than_creating_a_blank_row() {
        let mut todo = app();
        let before = todo.tasks.len();
        press(&mut todo, KeyCode::Char('a'));
        type_text(&mut todo, "   ");
        press(&mut todo, KeyCode::Enter);
        assert_eq!(todo.tasks.len(), before);
        assert_eq!(todo.status, "empty — nothing saved");
    }

    #[test]
    fn escape_abandons_what_was_typed() {
        let mut todo = app();
        let before = todo.tasks.len();
        press(&mut todo, KeyCode::Char('a'));
        type_text(&mut todo, "never mind");
        press(&mut todo, KeyCode::Escape);
        assert_eq!(todo.tasks.len(), before);
        assert!(todo.editor.is_empty());
        assert!(matches!(todo.mode, Mode::Browse));
    }

    #[test]
    fn escape_while_browsing_quits_but_while_typing_does_not() {
        let mut todo = app();
        press(&mut todo, KeyCode::Char('a'));
        assert_eq!(press(&mut todo, KeyCode::Escape), Flow::Continue, "cancels the add");
        assert_eq!(press(&mut todo, KeyCode::Escape), Flow::Quit, "then quits");
    }

    #[test]
    fn space_toggles_the_selected_task() {
        let mut todo = app();
        todo.selection.set_selected(3);
        assert!(!todo.tasks[3].done);
        press(&mut todo, KeyCode::Char(' '));
        assert!(todo.tasks[3].done);
        press(&mut todo, KeyCode::Char(' '));
        assert!(!todo.tasks[3].done);
    }

    #[test]
    fn editing_rewrites_the_selected_task_in_place() {
        let mut todo = app();
        todo.selection.set_selected(4);
        press(&mut todo, KeyCode::Char('e'));
        assert_eq!(
            todo.editor.value(),
            "set up CI for linux and windows",
            "the field starts prefilled"
        );
        for _ in 0.."windows".len() {
            press(&mut todo, KeyCode::Backspace);
        }
        type_text(&mut todo, "powershell");
        press(&mut todo, KeyCode::Enter);
        assert_eq!(todo.tasks[4].text, "set up CI for linux and powershell");
        assert_eq!(todo.tasks.len(), 6, "editing must not add a row");
    }

    #[test]
    fn deleting_the_last_row_steps_the_cursor_back_instead_of_off_the_end() {
        let mut todo = app();
        todo.selection.last(todo.visible().len());
        press(&mut todo, KeyCode::Char('d'));
        assert_eq!(todo.tasks.len(), 5);
        assert_eq!(todo.selection.selected(), 4);
        assert!(todo.selected().is_some(), "the cursor still points at a real task");
    }

    #[test]
    fn a_filter_hides_rows_and_the_cursor_stays_inside_what_is_left() {
        let mut todo = app();
        todo.selection.set_selected(5);
        press(&mut todo, KeyCode::Char('f')); // ALL -> OPEN
        assert_eq!(todo.filter, Filter::Open);
        assert_eq!(todo.visible().len(), 3);
        assert!(todo.selection.selected() < 3, "clamped into the shorter list");
        press(&mut todo, KeyCode::Char('f')); // -> DONE
        press(&mut todo, KeyCode::Char('f')); // -> ALL
        assert_eq!(todo.filter, Filter::All);
    }

    #[test]
    fn toggling_a_row_out_from_under_a_filter_keeps_the_cursor_valid() {
        let mut todo = app();
        press(&mut todo, KeyCode::Char('f')); // show open only
        todo.selection.last(todo.visible().len());
        press(&mut todo, KeyCode::Char(' ')); // the selected row is now done, so it vanishes
        assert_eq!(todo.visible().len(), 2);
        assert!(todo.selected().is_some(), "cursor must not dangle past the end");
    }

    #[test]
    fn clearing_removes_only_completed_tasks() {
        let mut todo = app();
        press(&mut todo, KeyCode::Char('c'));
        assert_eq!(todo.tasks.len(), 3);
        assert!(todo.tasks.iter().all(|task| !task.done));
        press(&mut todo, KeyCode::Char('c'));
        assert_eq!(todo.status, "nothing to clear");
    }

    #[test]
    fn a_paste_is_text_in_a_field_and_ignored_while_browsing() {
        let mut todo = app();
        todo.handle(&Event::Paste("pasted".to_string()));
        assert!(todo.editor.is_empty(), "browsing ignores a paste");
        press(&mut todo, KeyCode::Char('a'));
        todo.handle(&Event::Paste("pasted".to_string()));
        assert_eq!(todo.editor.value(), "pasted");
    }

    #[test]
    fn the_footer_shows_the_field_while_typing_and_the_legend_otherwise() {
        let mut todo = app();
        assert!(screen(&todo, 84, 22).contains("Q quit"));
        press(&mut todo, KeyCode::Char('a'));
        type_text(&mut todo, "half typed");
        let rendered = screen(&todo, 84, 22);
        assert!(rendered.contains("+ half typed"), "got {rendered}");
        assert!(!rendered.contains("Q quit"), "the legend gives way to the field");
    }

    #[test]
    fn the_list_scrolls_rather_than_overflowing_a_short_window() {
        let mut todo = app();
        todo.selection.last(todo.visible().len());
        // At this height the panel leaves four rows for six tasks, so with the selection on the
        // last one the window has to have slid: the last task shows and the first cannot.
        let rendered = screen(&todo, 84, 12);
        assert!(rendered.contains("publish 0.1 to crates.io"), "got {rendered}");
        assert!(!rendered.contains("wire the focus layer"), "got {rendered}");
    }

    #[test]
    fn an_empty_list_explains_itself_instead_of_showing_a_blank_panel() {
        let mut todo = app();
        todo.tasks.clear();
        assert!(screen(&todo, 84, 22).contains("press A to add a task"));
    }

    #[test]
    fn progress_of_an_empty_list_is_zero_not_complete() {
        let mut todo = app();
        todo.tasks.clear();
        let rendered = screen(&todo, 84, 22);
        assert!(rendered.contains("0%"), "got {rendered}");
        assert!(!rendered.contains("100%"));
    }

    // ---- Mouse ---------------------------------------------------------------------------

    /// Draw at this size, then click. Both halves matter: a click is resolved against last frame's
    /// geometry, so a test that skipped the draw would be clicking at a screen that never existed.
    fn click_at(todo: &mut Todo, column: u16, row: u16, height: u16) {
        let _ = screen(todo, 84, height);
        todo.handle(&Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: Modifiers::NONE,
        }));
    }

    /// Where the list drew itself, after one frame at this size.
    fn list_area(todo: &Todo, height: u16) -> Rect {
        let _ = screen(todo, 84, height);
        todo.hits.area_of(Zone::List).expect("the list draws every frame")
    }

    fn wheel(todo: &mut Todo, column: u16, row: u16, down: bool) {
        todo.handle(&Event::Mouse(MouseEvent {
            kind: if down { MouseKind::ScrollDown } else { MouseKind::ScrollUp },
            column,
            row,
            modifiers: Modifiers::NONE,
        }));
    }

    #[test]
    fn clicking_a_task_selects_it() {
        let mut todo = app();
        let area = list_area(&todo, 22);
        let text = todo.task_list().text_column();
        click_at(&mut todo, area.x + text + 1, area.y + 3, 22);
        assert_eq!(todo.selection.selected(), 3);
        assert_eq!(todo.selected(), Some(3));
        assert!(!todo.tasks[3].done, "selecting is not toggling");
    }

    #[test]
    fn clicking_a_tasks_tick_toggles_it() {
        let mut todo = app();
        let area = list_area(&todo, 22);
        assert!(!todo.tasks[4].done);
        // Left of the text is the status column, and clicking a tick is how you tick it.
        click_at(&mut todo, area.x, area.y + 4, 22);
        assert!(todo.tasks[4].done);
        assert_eq!(todo.selection.selected(), 4, "and the cursor followed the click");
    }

    #[test]
    fn clicking_past_the_last_task_changes_nothing() {
        let mut todo = app();
        let area = list_area(&todo, 22);
        todo.selection.set_selected(1);
        // Six tasks, so row six is the blank space under them.
        click_at(&mut todo, area.x + 6, area.y + 6, 22);
        assert_eq!(todo.selection.selected(), 1, "the blank space belongs to nobody");
    }

    #[test]
    fn the_wheel_moves_the_cursor_and_the_window_follows_it() {
        let mut todo = app();
        let area = list_area(&todo, 12);
        assert_eq!(area.height, 4, "the fixture assumes a four-row window over six tasks");
        for _ in 0..5 {
            wheel(&mut todo, area.x + 4, area.y + 1, true);
        }
        assert_eq!(todo.selection.selected(), 5, "five notches, five rows, then the end");
        let rendered = screen(&todo, 84, 12);
        assert!(rendered.contains("publish 0.1 to crates.io"), "the window slid: {rendered}");
        assert!(!rendered.contains("wire the focus layer"), "got {rendered}");
        wheel(&mut todo, area.x + 4, area.y + 1, false);
        assert_eq!(todo.selection.selected(), 4);
    }

    #[test]
    fn the_wheel_away_from_the_list_is_not_the_lists_business() {
        let mut todo = app();
        let _ = screen(&todo, 84, 22);
        wheel(&mut todo, 70, 5, true); // over the sidebar
        assert_eq!(todo.selection.selected(), 0);
    }

    #[test]
    fn a_click_after_scrolling_lands_on_the_row_it_looks_like() {
        let mut todo = app();
        // Four visible rows for six tasks. Put the cursor at the end so the window has slid by two,
        // then click the top row: it is task two now, not task zero.
        todo.selection.last(todo.visible().len());
        let area = list_area(&todo, 12);
        assert_eq!(area.height, 4, "the fixture assumes a four-row window");
        click_at(&mut todo, area.x + 6, area.y, 12);
        assert_eq!(todo.selection.selected(), 2);
        assert_eq!(todo.tasks[todo.selected().unwrap()].text, "ship List and Input");
    }

    #[test]
    fn a_click_outside_the_list_is_ignored() {
        let mut todo = app();
        todo.selection.set_selected(2);
        // The sidebar, which has nothing clickable on it.
        click_at(&mut todo, 70, 5, 22);
        assert_eq!(todo.selection.selected(), 2);
    }

    #[test]
    fn the_mouse_is_left_alone_while_typing() {
        let mut todo = app();
        let area = list_area(&todo, 22);
        press(&mut todo, KeyCode::Char('a'));
        type_text(&mut todo, "half typed");
        click_at(&mut todo, area.x, area.y + 4, 22);
        assert_eq!(todo.selection.selected(), 0, "the list is not what the user is talking to");
        assert!(!todo.tasks[4].done);
        assert_eq!(todo.editor.value(), "half typed", "and the field kept what was in it");
    }

    // ---- The file format ----------------------------------------------------------------

    #[test]
    fn a_markdown_checklist_round_trips() {
        let tasks = [
            Task { text: "open".to_string(), done: false },
            Task { text: "closed".to_string(), done: true },
        ];
        let mut body = String::new();
        for task in &tasks {
            body.push_str(if task.done { "- [x] " } else { "- [ ] " });
            body.push_str(&task.text);
            body.push('\n');
        }
        let parsed: Vec<Task> = body.lines().filter_map(parse_task).collect();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "open");
        assert!(!parsed[0].done);
        assert!(parsed[1].done);
    }

    #[test]
    fn prose_in_the_file_is_skipped_rather_than_imported_as_tasks() {
        let text =
            "# todo\n\nsome notes to self\n- [ ] real\n* [x] also real\n- [?] not a task\n- \n";
        let parsed: Vec<Task> = text.lines().filter_map(parse_task).collect();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "real");
        assert_eq!(parsed[1].text, "also real");
    }

    #[test]
    fn a_missing_file_is_an_empty_list_not_an_error() {
        assert!(read_tasks(Path::new("/nonexistent/conui/todo.md")).is_empty());
    }
}
