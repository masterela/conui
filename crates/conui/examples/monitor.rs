//! A process monitor, and the first example whose data comes from outside it.
//!
//! The other three own everything on their screens: snake moves a snake, todo keeps a list,
//! settings edits its own fields. Nothing under them changes unless a key is pressed, which makes
//! them good demonstrations and weak evidence. This one reads the machine once a second, and the
//! machine does not care where the cursor is.
//!
//! That is the problem worth showing. A [`Selection`] holds a row *index*, and a row index is a
//! claim about an ordering — so the moment a sample comes back with a process in a different place,
//! the cursor is on something the user never chose. Sorting by memory instead of CPU does the same
//! thing, and so does typing into the filter. The fix is to keep the identity rather than the
//! position: this app remembers a pid in `focus` and re-derives the index after every change to the
//! list, which is [`Monitor::follow`]. The selection becomes a *view* of the focus, recomputed, and
//! stops being state anyone has to keep in step.
//!
//! The second thing here that the other examples never meet is a reading nobody can take. CPU
//! percentage is not a value a machine holds — it is the difference between two readings of
//! cumulative CPU time — so it does not exist until the second sample, and on a platform with no
//! sampler for it, not at all. Every figure on this screen is therefore an `Option`, and prints `—`
//! rather than `0.0` when the answer is unknown: zero is a claim, and calling a busy process idle is
//! worse than admitting to not knowing.
//!
//! Where the numbers come from, which is deliberately the least interesting code in the file —
//! conui draws screens, and reading a machine is not its job:
//!
//! | | processes | memory | uptime | CPU |
//! |---|---|---|---|---|
//! | macOS | `ps -Ao pid,rss,time,state,comm` | `vm_stat` | `sysctl kern.boottime` | yes |
//! | Linux | `/proc/<pid>/stat` | `/proc/meminfo` | `/proc/uptime` | yes |
//! | Windows | `tasklist /nh /fo csv` | — | — | no |
//!
//! ```text
//! cargo run -p conui --example monitor
//! cargo run -p conui --example monitor -- --dump   # one frame from a fixture, touching nothing
//! ```
//!
//! There is deliberately no key that kills a process. An example is something people run to see
//! what it looks like, and one keystroke from stopping their window server is not a thing to hand
//! out with a UI toolkit.

use std::collections::HashMap;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use conui::state::{Editor, Selection};
use conui::view::{Column, Row, Spacer, View, ViewExt};
use conui::widget::{
    Field, Gauge, Hints, Input, List, ListRow, Panel, Readout, Rule, Scrollbar, Sparkline, Text,
};
use conui::{
    App, BarStyle, Buffer, Config, Event, Frame, Hits, KeyCode, KeyEvent, MouseButton, MouseEvent,
    MouseKind, Padding, Pos, Role, Theme,
};

/// Below this the table has no room for its columns.
const MIN_WIDTH: u16 = 54;
const MIN_HEIGHT: u16 = 14;
/// The detail column's width, fixed so that figures do not move about as the window resizes.
const DETAIL: u16 = 26;
/// Narrower than this and the table keeps the whole width: twenty columns of process name are
/// worth more than a detail pane too narrow to read.
const DETAIL_FROM: u16 = 96;
/// Columns the full key legend needs. Below this the footer shows the short one.
const FULL_LEGEND: u16 = 79;
/// How often the machine is asked. Much faster than this and the reading is mostly the cost of
/// taking it, since a sample shells out to `ps` or walks a few hundred files.
const REFRESH: Duration = Duration::from_millis(1000);
/// Readings kept for the sparklines — one a second, so a couple of minutes. Wider than any terminal
/// on purpose: a [`Sparkline`] draws the newest columns that fit and drops the rest, so a wide
/// window simply reaches further back.
const HISTORY: usize = 140;

/// The table's fixed columns. Heading and rows are formatted from these same three numbers, so a
/// heading cannot come to sit over the wrong column.
const PID_WIDTH: usize = 7;
const CPU_WIDTH: usize = 6;
const MEM_WIDTH: usize = 9;

/// What a figure the platform would not give prints as.
const UNKNOWN: &str = "—";

/// Returning `io::Result` from `main` would be shorter, and would print the error with `Debug`:
/// `Error: Custom { kind: Unsupported, error: "conui needs .." }`. The sentence inside it is the
/// part the user needs, so it gets printed on its own.
fn main() -> ExitCode {
    match dispatch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("monitor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch() -> io::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("monitor — a conui example\n");
            println!("  --dump [width] [height]   print one composed frame as text and exit");
            println!("  --help                    this\n");
            println!("Reads the machine once a second. Nothing is written anywhere.");
            Ok(())
        }
        Some("--dump") => {
            let width = arguments.get(1).and_then(|value| value.parse().ok()).unwrap_or(100);
            let height = arguments.get(2).and_then(|value| value.parse().ok()).unwrap_or(24);
            dump(width, height);
            Ok(())
        }
        _ => run(),
    }
}

fn run() -> io::Result<()> {
    let mut monitor = Monitor::new();
    let config = Config::new()
        .theme(Theme::LAYA)
        // Far faster than the sample rate, and on purpose: the frames in between are what keep a
        // keypress from waiting the best part of a second to be noticed.
        .fps(30)
        .min_size(MIN_WIDTH, MIN_HEIGHT)
        .mouse(true);
    let mut app = App::with(config)?;

    while app.is_running() {
        for event in app.poll()? {
            if monitor.handle(&event) == Flow::Quit {
                app.quit();
            }
        }
        let now = Instant::now();
        if monitor.is_due(now) {
            monitor.refresh(now);
        }
        app.draw(|frame| compose(frame, &monitor))?;
    }
    app.leave()
}

/// Render one frame into a plain buffer and print it. Uses the fixture rather than the machine, so
/// the output is the same on every run and on every platform: the point of a dump is to check the
/// layout, and a layout checked against live figures is checked against nothing.
fn dump(width: u16, height: u16) {
    let mut monitor = Monitor::fixture();
    monitor.select(2);
    let mut buffer = Buffer::new(width, height);
    let mut frame = Frame::new(&mut buffer, Theme::LAYA);
    compose(&mut frame, &monitor);
    // One write rather than a `println!` per row, because `println!` unwraps: `--dump | head -3`
    // would abort with `Broken pipe`, and a dump prints text precisely so it can be piped. A
    // reader that stops reading is a normal end to this, not a failure.
    let text: String =
        (0..height).map(|row| format!("{}\n", buffer.row_text(row).trim_end())).collect();
    let _ = io::stdout().write_all(text.as_bytes());
}

// ---- The screen -------------------------------------------------------------------------

fn compose(frame: &mut Frame<'_>, monitor: &Monitor) {
    // Last frame's geometry is gone. A stale row would resolve a click against an ordering that has
    // since been re-sorted, which on this screen means pointing at the wrong process.
    monitor.hits.clear();
    let (width, _) = frame.size();
    let detail = width >= DETAIL_FROM;

    let screen = Column::new()
        .child(heading(monitor).length(1))
        .child(Rule::new().length(1))
        .child(meters(monitor).length(2))
        .child(Spacer::new().length(1))
        .child(
            Row::new()
                .gap(2)
                .child(table(monitor).flex(1))
                .children(if detail { vec![sidebar(monitor).length(DETAIL)] } else { Vec::new() })
                .flex(1),
        )
        .child(Rule::new().length(1))
        .child(footer(monitor, width.saturating_sub(4)).length(1))
        .padding(Padding::xy(2, 1));

    frame.render_full(&screen);
}

/// The title row: what this is on the left, which machine and for how long on the right.
fn heading(monitor: &Monitor) -> impl View + '_ {
    let machine = match monitor.snapshot.uptime {
        Some(uptime) => format!("{} · up {}", monitor.host, duration(uptime)),
        None => monitor.host.clone(),
    };
    Row::new()
        .gap(2)
        .child(Text::new("CONUI  /  MONITOR").accent())
        .children(match monitor.paused {
            true => vec![Text::new("PAUSED").role(Role::Warn)],
            false => Vec::new(),
        })
        .child(Text::new(machine).muted().right().flex(1))
}

/// Two meters side by side, each a chart over its own bar, so that the shape of the last minute and
/// the level right now are one glance apart.
fn meters(monitor: &Monitor) -> impl View + '_ {
    let snapshot = &monitor.snapshot;
    let memory = match (snapshot.memory_used, snapshot.memory_total) {
        (Some(used), Some(total)) => format!("{} / {}", bytes(used), bytes(total)),
        (Some(used), None) => bytes(used),
        _ => UNKNOWN.to_string(),
    };
    Row::new()
        .gap(3)
        .child(
            meter("CPU", &monitor.cpu_history, snapshot.cpu, percent(snapshot.cpu), Role::Accent)
                .flex(1),
        )
        .child(
            meter("MEM", &monitor.memory_history, snapshot.memory_fraction(), memory, Role::Info)
                .flex(1),
        )
}

/// One meter: a label, a chart and a readout, over a bar that lines up under the chart.
///
/// The label and the readout are the row's children rather than the [`Gauge`]'s own, which is what
/// keeps the two rows aligned: both reserve the same columns, so the bar starts where the chart
/// starts and ends where it ends.
fn meter<'a>(
    label: &'a str,
    history: &'a [f32],
    value: Option<f32>,
    readout: String,
    role: Role,
) -> impl View + 'a {
    const LABEL: u16 = 4;
    const READOUT: u16 = 15;
    Column::new()
        .child(
            Row::new()
                .gap(1)
                .child(Text::new(label).muted().length(LABEL))
                .child(Sparkline::new(history.iter().copied()).max(1.0).role(role).flex(1))
                .child(Text::new(readout).right().length(READOUT))
                .length(1),
        )
        .child(
            Row::new()
                .gap(1)
                .child(Spacer::new().length(LABEL))
                // Nothing is known about the level yet, so there is no length to draw a bar to. An
                // empty track would read as zero.
                .child(match value {
                    // `Shaded` rather than the quieter `Rule`, whose fill and track are the same
                    // glyph in different colours: that reads well on a terminal and not at all in
                    // `--dump`, where the level is the thing being checked.
                    Some(value) => Box::new(
                        Gauge::new(value)
                            .role(role)
                            .style(BarStyle::Shaded)
                            .readout(Readout::None)
                            .flex(1),
                    ) as Box<dyn View>,
                    None => Box::new(Text::new("no reading").dim().flex(1)),
                })
                .child(Spacer::new().length(READOUT))
                .length(1),
        )
}

/// The process table: a heading over a list, with a bar down the right when it overflows.
fn table(monitor: &Monitor) -> impl View + '_ {
    let list = monitor.process_list();
    let indent = list.text_column();
    let shown = list.len();
    let title = format!(
        "PROCESSES · {} {}",
        monitor.order.label(),
        if monitor.descending { "↓" } else { "↑" }
    );
    // The heading is a child rather than the panel's subtitle so that it can carry a hit region of
    // its own: which column was clicked is the only question it answers, and the panel's title row
    // must not answer it too.
    let columns = format!("{}{}", " ".repeat(usize::from(indent)), table_heading());

    Panel::new(title)
        .child(Text::new(columns).muted().hit(&monitor.hits, Zone::Heading).length(1))
        .child(
            Row::new()
                .gap(1)
                .child(list.hit(&monitor.hits, Zone::Table).flex(1))
                .child(
                    Scrollbar::new(monitor.selection.offset(), shown).hit(&monitor.hits, Zone::Bar),
                )
                .flex(1),
        )
}

/// The right-hand column: the machine in four figures, then whatever the cursor is on.
fn sidebar(monitor: &Monitor) -> impl View + '_ {
    let snapshot = &monitor.snapshot;
    let machine = Panel::new("MACHINE")
        .child(Field::new("CORES", snapshot.cores.to_string()))
        .child(Field::new("PROCS", snapshot.processes.len().to_string()))
        .child(Field::new("SHOWN", monitor.visible().len().to_string()))
        .child(Field::new("VIA", snapshot.source.to_string()).value_role(Role::Muted));

    Column::new().gap(1).child(machine.length(5)).child(match monitor.selected() {
        Some(process) => Box::new(detail(process, snapshot).flex(1)) as Box<dyn View>,
        // A filter matching nothing is one keystroke away, and the pane still has to say
        // something when it happens.
        None => Box::new(Panel::new("PROCESS").child(Text::new("nothing selected").dim())),
    })
}

/// Everything known about one process, including the figures the table has no room for.
///
/// The panel's title names the process rather than saying `PROCESS`, which is both the more useful
/// label and a row saved over a heading that says nothing.
fn detail<'a>(process: &'a Process, snapshot: &'a Snapshot) -> impl View + 'a {
    let share = match snapshot.memory_total {
        Some(total) if total > 0 => percent(Some(process.memory as f32 / total as f32)),
        _ => UNKNOWN.to_string(),
    };
    // Name and pid in the title, which is chrome and so is the one part of a panel that a short
    // window cannot take away. That matters here: the fields below are laid out as rows of a fixed
    // height, and when a [`Column`] has less room than its children asked for it shrinks the most
    // elastic first and then cuts the earliest of what is left — so a pane with `PID` as its first
    // field is a pane that loses the pid first, which is the one thing that identifies what is
    // being described. The fields that do go are `CPU` and `MEM`, and they are the two the table is
    // already showing.
    Panel::new(format!("{} · {}", process.name, process.pid))
        .child(Field::new("CPU", cpu_text(process.cpu)))
        .child(Field::new("MEM", format!("{} · {}", bytes(process.memory), share)))
        .child(Field::new("THREADS", optional(process.threads)))
        .child(Field::new("STATE", state_text(&process.state)))
        .child(Spacer::new().length(1))
        // The path is the one field with no fixed width, so it wraps and takes what is left. It
        // goes last for that reason: a view whose height depends on its width cannot be followed by
        // one that needs a guaranteed row.
        .child(Text::new(process.command.clone()).dim().wrapped().flex(1))
}

/// One row at the bottom: the key legend, or the filter you are typing into.
fn footer(monitor: &Monitor, width: u16) -> Box<dyn View + '_> {
    match monitor.mode {
        Mode::Filtering => Box::new(
            Input::new(&monitor.filter)
                .prompt("/")
                .placeholder("name or pid — ENTER to keep, ESC to clear"),
        ),
        // Shed hints rather than let the legend run off the edge. `Q quit` is last, and it is the
        // one a stuck user needs.
        Mode::Browse if width < FULL_LEGEND => Box::new(
            Hints::new()
                .emphasise_keys()
                .spacing(2)
                .key("↑/↓", "move")
                .key("C", "cpu")
                .key("M", "mem")
                .key("/", "filter")
                .key("Q", "quit"),
        ),
        Mode::Browse => Box::new(
            Hints::new()
                .emphasise_keys()
                .spacing(2)
                .key("↑/↓", "move")
                .key("C", "cpu")
                .key("M", "mem")
                .key("P", "pid")
                .key("N", "name")
                .key("R", "reverse")
                .key("/", "filter")
                .key("SPACE", "pause")
                .key("Q", "quit"),
        ),
    }
}

/// The table's column headings, formatted exactly as a row is so the two cannot drift apart.
fn table_heading() -> String {
    format!(
        "{:>pid$} {:>cpu$} {:>mem$}  {}",
        "PID",
        "CPU%",
        "MEM",
        "COMMAND",
        pid = PID_WIDTH,
        cpu = CPU_WIDTH,
        mem = MEM_WIDTH
    )
}

/// One row of the table, on the same widths as [`table_heading`] and from the same constants.
fn table_row(process: &Process) -> String {
    format!(
        "{:>pid$} {:>cpu$} {:>mem$}  {}",
        process.pid,
        cpu_text(process.cpu),
        bytes(process.memory),
        process.name,
        pid = PID_WIDTH,
        cpu = CPU_WIDTH,
        mem = MEM_WIDTH
    )
}

/// Which column a click `local` columns into the heading landed on.
///
/// The heading is one string, so this walks the same widths that formatted it rather than asking a
/// set of sibling views where they ended up. Past the last boundary is the command, which is what
/// the eye expects: everything to the right of `MEM` belongs to the name.
fn column_at(local: u16, indent: u16) -> Order {
    let x = local.saturating_sub(indent);
    let pid = PID_WIDTH as u16;
    let cpu = pid + 1 + CPU_WIDTH as u16;
    let memory = cpu + 1 + MEM_WIDTH as u16;
    if x < pid {
        Order::Pid
    } else if x < cpu {
        Order::Cpu
    } else if x < memory {
        Order::Memory
    } else {
        Order::Name
    }
}

// ---- Formatting -------------------------------------------------------------------------

/// A byte count in the largest unit that keeps it under four digits: `812 KB`, `9.1 GB`.
///
/// One decimal place only below ten of a unit. `1.1 GB` and `1.2 GB` are a difference worth seeing;
/// `1.14 GB` on a row redrawn every second is two digits of flicker.
fn bytes(count: u64) -> String {
    const KB: f64 = 1024.0;
    let count = count as f64;
    if count < KB {
        return format!("{count:.0} B");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut value = count / KB;
    let mut unit = 0;
    while value >= KB && unit + 1 < units.len() {
        value /= KB;
        unit += 1;
    }
    let name = units[unit];
    match (unit, value) {
        // Fractions of a kilobyte are never the interesting part.
        (0, _) => format!("{value:.0} {name}"),
        (_, value) if value < 10.0 => format!("{value:.1} {name}"),
        _ => format!("{value:.0} {name}"),
    }
}

/// A fraction as a whole percentage, or `—`.
fn percent(value: Option<f32>) -> String {
    match value {
        Some(value) => format!("{:.0}%", value * 100.0),
        None => UNKNOWN.to_string(),
    }
}

/// A process's CPU share the way `top` writes it: percent of *one* core, so eight cores flat out
/// reads `800.0` rather than `100`. Hiding that would make a busy machine look idle.
fn cpu_text(cpu: Option<f32>) -> String {
    match cpu {
        Some(cpu) => format!("{:.1}", cpu * 100.0),
        None => UNKNOWN.to_string(),
    }
}

/// `4d 02:11` once a machine has been up a day, `02:11:07` before that.
fn duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (hours, minutes, seconds) = (rest / 3600, (rest % 3600) / 60, rest % 60);
    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

fn optional<T: std::fmt::Display>(value: Option<T>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => UNKNOWN.to_string(),
    }
}

/// A process state in words. `Ss` means nothing to most people, and the pane has room for the
/// answer rather than the code — but an unfamiliar letter is passed through rather than swallowed,
/// because a letter nobody here has documented is still information.
fn state_text(state: &str) -> String {
    match state.chars().next() {
        None => UNKNOWN.to_string(),
        Some('R') => "running".to_string(),
        Some('S') => "sleeping".to_string(),
        Some('I') => "idle".to_string(),
        Some('T') => "stopped".to_string(),
        Some('Z') => "zombie".to_string(),
        Some('D' | 'U') => "waiting".to_string(),
        Some(_) => state.to_string(),
    }
}

// ---- State ------------------------------------------------------------------------------

/// What the table is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    Cpu,
    Memory,
    Pid,
    Name,
}

impl Order {
    fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "MEM",
            Self::Pid => "PID",
            Self::Name => "NAME",
        }
    }

    /// Whether this column reads best largest-first. A name does not; a measurement does, because
    /// the reason to sort by CPU is to find out what is eating it.
    fn descends_by_default(self) -> bool {
        matches!(self, Self::Cpu | Self::Memory)
    }
}

/// What the keyboard is currently doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Browse,
    Filtering,
}

/// Whether the event loop should keep going.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

/// The parts of the screen a click can land in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    /// The rows, for selecting a process and for the wheel.
    Table,
    /// The column headings, for sorting by one.
    Heading,
    /// The one column beside the table, for dragging the thumb and paging the track.
    Bar,
}

struct Monitor {
    snapshot: Snapshot,
    sampler: Sampler,
    /// The machine's CPU and memory as fractions of the whole, oldest first. Only known readings
    /// are pushed, so a platform that cannot answer draws no chart rather than a flat line at zero.
    cpu_history: Vec<f32>,
    memory_history: Vec<f32>,
    order: Order,
    descending: bool,
    filter: Editor,
    /// The filter as the list applies it. Kept beside the field rather than read from it so that
    /// [`Monitor::visible`] needs nothing but `&self`.
    needle: String,
    mode: Mode,
    /// The pid the cursor is on, which is the thing that survives a re-sort. See
    /// [`Monitor::follow`].
    focus: Option<u32>,
    /// Where the cursor is *now*, derived from `focus` every time the list changes. Held rather
    /// than recomputed on demand because a [`List`] scrolls itself against it, and that scroll
    /// position is worth keeping.
    selection: Selection,
    /// How far down the thumb the pointer was when it grabbed it, or `None` when nothing is held.
    ///
    /// Without it the thumb would jump so that its top met the pointer on the first drag event,
    /// which is felt as the list lurching the moment you touch the bar.
    grab: Option<u16>,
    paused: bool,
    host: String,
    /// When the machine was last asked, or `None` before the first sample.
    sampled: Option<Instant>,
    hits: Hits<Zone>,
}

impl Monitor {
    fn new() -> Self {
        Self {
            snapshot: Snapshot::empty(),
            sampler: Sampler::new(),
            cpu_history: Vec::new(),
            memory_history: Vec::new(),
            order: Order::Cpu,
            descending: true,
            filter: Editor::new(),
            needle: String::new(),
            mode: Mode::Browse,
            focus: None,
            selection: Selection::new(),
            grab: None,
            paused: false,
            host: host(),
            sampled: None,
            hits: Hits::new(),
        }
    }

    /// Whether the machine is due to be asked again.
    ///
    /// Pausing works by answering `false` for ever, which freezes the figures without stopping the
    /// frames: the cursor still moves and the table can still be re-sorted, over a sample that
    /// holds still long enough to read.
    fn is_due(&self, now: Instant) -> bool {
        match (self.paused, self.sampled) {
            (true, _) => false,
            (false, None) => true,
            (false, Some(then)) => now.duration_since(then) >= REFRESH,
        }
    }

    /// Ask the machine, and put the answer on screen without losing the user's place.
    fn refresh(&mut self, now: Instant) {
        let snapshot = self.sampler.sample(now);
        self.install(snapshot);
        self.sampled = Some(now);
    }

    /// Take a snapshot as the current one: record its history, sort it, find the cursor again.
    fn install(&mut self, snapshot: Snapshot) {
        if let Some(cpu) = snapshot.cpu {
            push_history(&mut self.cpu_history, cpu);
        }
        if let Some(memory) = snapshot.memory_fraction() {
            push_history(&mut self.memory_history, memory);
        }
        self.snapshot = snapshot;
        self.reorder();
    }

    /// Sort the processes, then put the cursor back on whatever it was on.
    fn reorder(&mut self) {
        let (order, descending) = (self.order, self.descending);
        self.snapshot.processes.sort_by(|left, right| {
            let ordering = match order {
                Order::Cpu => compare_unknown_last(left.cpu, right.cpu, descending),
                Order::Memory => left.memory.cmp(&right.memory),
                Order::Pid => left.pid.cmp(&right.pid),
                Order::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            };
            if descending { ordering.reverse() } else { ordering }
        });
        self.follow();
    }

    /// Point the selection at the focused pid, wherever the list has since put it.
    ///
    /// The whole reason `focus` exists, and called after anything that can change the ordering or
    /// the contents: a sample, a sort, a filter. When the focused process has gone — and on a real
    /// machine they go constantly — the index is left where it was and clamped, so the cursor stays
    /// put on screen instead of springing to the top of the list.
    fn follow(&mut self) {
        let found =
            self.focus.and_then(|pid| self.visible().iter().position(|entry| entry.pid == pid));
        match found {
            Some(index) => self.selection.set_selected(index),
            None => {
                let len = self.visible().len();
                self.selection.clamp(len);
                // Whatever the cursor has landed on is what it now follows, or the next sample
                // would move it again.
                self.remember();
            }
        }
    }

    /// Note which process the cursor is on, after something has moved it.
    fn remember(&mut self) {
        let index = self.selection.selected();
        let pid = self.visible().get(index).map(|process| process.pid);
        self.focus = pid;
    }

    /// Put the cursor on a row of the visible list, as a click does.
    fn select(&mut self, index: usize) {
        self.selection.set_selected(index);
        self.remember();
    }

    /// The processes the table shows: everything matching the filter, in the current order.
    ///
    /// Rebuilt per call rather than cached. It is a few hundred pointer copies over a list that is
    /// already sorted in place, and a cache here would be a second copy of the ordering to keep in
    /// step with the first.
    fn visible(&self) -> Vec<&Process> {
        self.snapshot.processes.iter().filter(|process| self.matches(process)).collect()
    }

    /// Whether a process passes the filter: a name containing it, or a pid starting with it.
    ///
    /// `contains` for the name, because processes are called things like
    /// `com.apple.WebKit.WebContent` and a prefix match would never find the interesting half. A
    /// prefix for the pid, because a number is read left to right.
    fn matches(&self, process: &Process) -> bool {
        if self.needle.is_empty() {
            return true;
        }
        let needle = self.needle.to_lowercase();
        process.name.to_lowercase().contains(&needle)
            || process.pid.to_string().starts_with(&needle)
    }

    fn selected(&self) -> Option<&Process> {
        let index = self.selection.selected();
        self.snapshot.processes.iter().filter(|process| self.matches(process)).nth(index)
    }

    fn process_list(&self) -> List<'_> {
        let rows: Vec<ListRow> = self
            .visible()
            .into_iter()
            .map(|process| {
                let row = ListRow::new(table_row(process));
                // Colour is the only thing the table says that its columns do not: a process
                // taking half a core or more is the one the user came here to find.
                match process.cpu {
                    Some(cpu) if cpu >= 0.5 => row.role(Role::Warn),
                    Some(_) => row,
                    None => row.role(Role::Dim),
                }
            })
            .collect();
        let empty = match (self.snapshot.processes.is_empty(), self.needle.is_empty()) {
            (true, _) => format!("nothing from {}", self.snapshot.source),
            (false, false) => format!("nothing matching “{}”", self.needle),
            (false, true) => "no processes".to_string(),
        };
        List::new(rows).selection(&self.selection).empty(empty)
    }

    // ---- Events ------------------------------------------------------------------------

    fn handle(&mut self, event: &Event) -> Flow {
        match event {
            Event::Key(key) => self.key(key),
            Event::Mouse(mouse) => {
                self.mouse(mouse);
                Flow::Continue
            }
            // A pasted process name is the likeliest way anyone fills this field, and without this
            // arm the text would arrive as nothing at all: bracketed paste means a paste is one
            // event rather than a burst of keys, so a handler that only reads keys never sees it.
            Event::Paste(text) if self.mode == Mode::Filtering => {
                self.filter.insert_str(text);
                self.needle = self.filter.value().to_string();
                self.follow();
                Flow::Continue
            }
            _ => Flow::Continue,
        }
    }

    fn key(&mut self, key: &KeyEvent) -> Flow {
        if self.mode == Mode::Filtering {
            return self.filter_key(key);
        }
        let len = self.visible().len();
        match key.code {
            KeyCode::Char('q' | 'Q') | KeyCode::Escape => return Flow::Quit,
            KeyCode::Up | KeyCode::Char('k') => self.selection.up(),
            KeyCode::Down | KeyCode::Char('j') => self.selection.down(len),
            KeyCode::PageUp => self.selection.page_up(),
            KeyCode::PageDown => self.selection.page_down(len),
            KeyCode::Home => self.selection.first(),
            KeyCode::End => self.selection.last(len),
            KeyCode::Char('c' | 'C') => return self.sort_by(Order::Cpu),
            KeyCode::Char('m' | 'M') => return self.sort_by(Order::Memory),
            KeyCode::Char('p' | 'P') => return self.sort_by(Order::Pid),
            KeyCode::Char('n' | 'N') => return self.sort_by(Order::Name),
            KeyCode::Char('r' | 'R') => {
                self.descending = !self.descending;
                self.reorder();
                return Flow::Continue;
            }
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                return Flow::Continue;
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Filtering;
                self.filter.set_value(self.needle.clone());
                self.filter.end();
                return Flow::Continue;
            }
            _ => return Flow::Continue,
        }
        // Every arm that reaches here moved the cursor, and the cursor moving is the one thing that
        // changes which process is being followed.
        self.remember();
        Flow::Continue
    }

    /// Keys while the filter field has them. The table narrows as it is typed, because a filter you
    /// have to commit before seeing its effect is one you cannot aim.
    fn filter_key(&mut self, key: &KeyEvent) -> Flow {
        match key.code {
            KeyCode::Escape => {
                self.filter.clear();
                self.needle.clear();
                self.mode = Mode::Browse;
                self.follow();
            }
            KeyCode::Enter => self.mode = Mode::Browse,
            _ => {
                if self.filter.handle(key) {
                    self.needle = self.filter.value().to_string();
                    // A filter can take the focused process out of the list entirely, which is
                    // exactly the case `follow` is for.
                    self.follow();
                }
            }
        }
        Flow::Continue
    }

    /// Sort by a column, or turn it round if it is already the one.
    ///
    /// Pressing the same key twice is how anyone finds out what the other end of a list looks like,
    /// and it costs nothing to answer.
    fn sort_by(&mut self, order: Order) -> Flow {
        if self.order == order {
            self.descending = !self.descending;
        } else {
            self.order = order;
            self.descending = order.descends_by_default();
        }
        self.reorder();
        Flow::Continue
    }

    fn mouse(&mut self, mouse: &MouseEvent) {
        let at = Pos::new(mouse.column, mouse.row);
        match mouse.kind {
            // A drag belongs to whatever the press grabbed, not to whatever is under the pointer
            // now. The thumb is one column wide and the hand wanders sideways as it moves down.
            MouseKind::Drag(MouseButton::Left) => {
                if let Some(grab) = self.grab {
                    self.drag_thumb(at, grab);
                }
            }
            MouseKind::Up(MouseButton::Left) => self.grab = None,
            MouseKind::Down(MouseButton::Left) => match self.hits.at(at) {
                Some(Zone::Table) => self.press_table(at),
                Some(Zone::Heading) => self.press_heading(at),
                Some(Zone::Bar) => self.press_bar(at),
                None => {}
            },
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                // The bar counts as the table here: a wheel over a scrollbar means scroll, and
                // being a column away from the rows does not make it mean something else.
                if !matches!(self.hits.at(at), Some(Zone::Table | Zone::Bar)) {
                    return;
                }
                let delta = if mouse.kind == MouseKind::ScrollDown { 1 } else { -1 };
                self.selection.step(delta, self.visible().len());
                self.remember();
            }
            _ => {}
        }
    }

    /// A click in the table: select the row under the pointer, if there is one.
    fn press_table(&mut self, at: Pos) {
        let Some(area) = self.hits.area_of(Zone::Table) else { return };
        let len = self.visible().len();
        if let Some(row) = self.selection.row_at(area, at, len) {
            self.select(row);
        }
    }

    /// A press on the scrollbar: grab the thumb, or page the track beside it.
    ///
    /// Paging rather than jumping, because the track is where you press when you want the next
    /// screenful, not to be thrown somewhere else in the list. A bar with nothing to scroll draws no
    /// thumb, and a press on it does nothing at all.
    fn press_bar(&mut self, at: Pos) {
        let Some(area) = self.hits.area_of(Zone::Bar) else { return };
        let bar = Scrollbar::new(self.selection.offset(), self.visible().len());
        let Some(thumb) = bar.thumb(area.height) else { return };
        let row = at.y - area.y;
        if thumb.contains(&row) {
            self.grab = Some(row - thumb.start);
        } else if row < thumb.start {
            self.selection.page_up();
            self.remember();
        } else {
            self.selection.page_down(self.visible().len());
            self.remember();
        }
    }

    /// Drag the thumb so it goes on sitting `grab` rows below the pointer.
    ///
    /// The pointer's row is clamped to the bar rather than discarded: overshooting the end and
    /// having the list stop responding is what gets called sticky, and reading a pointer past the
    /// end as a request for the end is the fix.
    ///
    /// The selection travels with the view — [`Selection::scroll_to`] brings it to the nearest edge
    /// of the new window — so the detail pane follows the drag. That is the honest behaviour for a
    /// list whose scroll position is derived from its cursor: there is no offset here that the
    /// cursor does not imply.
    fn drag_thumb(&mut self, at: Pos, grab: u16) {
        let Some(area) = self.hits.area_of(Zone::Bar) else { return };
        let len = self.visible().len();
        let row = at.y.clamp(area.y, area.y + area.height.saturating_sub(1)) - area.y;
        let offset = Scrollbar::new(self.selection.offset(), len)
            .offset_at(row.saturating_sub(grab), area.height);
        self.selection.scroll_to(offset, len);
        self.remember();
    }

    /// A click on the column headings: sort by the column clicked.
    fn press_heading(&mut self, at: Pos) {
        let Some(local) = self.hits.local(Zone::Heading, at) else { return };
        // The heading is indented by the list's cursor column, and the list is the only thing that
        // knows how wide that is.
        let indent = self.process_list().text_column();
        self.sort_by(column_at(local.x, indent));
    }

    // ---- Fixtures ----------------------------------------------------------------------

    /// A fixed machine, for `--dump` and for the tests.
    ///
    /// Figures from a real laptop, frozen: every number on the screen is then reproducible, which
    /// is what makes a rendered frame something a test can assert on.
    fn fixture() -> Self {
        let mut monitor = Self::new();
        monitor.host = "studio.local".to_string();
        monitor.install(Snapshot::fixture());
        // History for the charts to draw. Written out rather than generated, for the same reason
        // the processes are.
        monitor.cpu_history = HISTORY_CPU.to_vec();
        monitor.memory_history = HISTORY_MEMORY.to_vec();
        monitor
    }
}

/// Keep the last [`HISTORY`] readings, oldest first.
fn push_history(history: &mut Vec<f32>, value: f32) {
    history.push(value);
    if history.len() > HISTORY {
        history.remove(0);
    }
}

/// Compare two readings so that `None` sorts last whichever way the column points.
///
/// Sorting descending reverses the comparison, so an unknown has to be *pre*-reversed to stay at the
/// bottom — otherwise the rows with nothing to say take the top of the table.
fn compare_unknown_last<T: PartialOrd>(
    left: Option<T>,
    right: Option<T>,
    descending: bool,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let last = if descending { Ordering::Less } else { Ordering::Greater };
    match (left, right) {
        (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        (Some(_), None) => last.reverse(),
        (None, Some(_)) => last,
        (None, None) => Ordering::Equal,
    }
}

// ---- The machine ------------------------------------------------------------------------

/// One process, as this app thinks of it.
#[derive(Clone, Debug)]
struct Process {
    pid: u32,
    /// What to call it: the executable's name, without its path.
    name: String,
    /// Where it came from, for the detail pane.
    command: String,
    memory: u64,
    /// Share of one core, `1.0` being a core flat out. `None` until a rate can be worked out, which
    /// takes two samples — and for ever on a platform that will not say.
    cpu: Option<f32>,
    threads: Option<u32>,
    state: String,
}

/// The machine at one instant.
#[derive(Clone, Debug)]
struct Snapshot {
    cores: usize,
    /// All cores together, `1.0` being every core flat out.
    cpu: Option<f32>,
    memory_used: Option<u64>,
    memory_total: Option<u64>,
    uptime: Option<Duration>,
    /// Where these figures came from, named on screen. A dashboard that will not say where its
    /// numbers come from is asking to be believed rather than read.
    source: &'static str,
    processes: Vec<Process>,
}

impl Snapshot {
    fn empty() -> Self {
        Self {
            cores: cores(),
            cpu: None,
            memory_used: None,
            memory_total: None,
            uptime: None,
            source: SOURCE,
            processes: Vec::new(),
        }
    }

    fn memory_fraction(&self) -> Option<f32> {
        match (self.memory_used, self.memory_total) {
            (Some(used), Some(total)) if total > 0 => Some(used as f32 / total as f32),
            _ => None,
        }
    }
}

/// What a platform hands over, before any rate has been worked out.
struct Readings {
    memory_used: Option<u64>,
    memory_total: Option<u64>,
    uptime: Option<Duration>,
    processes: Vec<Reading>,
}

impl Readings {
    /// A platform that answered nothing, and the base every partial answer is built from.
    ///
    /// Unused on macOS and Linux, which fill in every field — hence the `allow`, which is a truer
    /// statement than making one of them pretend to need it.
    #[allow(dead_code)]
    fn none() -> Self {
        Self { memory_used: None, memory_total: None, uptime: None, processes: Vec::new() }
    }
}

/// One process as the platform describes it: totals, never rates.
struct Reading {
    pid: u32,
    name: String,
    command: String,
    memory: u64,
    /// CPU time used since the process started. The only honest thing a single reading can give,
    /// and what [`Sampler`] turns into a percentage.
    used: Option<Duration>,
    threads: Option<u32>,
    state: String,
}

/// Turns readings into rates.
///
/// The one piece of state in the data path, and it exists because CPU percentage is not a quantity
/// the machine has: it is `(this reading − the last one) ÷ the time between them`. Keeping the last
/// reading here rather than in the app is what lets the app treat a snapshot as a value.
struct Sampler {
    /// Cumulative CPU time per pid, as of the last sample.
    previous: HashMap<u32, Duration>,
    at: Option<Instant>,
}

impl Sampler {
    fn new() -> Self {
        Self { previous: HashMap::new(), at: None }
    }

    /// Ask the platform, and work out what has changed since last time.
    fn sample(&mut self, now: Instant) -> Snapshot {
        self.rates(readings(), now)
    }

    /// The half of [`Sampler::sample`] that does not touch the machine, so a test can hand it two
    /// readings a known interval apart and check the arithmetic rather than the weather.
    fn rates(&mut self, readings: Readings, now: Instant) -> Snapshot {
        let elapsed = self.at.map(|then| now.duration_since(then)).filter(|gap| !gap.is_zero());
        let mut current = HashMap::with_capacity(readings.processes.len());
        let mut busy = 0.0f32;
        let mut measured = false;

        let processes = readings
            .processes
            .into_iter()
            .map(|reading| {
                let cpu = match (reading.used, self.previous.get(&reading.pid), elapsed) {
                    (Some(used), Some(&before), Some(elapsed)) => {
                        // `saturating_sub` because a pid can be reused: the new process starts from
                        // nothing, and a negative rate is not a figure to show anyone.
                        let spent = used.saturating_sub(before);
                        Some(spent.as_secs_f32() / elapsed.as_secs_f32())
                    }
                    _ => None,
                };
                if let Some(used) = reading.used {
                    current.insert(reading.pid, used);
                }
                if let Some(cpu) = cpu {
                    busy += cpu;
                    measured = true;
                }
                Process {
                    pid: reading.pid,
                    name: reading.name,
                    command: reading.command,
                    memory: reading.memory,
                    cpu,
                    threads: reading.threads,
                    state: reading.state,
                }
            })
            .collect();

        self.previous = current;
        self.at = Some(now);
        let cores = cores();
        Snapshot {
            cores,
            // Every process's share over the cores there are to share. This counts only work done
            // inside a process the platform listed, so a machine busy in the kernel reads a little
            // low — the alternative is a second, per-platform way of asking, for a figure whose job
            // is to show a trend.
            cpu: measured.then(|| (busy / cores as f32).clamp(0.0, 1.0)),
            memory_used: readings.memory_used,
            memory_total: readings.memory_total,
            uptime: readings.uptime,
            source: SOURCE,
            processes,
        }
    }
}

/// How many cores there are to divide by. The std answer, which is the right one everywhere.
fn cores() -> usize {
    std::thread::available_parallelism().map(usize::from).unwrap_or(1)
}

// ---- macOS ------------------------------------------------------------------------------

#[cfg(target_os = "macos")]
const SOURCE: &str = "ps";

#[cfg(target_os = "macos")]
fn host() -> String {
    output_of("/bin/hostname", &["-s"]).map(|text| text.trim().to_string()).unwrap_or_default()
}

/// `ps` for the processes, `vm_stat` for memory, `sysctl` for the boot time.
///
/// Shelling out rather than calling `sysctl(3)` and `proc_pidinfo` keeps this example inside the
/// crate's own dependency set: `conui` does not link `rustix`, and adding a dependency for an
/// example would misrepresent what the library needs.
#[cfg(target_os = "macos")]
fn readings() -> Readings {
    Readings {
        memory_used: memory_used(),
        memory_total: sysctl("hw.memsize").and_then(|text| text.trim().parse().ok()),
        uptime: uptime(),
        processes: processes(),
    }
}

#[cfg(target_os = "macos")]
fn processes() -> Vec<Reading> {
    let Some(output) = output_of("/bin/ps", &["-Ao", "pid=,rss=,time=,state=,comm="]) else {
        return Vec::new();
    };
    output.lines().filter_map(parse_ps).collect()
}

/// One line of `ps -Ao pid=,rss=,time=,state=,comm=`.
///
/// Four fields and then the rest of the line, because the last one is a path and a path can contain
/// spaces — `split_whitespace` over the whole line would lose everything after the first.
#[cfg(target_os = "macos")]
fn parse_ps(line: &str) -> Option<Reading> {
    let mut rest = line.trim_start();
    let mut fields = [""; 4];
    for field in &mut fields {
        let end = rest.find(char::is_whitespace)?;
        *field = &rest[..end];
        rest = rest[end..].trim_start();
    }
    let [pid, rss, time, state] = fields;
    if rest.is_empty() {
        return None;
    }
    Some(Reading {
        pid: pid.parse().ok()?,
        name: rest.rsplit('/').next().unwrap_or(rest).to_string(),
        command: rest.to_string(),
        // `rss` is in kilobytes, which is the only unit `ps` reports it in.
        memory: rss.parse::<u64>().ok()? * 1024,
        used: parse_cpu_time(time),
        threads: None,
        state: state.to_string(),
    })
}

/// Cumulative CPU time as `ps` prints it: `mm:ss.cc`, `hh:mm:ss` once it runs long, and
/// `dd-hh:mm:ss` for something that has been up for weeks.
#[cfg(target_os = "macos")]
fn parse_cpu_time(text: &str) -> Option<Duration> {
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.parse::<f64>().ok()?, rest),
        None => (0.0, text),
    };
    let mut seconds = 0.0f64;
    for part in rest.split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Duration::try_from_secs_f64(seconds + days * 86_400.0).ok()
}

/// Memory in use, counted as Activity Monitor counts it: the pages that are resident and not free.
/// `vm_stat` reports in pages and states its own page size, which is 16K on Apple silicon and 4K
/// elsewhere — hence reading it rather than assuming.
#[cfg(target_os = "macos")]
fn memory_used() -> Option<u64> {
    let output = output_of("/usr/bin/vm_stat", &[])?;
    let page = output
        .split_once("page size of ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(4096);
    let mut pages = 0u64;
    for line in output.lines() {
        let Some((label, value)) = line.split_once(':') else { continue };
        let counted = matches!(
            label.trim(),
            "Pages active" | "Pages wired down" | "Pages occupied by compressor"
        );
        if counted {
            pages += value.trim().trim_end_matches('.').parse::<u64>().unwrap_or(0);
        }
    }
    Some(pages * page)
}

/// Uptime from the boot time, which is what the kernel actually keeps.
#[cfg(target_os = "macos")]
fn uptime() -> Option<Duration> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let output = sysctl("kern.boottime")?;
    // `{ sec = 1758537314, usec = 931725 } Mon Sep 22 ...`
    let (_, rest) = output.split_once("sec = ")?;
    let seconds: u64 = rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    now.checked_sub(Duration::from_secs(seconds))
}

#[cfg(target_os = "macos")]
fn sysctl(name: &str) -> Option<String> {
    output_of("/usr/sbin/sysctl", &["-n", name])
}

// ---- Linux ------------------------------------------------------------------------------

#[cfg(target_os = "linux")]
const SOURCE: &str = "/proc";

#[cfg(target_os = "linux")]
fn host() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn readings() -> Readings {
    let (used, total) = memory();
    Readings { memory_used: used, memory_total: total, uptime: uptime(), processes: processes() }
}

/// Every numeric directory in `/proc` is a process. Ones that go away mid-walk are skipped rather
/// than reported, which happens often enough on a busy machine to be the normal case.
#[cfg(target_os = "linux")]
fn processes() -> Vec<Reading> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(read_process)
        .collect()
}

#[cfg(target_os = "linux")]
fn read_process(pid: u32) -> Option<Reading> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut reading = parse_proc_stat(pid, &stat)?;
    // The command line is a separate file and worth the extra read: `stat` gives a name truncated
    // to fifteen characters, which is how `chrome_crashpad` ends up on screen.
    let command = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|raw| raw.replace('\0', " ").trim().to_string())
        .filter(|text| !text.is_empty());
    if let Some(command) = command {
        reading.command = command;
    }
    Some(reading)
}

/// One `/proc/<pid>/stat` line.
///
/// The comm field is parenthesised and may contain spaces and parentheses of its own — a process is
/// free to call itself `((:` — so everything is measured from the *last* `)` rather than counted
/// from the left. The field numbers below are `proc(5)`'s own, one-based.
#[cfg(target_os = "linux")]
fn parse_proc_stat(pid: u32, stat: &str) -> Option<Reading> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    let fields: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    // `fields[0]` is field 3, so field *n* is `fields[n - 3]`.
    let field = |n: usize| fields.get(n - 3).copied().unwrap_or("0");
    let user: f64 = field(14).parse().unwrap_or(0.0);
    let system: f64 = field(15).parse().unwrap_or(0.0);
    Some(Reading {
        pid,
        name: name.clone(),
        command: name,
        // Field 24 is resident pages. 4K is the page size on every platform this code will meet;
        // `getconf PAGESIZE` would be exact, at one more process spawned per sample.
        memory: field(24).parse::<u64>().unwrap_or(0) * 4096,
        // Ticks are USER_HZ, which is 100 on Linux and has been since the field existed.
        used: Duration::try_from_secs_f64((user + system) / 100.0).ok(),
        threads: field(20).parse().ok(),
        state: field(3).to_string(),
    })
}

#[cfg(target_os = "linux")]
fn memory() -> (Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else { return (None, None) };
    let kilobytes = |key: &str| -> Option<u64> {
        let line = text.lines().find(|line| line.starts_with(key))?;
        Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
    };
    let total = kilobytes("MemTotal:");
    // `MemAvailable` rather than `MemFree`: free memory on Linux is memory doing nothing, and the
    // page cache is not doing nothing. `MemFree` would report a healthy machine as nearly full.
    let used = match (total, kilobytes("MemAvailable:")) {
        (Some(total), Some(available)) => Some(total.saturating_sub(available)),
        _ => None,
    };
    (used, total)
}

#[cfg(target_os = "linux")]
fn uptime() -> Option<Duration> {
    let text = std::fs::read_to_string("/proc/uptime").ok()?;
    Duration::try_from_secs_f64(text.split_whitespace().next()?.parse().ok()?).ok()
}

// ---- Windows ----------------------------------------------------------------------------

#[cfg(target_os = "windows")]
const SOURCE: &str = "tasklist";

#[cfg(target_os = "windows")]
fn host() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// `tasklist` answers half the questions, and nothing in the box answers the rest.
///
/// Per-process CPU time, total memory and uptime all want the Win32 calls that `conui-term` links
/// and this crate does not — so rather than pretend, those fields stay `None` and the screen prints
/// `—` where a figure would go. That path is worth having anyway: it is the same one every platform
/// takes for the first second, before a rate can exist.
#[cfg(target_os = "windows")]
fn readings() -> Readings {
    let Some(output) = output_of("tasklist", &["/nh", "/fo", "csv"]) else {
        return Readings::none();
    };
    Readings { processes: output.lines().filter_map(parse_tasklist).collect(), ..Readings::none() }
}

/// One CSV row: `"conui.exe","4821","Console","1","12,345 K"`.
#[cfg(target_os = "windows")]
fn parse_tasklist(line: &str) -> Option<Reading> {
    let fields: Vec<&str> = line.split("\",\"").map(|field| field.trim_matches('"')).collect();
    let [name, pid, .., memory] = fields.as_slice() else { return None };
    // The figure is grouped by thousands in whatever the console's locale is, so the digits are
    // taken and the separators left behind.
    let kilobytes: u64 =
        memory.chars().filter(char::is_ascii_digit).collect::<String>().parse().ok()?;
    Some(Reading {
        pid: pid.parse().ok()?,
        name: (*name).to_string(),
        command: (*name).to_string(),
        memory: kilobytes * 1024,
        used: None,
        threads: None,
        state: String::new(),
    })
}

// ---- Anywhere else ----------------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const SOURCE: &str = "nowhere";

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn host() -> String {
    String::new()
}

/// No sampler for this platform. The screen says so, which is the whole reason every figure on it
/// is an `Option`.
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn readings() -> Readings {
    Readings::none()
}

/// Run a command and take its standard output, or `None` if anything at all went wrong.
///
/// Nothing here is worth an error path: a monitor that cannot reach `ps` has nothing to show, and
/// the screen already knows how to say that.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn output_of(program: &str, arguments: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program).args(arguments).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

// ---- The fixture ------------------------------------------------------------------------

/// CPU history: idle, then a build, then a long tail. Written out rather than generated so that a
/// rendered sparkline is a fixed string a test can look for.
#[rustfmt::skip]
const HISTORY_CPU: [f32; 32] = [
    0.04, 0.06, 0.05, 0.08, 0.12, 0.10, 0.09, 0.07,
    0.22, 0.48, 0.71, 0.86, 0.92, 0.88, 0.74, 0.61,
    0.55, 0.49, 0.44, 0.38, 0.35, 0.31, 0.29, 0.27,
    0.26, 0.28, 0.31, 0.34, 0.36, 0.35, 0.34, 0.34,
];

#[rustfmt::skip]
const HISTORY_MEMORY: [f32; 32] = [
    0.41, 0.41, 0.42, 0.42, 0.43, 0.44, 0.44, 0.45,
    0.47, 0.51, 0.54, 0.56, 0.58, 0.59, 0.59, 0.58,
    0.57, 0.57, 0.56, 0.56, 0.56, 0.55, 0.55, 0.56,
    0.56, 0.56, 0.57, 0.57, 0.57, 0.57, 0.56, 0.56,
];

impl Snapshot {
    /// A machine that is not this one: eight cores, 16 GB, up four days, and eight processes with
    /// one of them unmeasured so that the `—` path is on screen in the dump.
    fn fixture() -> Self {
        fn process(
            pid: u32,
            name: &str,
            command: &str,
            cpu: Option<f32>,
            memory: u64,
            threads: u32,
            state: &str,
        ) -> Process {
            Process {
                pid,
                name: name.to_string(),
                command: command.to_string(),
                memory,
                cpu,
                threads: Some(threads),
                state: state.to_string(),
            }
        }
        Self {
            cores: 8,
            cpu: Some(0.34),
            memory_used: Some(9_760_000_000),
            memory_total: Some(17_179_869_184),
            uptime: Some(Duration::from_secs(4 * 86_400 + 2 * 3600 + 11 * 60)),
            source: "a fixture",
            processes: vec![
                process(0, "kernel_task", "kernel_task", Some(1.24), 1_181_116_006, 184, "Ss"),
                process(
                    182,
                    "WindowServer",
                    "/System/Library/PrivateFrameworks/SkyLight.framework/WindowServer",
                    Some(0.81),
                    778_043_392,
                    22,
                    "Ss",
                ),
                process(
                    4821,
                    "cargo",
                    "/opt/homebrew/bin/cargo build --workspace",
                    Some(0.62),
                    214_958_080,
                    9,
                    "R",
                ),
                process(
                    4832,
                    "rustc",
                    "/opt/homebrew/bin/rustc --crate-name conui",
                    Some(0.58),
                    486_539_264,
                    12,
                    "R",
                ),
                process(
                    311,
                    "mds_stores",
                    "/System/Library/Frameworks/CoreServices.framework/mds_stores",
                    Some(0.07),
                    96_468_992,
                    6,
                    "Ss",
                ),
                process(
                    1204,
                    "monitor",
                    "target/debug/examples/monitor",
                    Some(0.009),
                    12_582_912,
                    3,
                    "R",
                ),
                process(
                    97,
                    "fseventsd",
                    "/System/Library/Frameworks/CoreServices.framework/fseventsd",
                    Some(0.004),
                    6_291_456,
                    5,
                    "Ss",
                ),
                process(93, "logd", "/usr/libexec/logd", None, 32_178_176, 4, "Ss"),
            ],
        }
    }
}

// ---- Tests ------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use conui::{Modifiers, Rect};

    fn app() -> Monitor {
        Monitor::fixture()
    }

    /// Compose one frame and return it as text, which is what every layout assertion here reads.
    fn screen(monitor: &Monitor, width: u16, height: u16) -> String {
        let mut buffer = Buffer::new(width, height);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        compose(&mut frame, monitor);
        (0..height).map(|row| buffer.row_text(row)).collect::<Vec<_>>().join("\n")
    }

    fn press(monitor: &mut Monitor, code: KeyCode) -> Flow {
        monitor.handle(&Event::Key(KeyEvent::plain(code)))
    }

    fn type_text(monitor: &mut Monitor, text: &str) {
        for character in text.chars() {
            press(monitor, KeyCode::Char(character));
        }
    }

    /// The names in the table, top to bottom.
    fn names(monitor: &Monitor) -> Vec<String> {
        monitor.visible().iter().map(|process| process.name.clone()).collect()
    }

    fn focused(monitor: &Monitor) -> Option<&str> {
        monitor.selected().map(|process| process.name.as_str())
    }

    /// The column a substring starts at, counting cells rather than bytes: the marker on the
    /// selected row is one cell of three bytes, and a byte offset would be two out.
    fn column_of(line: &str, needle: &str) -> usize {
        let byte = line.find(needle).unwrap_or_else(|| panic!("{needle:?} is not in {line:?}"));
        line[..byte].chars().count() + needle.chars().count()
    }

    // ---- The screen ----------------------------------------------------------------------

    #[test]
    fn the_frame_has_a_heading_a_table_and_a_legend() {
        let rendered = screen(&app(), 100, 24);
        assert!(rendered.contains("CONUI  /  MONITOR"));
        assert!(rendered.contains("studio.local · up 4d 02:11"));
        assert!(rendered.contains("PROCESSES · CPU ↓"));
        assert!(rendered.contains("COMMAND"), "the column headings are missing");
        assert!(rendered.contains("Q quit"));
    }

    #[test]
    fn the_columns_line_up_under_their_headings() {
        let rendered = screen(&app(), 100, 24);
        let heading = rendered.lines().find(|line| line.contains("CPU%")).expect("a heading");
        let row = rendered.lines().find(|line| line.contains("kernel_task")).expect("a row");
        // Both are right-aligned in the same nine columns, by two different functions reading the
        // same constant. If either drifts, the figure stops sitting under its heading.
        assert_eq!(column_of(heading, "MEM"), column_of(row, "GB"), "{heading:?} / {row:?}");
    }

    #[test]
    fn both_meters_draw_a_chart_and_a_bar() {
        let rendered = screen(&app(), 100, 24);
        assert!(rendered.contains("CPU"));
        assert!(rendered.contains("MEM"));
        assert!(rendered.contains("34%"), "the CPU readout");
        assert!(rendered.contains("9.1 GB / 16 GB"), "the memory readout: {rendered}");
        // Eight levels of block, scaled against a whole machine rather than against the tallest
        // reading — so the fixture's busiest second is high but not full, which is the honest shape.
        assert!(rendered.contains('▁') && rendered.contains('▇'), "no sparkline: {rendered}");
        // Both bars are on one row. A visible track is what makes a level look like a level, and
        // with the machine at 34% and 57% rather more of the row should be empty than filled.
        let bars = rendered.lines().find(|line| line.contains('░')).expect("a bar");
        let (filled, track) = (bars.matches('█').count(), bars.matches('░').count());
        assert!(filled > 0, "no fill: {bars:?}");
        assert!(track > filled, "a half-idle machine drew a full bar: {bars:?}");
    }

    #[test]
    fn a_reading_the_platform_will_not_give_prints_as_a_dash_not_a_zero() {
        let rendered = screen(&app(), 100, 24);
        let row = rendered.lines().find(|line| line.contains("logd")).expect("a row for logd");
        assert!(row.contains(UNKNOWN), "an unmeasured process claims a figure: {row:?}");
        assert!(!row.contains("0.0"), "got {row:?}");
    }

    #[test]
    fn a_machine_with_nothing_to_say_says_so() {
        let mut monitor = Monitor::new();
        monitor.install(Snapshot::empty());
        let rendered = screen(&monitor, 100, 24);
        assert!(rendered.contains("nothing from"), "the table: {rendered}");
        assert!(rendered.contains("no reading"), "and the meters admit it too");
    }

    #[test]
    fn the_detail_pane_gives_way_to_the_table_when_the_window_is_narrow() {
        let wide = screen(&app(), 110, 24);
        assert!(wide.contains("MACHINE"));
        assert!(wide.contains("THREADS"));
        let narrow = screen(&app(), 80, 24);
        assert!(!narrow.contains("MACHINE"), "the sidebar survived a narrow window: {narrow}");
        assert!(narrow.contains("PROCESSES"), "and the table did not");
    }

    #[test]
    fn the_detail_pane_describes_the_selected_process() {
        let mut monitor = app();
        monitor.select(0);
        let rendered = screen(&monitor, 110, 24);
        assert!(rendered.contains("kernel_task"));
        assert!(rendered.contains("184"), "the thread count: {rendered}");
        assert!(rendered.contains("sleeping"), "the state in words: {rendered}");
        monitor.select(2);
        let rendered = screen(&monitor, 110, 24);
        assert!(rendered.contains("build --workspace"), "the command, wrapped: {rendered}");
    }

    #[test]
    fn the_legend_sheds_hints_before_it_would_run_off_the_edge() {
        // Two columns of padding either side, so the legend has four fewer than the window.
        let wide = screen(&app(), FULL_LEGEND + 4, 24);
        assert!(
            wide.contains("SPACE pause"),
            "the full legend fits and was dropped anyway: {wide}"
        );
        let narrow = screen(&app(), FULL_LEGEND + 3, 24);
        assert!(!narrow.contains("SPACE pause"), "a legend one column too wide: {narrow}");
        assert!(narrow.contains("Q quit"), "and quit is the one hint that never goes");
    }

    /// A pane too short for its fields drops some of them, and this pins which.
    ///
    /// The pid identifies what is being described, so it lives in the title where nothing can take
    /// it; the fields that go are the two the table is already showing.
    #[test]
    fn a_detail_pane_with_too_few_rows_keeps_the_process_it_is_describing() {
        let mut monitor = app();
        monitor.select(2);
        // From 16 rows up. At the two below that the sidebar has no room for a second panel and the
        // pane is gone altogether, which is a different thing from being there and unreadable.
        for height in 16..24 {
            let rendered = screen(&monitor, 110, height);
            assert!(rendered.contains("cargo · 4821"), "at {height} rows: {rendered}");
        }
        let short = screen(&monitor, 110, 20);
        assert!(short.contains("STATE"), "a field the table cannot show went first: {short}");
        // The `CPU` field's own row, which is the only line carrying both the label and the figure:
        // the meter above has the label and the table row has the figure.
        let field = short.lines().find(|line| line.contains("CPU") && line.contains("62.0"));
        assert_eq!(field, None, "the field the table duplicates stayed instead: {short}");
    }

    #[test]
    fn a_short_window_loses_rows_rather_than_the_chrome() {
        let rendered = screen(&app(), 100, MIN_HEIGHT);
        assert!(rendered.contains("PROCESSES"));
        assert!(rendered.contains("Q quit"), "the legend went first: {rendered}");
    }

    // ---- Sorting -------------------------------------------------------------------------

    #[test]
    fn the_table_starts_sorted_by_cpu_because_that_is_what_anyone_opens_it_for() {
        let monitor = app();
        assert_eq!(names(&monitor).first().map(String::as_str), Some("kernel_task"));
        assert_eq!(names(&monitor).last().map(String::as_str), Some("logd"), "unknown sorts last");
    }

    #[test]
    fn sorting_by_memory_puts_the_biggest_first() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('m'));
        assert_eq!(names(&monitor).first().map(String::as_str), Some("kernel_task"));
        assert_eq!(names(&monitor).last().map(String::as_str), Some("fseventsd"));
        assert!(screen(&monitor, 100, 24).contains("PROCESSES · MEM ↓"));
    }

    #[test]
    fn sorting_by_name_is_alphabetical_and_upwards() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('n'));
        assert_eq!(names(&monitor).first().map(String::as_str), Some("cargo"));
        assert!(!monitor.descending, "a list of names reads top to bottom");
    }

    #[test]
    fn sorting_by_pid_is_numeric_not_lexical() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('p'));
        let pids: Vec<u32> = monitor.visible().iter().map(|process| process.pid).collect();
        assert_eq!(pids, vec![0, 93, 97, 182, 311, 1204, 4821, 4832]);
    }

    #[test]
    fn asking_for_the_same_column_twice_turns_it_round() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('m'));
        let descending = names(&monitor);
        press(&mut monitor, KeyCode::Char('m'));
        let mut ascending = names(&monitor);
        ascending.reverse();
        assert_eq!(descending, ascending);
        assert!(screen(&monitor, 100, 24).contains("PROCESSES · MEM ↑"));
    }

    #[test]
    fn r_reverses_whatever_the_order_is() {
        let mut monitor = app();
        // By name, where every row has a value: a CPU sort pins the unmeasured rows to the bottom
        // whichever way it points, so reversing that list is deliberately not a plain reversal.
        press(&mut monitor, KeyCode::Char('n'));
        let forwards = names(&monitor);
        press(&mut monitor, KeyCode::Char('r'));
        let mut backwards = names(&monitor);
        backwards.reverse();
        assert_eq!(forwards, backwards);
        assert!(monitor.descending);
    }

    #[test]
    fn an_unmeasured_process_sorts_last_in_both_directions() {
        let mut monitor = app();
        assert_eq!(names(&monitor).last().map(String::as_str), Some("logd"));
        press(&mut monitor, KeyCode::Char('c'));
        assert!(!monitor.descending, "the same key twice turned it round");
        assert_eq!(
            names(&monitor).last().map(String::as_str),
            Some("logd"),
            "a row with no reading belongs at the bottom whichever way the column points"
        );
    }

    // ---- The cursor follows the process, not the row ------------------------------------

    #[test]
    fn re_sorting_keeps_the_cursor_on_the_same_process() {
        let mut monitor = app();
        monitor.select(2);
        assert_eq!(focused(&monitor), Some("cargo"));
        press(&mut monitor, KeyCode::Char('n'));
        assert_eq!(focused(&monitor), Some("cargo"), "the cursor followed the row number");
        assert_eq!(monitor.selection.selected(), 0, "and cargo is first alphabetically");
    }

    #[test]
    fn a_sample_that_reorders_the_list_does_not_move_the_cursor() {
        let mut monitor = app();
        monitor.select(3);
        assert_eq!(focused(&monitor), Some("rustc"));
        // The machine gets on with it: rustc finishes and drops down a CPU sort.
        let mut next = Snapshot::fixture();
        for process in &mut next.processes {
            if process.name == "rustc" {
                process.cpu = Some(0.001);
            }
        }
        monitor.install(next);
        assert_eq!(focused(&monitor), Some("rustc"), "the cursor stayed on the row number");
        assert!(monitor.selection.selected() > 3, "and rustc has moved down the table");
    }

    #[test]
    fn a_process_that_exits_leaves_the_cursor_where_it_was() {
        let mut monitor = app();
        monitor.select(2);
        assert_eq!(focused(&monitor), Some("cargo"));
        let mut next = Snapshot::fixture();
        next.processes.retain(|process| process.name != "cargo");
        monitor.install(next);
        assert_eq!(monitor.selection.selected(), 2, "the cursor jumped rather than staying put");
        assert_eq!(focused(&monitor), Some("rustc"), "which is the row that took its place");
        assert_eq!(monitor.focus, Some(4832), "and that is what it follows now");
    }

    #[test]
    fn the_cursor_survives_the_last_process_going_away() {
        let mut monitor = app();
        monitor.select(7);
        let mut next = Snapshot::fixture();
        next.processes.truncate(3);
        monitor.install(next);
        assert_eq!(monitor.selection.selected(), 2, "clamped to the end of a shorter list");
        assert!(focused(&monitor).is_some());
    }

    #[test]
    fn an_empty_machine_leaves_nothing_selected_rather_than_panicking() {
        let mut monitor = app();
        monitor.select(4);
        monitor.install(Snapshot::empty());
        assert_eq!(focused(&monitor), None);
        assert_eq!(monitor.focus, None);
        let _ = screen(&monitor, 100, 24);
    }

    // ---- Filtering -----------------------------------------------------------------------

    #[test]
    fn the_filter_narrows_the_table_as_it_is_typed() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "rust");
        assert_eq!(names(&monitor), vec!["rustc"]);
        // The footer has become the field, and the field is showing what has been typed into it.
        let rendered = screen(&monitor, 100, 24);
        assert!(rendered.contains("/rust"), "got {rendered}");
        assert!(!rendered.contains("Q quit"), "the legend is still there under the field");
    }

    #[test]
    fn an_empty_filter_says_what_the_field_is_for() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        assert!(screen(&monitor, 100, 24).contains("ENTER to keep"), "no placeholder");
    }

    #[test]
    fn a_process_name_can_be_pasted_into_the_filter() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        monitor.handle(&Event::Paste("WindowServer".to_string()));
        assert_eq!(monitor.needle, "WindowServer");
        assert_eq!(names(&monitor), vec!["WindowServer"], "the list narrowed on the paste alone");
    }

    #[test]
    fn a_paste_while_browsing_is_not_a_filter() {
        let mut monitor = app();
        monitor.handle(&Event::Paste("rustc".to_string()));
        assert!(monitor.needle.is_empty(), "text arrived in a field nobody had opened");
        assert_eq!(names(&monitor).len(), 8);
    }

    #[test]
    fn the_filter_matches_a_pid_by_its_start() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "48");
        assert_eq!(names(&monitor), vec!["cargo", "rustc"]);
    }

    #[test]
    fn the_filter_is_not_case_sensitive() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "WINDOW");
        assert_eq!(names(&monitor), vec!["WindowServer"]);
    }

    #[test]
    fn filtering_the_selected_process_away_moves_the_cursor_to_one_that_is_there() {
        let mut monitor = app();
        monitor.select(0);
        assert_eq!(focused(&monitor), Some("kernel_task"));
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "rustc");
        assert_eq!(focused(&monitor), Some("rustc"), "the cursor is off the end of a one-row list");
        assert_eq!(monitor.selection.selected(), 0);
    }

    #[test]
    fn a_filter_that_matches_nothing_says_so_rather_than_looking_broken() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "nothing called this");
        assert!(monitor.visible().is_empty());
        let rendered = screen(&monitor, 110, 24);
        assert!(rendered.contains("nothing matching"), "got {rendered}");
        assert!(rendered.contains("nothing selected"), "and the detail pane too");
    }

    #[test]
    fn escape_clears_the_filter_and_gives_the_keys_back() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "rustc");
        press(&mut monitor, KeyCode::Escape);
        assert_eq!(monitor.mode, Mode::Browse);
        assert!(monitor.needle.is_empty());
        assert_eq!(names(&monitor).len(), 8);
    }

    #[test]
    fn enter_keeps_the_filter_and_gives_the_keys_back() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "rust");
        press(&mut monitor, KeyCode::Enter);
        assert_eq!(monitor.mode, Mode::Browse);
        assert_eq!(names(&monitor), vec!["rustc"], "the filter is still on");
        // And the keys mean what they used to: `m` sorts rather than typing an m.
        press(&mut monitor, KeyCode::Char('m'));
        assert_eq!(monitor.order, Order::Memory);
        assert_eq!(names(&monitor), vec!["rustc"]);
    }

    #[test]
    fn a_letter_that_is_a_sort_key_is_just_a_letter_while_filtering() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        type_text(&mut monitor, "m");
        assert_eq!(monitor.order, Order::Cpu, "the m sorted the table instead of filtering it");
        assert_eq!(monitor.needle, "m");
    }

    #[test]
    fn escape_quits_while_browsing_but_only_clears_the_filter_while_filtering() {
        let mut monitor = app();
        press(&mut monitor, KeyCode::Char('/'));
        assert_eq!(press(&mut monitor, KeyCode::Escape), Flow::Continue);
        assert_eq!(press(&mut monitor, KeyCode::Escape), Flow::Quit);
    }

    // ---- Sampling ------------------------------------------------------------------------

    /// A reading with a given pid and cumulative CPU time, and nothing else worth naming.
    fn reading(pid: u32, used: Option<Duration>) -> Reading {
        Reading {
            pid,
            name: format!("p{pid}"),
            command: String::new(),
            memory: 1024,
            used,
            threads: None,
            state: String::new(),
        }
    }

    fn readings(processes: Vec<Reading>) -> Readings {
        Readings { processes, ..Readings::none() }
    }

    #[test]
    fn the_first_sample_has_no_cpu_figures_because_a_rate_needs_two_readings() {
        let mut sampler = Sampler::new();
        let snapshot =
            sampler.rates(readings(vec![reading(1, Some(Duration::from_secs(9)))]), Instant::now());
        assert_eq!(snapshot.processes[0].cpu, None);
        assert_eq!(snapshot.cpu, None, "and the machine total is unknown, not zero");
    }

    #[test]
    fn cpu_is_the_time_spent_over_the_time_that_passed() {
        let mut sampler = Sampler::new();
        let start = Instant::now();
        sampler.rates(readings(vec![reading(1, Some(Duration::from_secs(10)))]), start);
        // Half a second of CPU in the second that followed: half of one core.
        let later = start + Duration::from_secs(1);
        let snapshot =
            sampler.rates(readings(vec![reading(1, Some(Duration::from_millis(10_500)))]), later);
        let cpu = snapshot.processes[0].cpu.expect("a second reading makes a rate");
        assert!((cpu - 0.5).abs() < 0.001, "got {cpu}");
        let total = snapshot.cpu.expect("a total");
        assert!((total - 0.5 / cores() as f32).abs() < 0.001, "half a core of {}", cores());
    }

    #[test]
    fn a_process_that_used_nothing_reads_zero_rather_than_unknown() {
        let mut sampler = Sampler::new();
        let start = Instant::now();
        let idle = Duration::from_secs(3);
        sampler.rates(readings(vec![reading(1, Some(idle))]), start);
        let snapshot =
            sampler.rates(readings(vec![reading(1, Some(idle))]), start + Duration::from_secs(1));
        assert_eq!(snapshot.processes[0].cpu, Some(0.0), "idle is a measurement");
    }

    #[test]
    fn a_process_that_appeared_since_the_last_sample_has_no_rate_yet() {
        let mut sampler = Sampler::new();
        let start = Instant::now();
        sampler.rates(readings(vec![reading(1, Some(Duration::ZERO))]), start);
        let snapshot = sampler.rates(
            readings(vec![reading(1, Some(Duration::ZERO)), reading(2, Some(Duration::ZERO))]),
            start + Duration::from_secs(1),
        );
        assert_eq!(snapshot.processes[0].cpu, Some(0.0));
        assert_eq!(snapshot.processes[1].cpu, None, "nothing to subtract from");
    }

    #[test]
    fn a_reused_pid_does_not_report_a_negative_rate() {
        let mut sampler = Sampler::new();
        let start = Instant::now();
        sampler.rates(readings(vec![reading(1, Some(Duration::from_secs(400)))]), start);
        // The same pid with less time used than before: the old process died and a new one has it.
        let snapshot = sampler.rates(
            readings(vec![reading(1, Some(Duration::ZERO))]),
            start + Duration::from_secs(1),
        );
        assert_eq!(snapshot.processes[0].cpu, Some(0.0));
    }

    #[test]
    fn a_platform_that_reports_no_cpu_time_leaves_every_figure_unknown() {
        let mut sampler = Sampler::new();
        let start = Instant::now();
        sampler.rates(readings(vec![reading(1, None)]), start);
        let snapshot =
            sampler.rates(readings(vec![reading(1, None)]), start + Duration::from_secs(1));
        assert_eq!(snapshot.processes[0].cpu, None);
        assert_eq!(snapshot.cpu, None);
    }

    #[test]
    fn two_samples_at_the_same_instant_are_not_a_rate() {
        let mut sampler = Sampler::new();
        let now = Instant::now();
        sampler.rates(readings(vec![reading(1, Some(Duration::ZERO))]), now);
        let snapshot = sampler.rates(readings(vec![reading(1, Some(Duration::from_secs(1)))]), now);
        assert_eq!(snapshot.processes[0].cpu, None, "dividing by no time at all");
    }

    #[test]
    fn history_keeps_the_most_recent_readings_and_no_more() {
        let mut history = Vec::new();
        for index in 0..HISTORY + 10 {
            push_history(&mut history, index as f32);
        }
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history[0], 10.0, "the oldest readings went, not the newest");
        assert_eq!(history[HISTORY - 1], (HISTORY + 9) as f32);
    }

    #[test]
    fn only_known_readings_reach_the_charts() {
        let mut monitor = Monitor::new();
        monitor.install(Snapshot::empty());
        assert!(monitor.cpu_history.is_empty(), "a flat line at zero is a claim about the machine");
        monitor.install(Snapshot::fixture());
        assert_eq!(monitor.cpu_history.len(), 1);
    }

    // ---- Pausing -------------------------------------------------------------------------

    #[test]
    fn the_machine_is_asked_once_and_then_once_a_second() {
        let mut monitor = app();
        let start = Instant::now();
        assert!(monitor.is_due(start), "nothing has been sampled yet");
        monitor.sampled = Some(start);
        assert!(!monitor.is_due(start + REFRESH / 2));
        assert!(monitor.is_due(start + REFRESH));
    }

    #[test]
    fn pausing_stops_the_figures_without_stopping_the_frames() {
        let mut monitor = app();
        monitor.sampled = Some(Instant::now());
        press(&mut monitor, KeyCode::Char(' '));
        assert!(monitor.paused);
        assert!(!monitor.is_due(Instant::now() + REFRESH * 10), "a pause that samples anyway");
        assert!(screen(&monitor, 100, 24).contains("PAUSED"), "and says so");
        // The cursor still moves over the frozen sample, which is the point of freezing it.
        press(&mut monitor, KeyCode::Down);
        assert_eq!(monitor.selection.selected(), 1);
    }

    // ---- Mouse ---------------------------------------------------------------------------

    /// Draw, then click. Both halves matter: a click resolves against the frame that was drawn, so
    /// a test that skipped the draw would be clicking a screen that never existed.
    fn click_at(monitor: &mut Monitor, column: u16, row: u16) {
        let _ = screen(monitor, 110, 24);
        monitor.handle(&Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: Modifiers::NONE,
        }));
    }

    fn wheel(monitor: &mut Monitor, column: u16, row: u16, down: bool) {
        monitor.handle(&Event::Mouse(MouseEvent {
            kind: if down { MouseKind::ScrollDown } else { MouseKind::ScrollUp },
            column,
            row,
            modifiers: Modifiers::NONE,
        }));
    }

    fn area_of(monitor: &Monitor, zone: Zone) -> Rect {
        let _ = screen(monitor, 110, 24);
        monitor.hits.area_of(zone).expect("the zone drew this frame")
    }

    #[test]
    fn clicking_a_row_selects_that_process() {
        let mut monitor = app();
        let area = area_of(&monitor, Zone::Table);
        click_at(&mut monitor, area.x + 4, area.y + 3);
        assert_eq!(monitor.selection.selected(), 3);
        assert_eq!(focused(&monitor), Some("rustc"));
        assert_eq!(monitor.focus, Some(4832), "and the pid is what is remembered");
    }

    #[test]
    fn clicking_past_the_last_row_changes_nothing() {
        let mut monitor = app();
        monitor.select(1);
        let area = area_of(&monitor, Zone::Table);
        click_at(&mut monitor, area.x + 4, area.y + area.height - 1);
        assert_eq!(monitor.selection.selected(), 1, "the blank space belongs to nobody");
    }

    #[test]
    fn clicking_a_column_heading_sorts_by_it() {
        let mut monitor = app();
        let area = area_of(&monitor, Zone::Heading);
        let indent = monitor.process_list().text_column();
        click_at(&mut monitor, area.x + indent + 1, area.y);
        assert_eq!(monitor.order, Order::Pid, "the leftmost column is PID");
        click_at(&mut monitor, area.x + indent + PID_WIDTH as u16 + 2, area.y);
        assert_eq!(monitor.order, Order::Cpu);
        click_at(&mut monitor, area.x + area.width - 2, area.y);
        assert_eq!(monitor.order, Order::Name, "everything right of MEM is the command");
    }

    #[test]
    fn clicking_the_same_heading_twice_turns_the_column_round() {
        let mut monitor = app();
        let area = area_of(&monitor, Zone::Heading);
        let indent = monitor.process_list().text_column();
        let column = area.x + indent + 1;
        click_at(&mut monitor, column, area.y);
        assert!(!monitor.descending, "a list of pids starts at the lowest");
        click_at(&mut monitor, column, area.y);
        assert!(monitor.descending);
    }

    #[test]
    fn the_wheel_over_the_table_moves_the_cursor() {
        let mut monitor = app();
        let area = area_of(&monitor, Zone::Table);
        wheel(&mut monitor, area.x + 4, area.y + 1, true);
        assert_eq!(monitor.selection.selected(), 1);
        assert_eq!(monitor.focus, Some(182), "and the wheel remembers what it landed on");
        wheel(&mut monitor, area.x + 4, area.y + 1, false);
        assert_eq!(monitor.selection.selected(), 0);
    }

    #[test]
    fn the_wheel_away_from_the_table_is_not_the_tables_business() {
        let mut monitor = app();
        let _ = screen(&monitor, 110, 24);
        wheel(&mut monitor, 105, 2, true); // over the meters
        assert_eq!(monitor.selection.selected(), 0);
    }

    // ---- The scrollbar -------------------------------------------------------------------

    /// Wide, and short enough that nine processes do not fit in the table — which is the only
    /// condition under which there is a thumb at all. At 24 rows the fixture fits and the bar
    /// correctly draws nothing, so a drag test at that size would be testing an empty column.
    const CRAMPED: (u16, u16) = (110, 14);

    /// Draw at [`CRAMPED`], then send one mouse event. The draw is what puts the bar's region in
    /// `hits` and the offset in the selection, and a drag resolves against both.
    fn mouse_at(monitor: &mut Monitor, kind: MouseKind, column: u16, row: u16) {
        let _ = screen(monitor, CRAMPED.0, CRAMPED.1);
        monitor.handle(&Event::Mouse(MouseEvent { kind, column, row, modifiers: Modifiers::NONE }));
    }

    /// The bar's region and its thumb at [`CRAMPED`].
    fn bar(monitor: &Monitor) -> (Rect, std::ops::Range<u16>) {
        let _ = screen(monitor, CRAMPED.0, CRAMPED.1);
        let area = monitor.hits.area_of(Zone::Bar).expect("the bar drew this frame");
        let thumb = Scrollbar::new(monitor.selection.offset(), monitor.visible().len())
            .thumb(area.height)
            .expect("nine processes do not fit in a cramped table");
        (area, thumb)
    }

    #[test]
    fn dragging_the_thumb_scrolls_the_table_and_brings_the_cursor_along() {
        let mut monitor = app();
        let (area, thumb) = bar(&monitor);
        assert_eq!(monitor.selection.offset(), 0, "starts at the top");

        mouse_at(&mut monitor, MouseKind::Down(MouseButton::Left), area.x, area.y + thumb.start);
        assert_eq!(monitor.grab, Some(0), "the thumb is held, by its top row");

        // All the way to the bottom of the bar.
        mouse_at(
            &mut monitor,
            MouseKind::Drag(MouseButton::Left),
            area.x,
            area.y + area.height - 1,
        );
        let offset = monitor.selection.offset();
        assert!(offset > 0, "dragging the thumb down scrolls the table, got offset {offset}");
        // The cursor came with it, because a list whose window follows its cursor has no offset the
        // cursor does not imply — and a selection left behind would be dragged back next frame.
        assert!(
            monitor.selection.selected() >= offset,
            "cursor at {} is above the window starting at {offset}",
            monitor.selection.selected()
        );
        // And the focus is the process actually under the cursor now, not the one it started on.
        assert_eq!(
            monitor.focus,
            monitor.visible().get(monitor.selection.selected()).map(|p| p.pid)
        );
    }

    #[test]
    fn a_drag_past_the_end_of_the_bar_asks_for_the_end_rather_than_sticking() {
        let mut monitor = app();
        let (area, thumb) = bar(&monitor);
        mouse_at(&mut monitor, MouseKind::Down(MouseButton::Left), area.x, area.y + thumb.start);
        mouse_at(&mut monitor, MouseKind::Drag(MouseButton::Left), area.x, area.y + 200);
        let len = monitor.visible().len();
        let furthest = len - usize::from(area.height);
        assert_eq!(
            monitor.selection.offset(),
            furthest,
            "a pointer well past the bar means the last windowful, not a stuck bar"
        );
        // The cursor comes to the nearest edge of that window, which is its top — it is dragged
        // along, not thrown to the end of the list.
        assert_eq!(monitor.selection.selected(), furthest);
    }

    #[test]
    fn a_drag_nothing_grabbed_moves_nothing() {
        let mut monitor = app();
        let (area, _) = bar(&monitor);
        mouse_at(
            &mut monitor,
            MouseKind::Drag(MouseButton::Left),
            area.x,
            area.y + area.height - 1,
        );
        assert_eq!(monitor.selection.offset(), 0, "a drag with no grab is somebody else's drag");
        assert_eq!(monitor.selection.selected(), 0);
    }

    #[test]
    fn releasing_the_button_lets_go_of_the_thumb() {
        let mut monitor = app();
        let (area, thumb) = bar(&monitor);
        mouse_at(&mut monitor, MouseKind::Down(MouseButton::Left), area.x, area.y + thumb.start);
        mouse_at(&mut monitor, MouseKind::Up(MouseButton::Left), area.x, area.y + thumb.start);
        assert_eq!(monitor.grab, None);
        let before = monitor.selection.offset();
        mouse_at(
            &mut monitor,
            MouseKind::Drag(MouseButton::Left),
            area.x,
            area.y + area.height - 1,
        );
        assert_eq!(monitor.selection.offset(), before, "the hand let go");
    }

    #[test]
    fn pressing_the_track_pages_rather_than_jumping() {
        let mut monitor = app();
        let (area, thumb) = bar(&monitor);
        // Below the thumb: a page down, not a leap to wherever the finger landed. A page is the
        // window less a row of context, so pressing the track twice is what it takes to move the
        // window off the top at all with only three rows of it.
        let page = monitor.selection.height() - 1;
        mouse_at(&mut monitor, MouseKind::Down(MouseButton::Left), area.x, area.y + thumb.end);
        assert_eq!(monitor.selection.selected(), page, "paged, rather than jumped to the pointer");
        assert_eq!(monitor.grab, None, "the track is not the thumb");

        let (area, thumb) = bar(&monitor);
        mouse_at(&mut monitor, MouseKind::Down(MouseButton::Left), area.x, area.y + thumb.end);
        assert_eq!(monitor.selection.selected(), page * 2);

        // And back up, from the track above the thumb.
        let (area, thumb) = bar(&monitor);
        assert!(thumb.start > 0, "the window has moved, so there is track above the thumb");
        mouse_at(&mut monitor, MouseKind::Down(MouseButton::Left), area.x, area.y);
        assert_eq!(monitor.selection.selected(), page, "paged back up");
    }

    #[test]
    fn the_wheel_over_the_bar_scrolls_the_table_it_belongs_to() {
        let mut monitor = app();
        let (area, _) = bar(&monitor);
        let _ = screen(&monitor, CRAMPED.0, CRAMPED.1);
        wheel(&mut monitor, area.x, area.y + 1, true);
        assert_eq!(
            monitor.selection.selected(),
            1,
            "a column away from the rows is still the list"
        );
    }

    #[test]
    fn a_table_with_nothing_to_scroll_has_no_bar_to_press() {
        // At full height the fixture fits, `Scrollbar` draws nothing, and a press in the column
        // where a thumb would have been must not page the list.
        let mut monitor = app();
        let _ = screen(&monitor, 110, 24);
        let area = monitor.hits.area_of(Zone::Bar).expect("the region is still recorded");
        click_at(&mut monitor, area.x, area.y + 1);
        assert_eq!(monitor.selection.selected(), 0);
        assert_eq!(monitor.grab, None);
    }

    // ---- Formatting ----------------------------------------------------------------------

    #[test]
    fn byte_counts_read_as_figures_rather_than_as_digits() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1024), "1 KB");
        assert_eq!(bytes(12_582_912), "12 MB");
        assert_eq!(bytes(1_181_116_006), "1.1 GB");
        assert_eq!(bytes(17_179_869_184), "16 GB");
    }

    #[test]
    fn an_uptime_gains_a_day_count_only_once_there_is_one() {
        assert_eq!(duration(Duration::from_secs(0)), "00:00:00");
        assert_eq!(duration(Duration::from_secs(3661)), "01:01:01");
        assert_eq!(duration(Duration::from_secs(86_400)), "1d 00:00");
        assert_eq!(duration(Duration::from_secs(4 * 86_400 + 2 * 3600 + 11 * 60)), "4d 02:11");
    }

    #[test]
    fn a_process_using_more_than_one_core_says_so() {
        assert_eq!(cpu_text(Some(0.0)), "0.0");
        assert_eq!(cpu_text(Some(0.124)), "12.4");
        assert_eq!(cpu_text(Some(3.5)), "350.0", "eight cores can be busier than one");
        assert_eq!(cpu_text(None), UNKNOWN);
    }

    #[test]
    fn a_state_nobody_here_has_documented_is_passed_through() {
        assert_eq!(state_text("R"), "running");
        assert_eq!(state_text("Ss"), "sleeping");
        assert_eq!(state_text("Q"), "Q");
        assert_eq!(state_text(""), UNKNOWN);
    }

    #[test]
    fn a_click_on_the_heading_resolves_to_the_column_under_it() {
        // The boundaries, which is where an off-by-one would hide.
        assert_eq!(column_at(2, 2), Order::Pid);
        assert_eq!(column_at(2 + PID_WIDTH as u16 - 1, 2), Order::Pid);
        assert_eq!(column_at(2 + PID_WIDTH as u16, 2), Order::Cpu);
        assert_eq!(column_at(2 + (PID_WIDTH + CPU_WIDTH) as u16 + 1, 2), Order::Memory);
        assert_eq!(column_at(200, 2), Order::Name);
        assert_eq!(column_at(0, 2), Order::Pid, "left of the indent is still the first column");
    }

    // ---- The platform underneath ---------------------------------------------------------

    /// `readings` above is a fixture helper of the same name, so the real one needs saying in full.
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn from_the_machine() -> Readings {
        super::readings()
    }

    /// The one test that reads the real machine, and the only thing that can say whether the parsing
    /// still matches what this platform prints. It runs on all three in CI.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn the_platform_sampler_can_see_this_very_process() {
        let readings = from_the_machine();
        assert!(!readings.processes.is_empty(), "{SOURCE} listed nothing at all");
        let mine = std::process::id();
        assert!(
            readings.processes.iter().any(|process| process.pid == mine),
            "{SOURCE} did not list the process ({mine}) that is asking"
        );
        assert!(
            readings.processes.iter().all(|process| !process.name.is_empty()),
            "a process with no name means the fields came apart"
        );
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn the_platform_knows_how_much_memory_there_is_and_how_much_is_gone() {
        let readings = from_the_machine();
        let total = readings.memory_total.expect("a total");
        let used = readings.memory_used.expect("a figure for memory in use");
        assert!(total > 128 << 20, "{total} bytes of RAM is not a machine that runs this");
        assert!(used > 0 && used <= total, "{used} of {total}");
        let uptime = readings.uptime.expect("an uptime");
        assert!(uptime > Duration::from_secs(1), "the machine booted {uptime:?} ago");
    }

    /// Two samples of the live machine, which is the only way to get a rate at all.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn sampling_the_real_machine_twice_produces_a_believable_figure() {
        let mut sampler = Sampler::new();
        let start = Instant::now();
        sampler.rates(from_the_machine(), start);
        // No sleeping: the interval is asserted rather than waited for, so the test costs two
        // samples and not a second.
        let snapshot = sampler.rates(from_the_machine(), start + Duration::from_secs(1));
        let cpu = snapshot.cpu.expect("a total after two samples");
        assert!((0.0..=1.0).contains(&cpu), "a machine cannot be {cpu} busy");
        assert!(
            snapshot.processes.iter().any(|process| process.cpu.is_some()),
            "not one process had a rate"
        );
    }
}
