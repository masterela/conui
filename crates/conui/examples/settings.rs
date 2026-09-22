//! A settings screen: tabs, dropdowns, a text field, and two buttons.
//!
//! The other two examples are keyboard apps where every key means one thing. This one is the
//! awkward case that makes a toolkit prove itself: several controls on screen at once, only one of
//! them listening, and a list that has to paint over the top of everything else.
//!
//! Three things are worth watching:
//!
//! - **Focus is a value.** [`Focus<Id>`] is a ring of your own enum, and each widget is *told*
//!   whether it is focused. Nothing registers itself during render, so what has focus never
//!   depends on what happened to draw last frame.
//! - **Modality is an early return.** An open dropdown is handled before anything else and
//!   swallows every key, including `Tab`. That is the whole of it; there is no modal stack.
//! - **The overlay is a second pass.** A view is clipped to its own region and cannot escape it,
//!   so the open list is not a child of the field. The field records where it landed, and the
//!   frame draws the list on top afterwards.
//! - **A click is resolved against last frame.** [`Hits<Id>`] collects where each control landed
//!   while composing, and the mouse handler asks it what is under the pointer. Focus is declared;
//!   hit-testing is observed. They are two structures because they answer two different questions.
//! - **Scrolling is a shifted origin, not a re-layout.** The ABOUT tab holds more text than fits.
//!   It is drawn in full, starting above the top of the pane, and clipped — so nothing in it knows
//!   it is scrolled. What makes it possible is that the content can say how tall it is: a plain
//!   column stretches to its parent, and `fit()` is the opt-in that makes it add itself up instead.
//!
//! Applying really does re-theme the running app, so the buttons are not decoration. The whole
//! screen is usable with the mouse as well: click a tab, a field or a button, click a dropdown to
//! open it, click an option to choose it, click anywhere else to dismiss it.
//!
//! ```sh
//! cargo run -p conui --example settings
//! cargo run -p conui --example settings -- --dump 88 24
//! ```

use std::cell::Cell;
use std::io;

use conui::view::{Column, Paint, Row, Scroll, Spacer, ViewExt};
use conui::widget::{
    Border, Button, Field, Gauge, Hints, Menu, Panel, Readout, Select, Tabs, Text,
};
use conui::widget::{Input, Rule};
use conui::{
    App, BarStyle, Buffer, Config, Dropdown, Editor, Event, Focus, Frame, Hits, KeyCode,
    MouseEvent, Pos, Rect, Role, Selection, Style, Theme, View, Viewport,
};

const MIN_WIDTH: u16 = 66;
const MIN_HEIGHT: u16 = 18;
/// Column the value of a form row starts at, so every label and control lines up.
const LABEL_WIDTH: u16 = 12;
/// Width of a select field. Wide enough for the longest choice plus its brackets and caret.
const CONTROL_WIDTH: u16 = 22;
const PREVIEW_WIDTH: u16 = 30;

const TABS: [&str; 3] = ["APPEARANCE", "LAYOUT", "ABOUT"];

/// The ABOUT prose, broken by hand rather than word-wrapped.
///
/// Wrapped text is worth its cost when the content is unknown, but its height then depends on the
/// width it is given, and the thing measuring this pane wants a number. A `Text` counts its own
/// newlines, so hard breaks make the content height exact at any width.
const ABOUT: &str = "\
A devkit for console applications with a proper user
interface, in pure Rust. No curses, no crossterm, no
ratatui: the backend is ours, and the dependency list is
rustix, unicode-width and unicode-segmentation.";

const LAYERS: &str = "\
Two APIs over one renderer. Underneath is an immediate
canvas you draw on; over it is a tree of views that lay
themselves out. They meet at one method: a view is handed
a canvas clipped to its own region and cannot escape it.";

const STATE: &str = "\
Views are stateless. Anything with a cursor, a selection
or an open/closed flag borrows a plain struct that your
app owns, for the frame. Nothing registers itself while
drawing, so what has focus never depends on draw order.";

const SCROLLING: &str = "\
This pane is the example. The text below the fold is
drawn in full, from an origin above the visible region,
and the clip throws away the rest — nothing is re-laid
out and the paragraphs cannot tell they are half off
screen. Wheel, arrows, PAGE UP/DOWN, HOME and END.";
const PALETTES: [&str; 3] = ["LAYA", "EMBER", "INHERIT"];
const BARS: [&str; 4] = ["Rule", "Shaded", "Blocks", "Smooth"];
const DENSITY: [&str; 3] = ["Compact", "Comfortable", "Spacious"];
const SIDEBAR: [&str; 3] = ["Left", "Right", "Hidden"];

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("settings — a conui example");
            println!();
            println!("  --dump [width] [height]   print one composed frame as text and exit");
            println!("  --dump --open             the same frame with a dropdown showing");
            println!("  --dump --tab N            the same frame on tab N");
            println!("  --help                    this");
            Ok(())
        }
        Some("--dump") => {
            let width = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(88);
            let height = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(24);
            let tab = args
                .iter()
                .position(|arg| arg == "--tab")
                .and_then(|at| args.get(at + 1))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            dump(width, height, args.iter().any(|arg| arg == "--open"), tab);
            Ok(())
        }
        _ => run(),
    }
}

fn run() -> io::Result<()> {
    let mut ui = Ui::new();
    let mut app = App::with(
        Config::new().theme(ui.theme()).fps(20).min_size(MIN_WIDTH, MIN_HEIGHT).mouse(true),
    )?;

    while app.is_running() {
        for event in app.poll()? {
            match ui.handle(&event) {
                Flow::Quit => app.quit(),
                // Applying is the one action that reaches past the screen into the app itself.
                Flow::Retheme => app.set_theme(ui.theme()),
                Flow::Continue => {}
            }
        }
        app.draw(|frame| ui.compose(frame))?;
    }
    app.leave()
}

/// Render one frame into memory, which is how the layout is checked and how the tests read it.
fn dump(width: u16, height: u16, open: bool, tab: usize) {
    let mut ui = Ui::demo();
    ui.tab.set_selected(tab.min(TABS.len() - 1));
    ui.retarget();
    if open {
        ui.palette.open();
    }
    let mut buffer = Buffer::new(width, height);
    let mut frame = Frame::new(&mut buffer, ui.theme());
    ui.compose(&mut frame);
    for row in 0..buffer.height() {
        println!("{}", buffer.row_text(row).trim_end());
    }
}

// ---- The screen --------------------------------------------------------------------------

/// Everything that can hold focus. Naming these in one enum is the whole focus "system": the ring
/// is a list of these, and `focus.is(Id::Apply)` is how the Apply button learns it is highlighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Id {
    Tabs,
    Palette,
    Bars,
    Label,
    Density,
    Sidebar,
    /// The scrollable pane on the ABOUT tab. Focusable so the keyboard can scroll it, and
    /// hit-tested so the wheel knows the pointer is over it rather than over the form.
    About,
    Revert,
    Apply,
}

/// What the key handler wants the loop to do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Retheme,
    Quit,
}

/// The values the screen edits, together, so "has anything changed?" is one comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Values {
    palette: usize,
    bars: usize,
    label: String,
    density: usize,
    sidebar: usize,
}

struct Ui {
    tab: Selection,
    focus: Focus<Id>,
    palette: Dropdown,
    bars: Dropdown,
    label: Editor,
    density: Dropdown,
    sidebar: Dropdown,
    /// How far the ABOUT pane has been scrolled. Unlike a [`Selection`] there is no cursor for a
    /// window to follow here, so the offset genuinely is the state — and because it is, the wheel
    /// over this pane does what a wheel normally does instead of moving a highlight.
    about: Viewport,
    /// The last applied values, for the dirty marker and for Revert.
    saved: Values,
    status: String,
    status_role: Role,
    /// Where each control landed last frame, so a click can be resolved back to one.
    hits: Hits<Id>,
    /// The frame the last compose drew into. A click has to be resolved against the same screen
    /// the overlay was placed against, and only the draw knows how big that was.
    screen: Cell<Rect>,
}

impl Ui {
    fn new() -> Self {
        let saved = Values {
            palette: 0,
            bars: 1,
            label: String::from("LOCAL INTELLIGENCE"),
            density: 1,
            sidebar: 0,
        };
        let mut ui = Self {
            tab: Selection::new(),
            focus: Focus::new([Id::Tabs]),
            palette: Dropdown::at(saved.palette),
            bars: Dropdown::at(saved.bars),
            label: Editor::with(&saved.label),
            density: Dropdown::at(saved.density),
            sidebar: Dropdown::at(saved.sidebar),
            about: Viewport::new(),
            saved,
            status: String::from("ready"),
            status_role: Role::Muted,
            hits: Hits::new(),
            screen: Cell::new(Rect::sized(MIN_WIDTH, MIN_HEIGHT)),
        };
        ui.retarget();
        ui
    }

    /// The state the `--dump` and the tests start from: a pending edit, so the dirty marker and
    /// the Revert button are both doing something.
    fn demo() -> Self {
        let mut ui = Self::new();
        ui.bars.set_selected(3);
        ui.focus.focus(Id::Palette);
        ui
    }

    // ---- Values ------------------------------------------------------------------------

    fn values(&self) -> Values {
        Values {
            palette: self.palette.selected(),
            bars: self.bars.selected(),
            label: self.label.value().to_owned(),
            density: self.density.selected(),
            sidebar: self.sidebar.selected(),
        }
    }

    fn is_modified(&self) -> bool {
        self.values() != self.saved
    }

    /// The theme the *applied* palette names. The draft palette deliberately does not take effect
    /// until Apply — a settings screen that re-themes itself under your cursor is a toy.
    fn theme(&self) -> Theme {
        match self.saved.palette {
            1 => Theme::EMBER,
            2 => Theme::INHERIT,
            _ => Theme::LAYA,
        }
    }

    fn bar_style(&self) -> BarStyle {
        match self.bars.selected() {
            1 => BarStyle::Shaded,
            2 => BarStyle::Blocks,
            3 => BarStyle::Smooth,
            _ => BarStyle::Rule,
        }
    }

    /// The palette being previewed, which *is* the draft one — a swatch that showed the applied
    /// colours would be a preview of nothing.
    fn draft_theme(&self) -> Theme {
        match self.palette.selected() {
            1 => Theme::EMBER,
            2 => Theme::INHERIT,
            _ => Theme::LAYA,
        }
    }

    fn apply(&mut self) -> Flow {
        if !self.is_modified() {
            self.note("no changes to apply", Role::Muted);
            return Flow::Continue;
        }
        self.saved = self.values();
        self.note("applied", Role::Accent);
        Flow::Retheme
    }

    fn revert(&mut self) {
        // Guarded here rather than at each call site: the button is drawn disabled when there is
        // nothing to revert, and both `Enter` and a click have to agree with what it looks like.
        if !self.is_modified() {
            self.note("nothing to revert", Role::Muted);
            return;
        }
        let saved = self.saved.clone();
        self.palette.set_selected(saved.palette);
        self.bars.set_selected(saved.bars);
        self.label.set_value(&saved.label);
        self.density.set_selected(saved.density);
        self.sidebar.set_selected(saved.sidebar);
        self.note("reverted", Role::Info);
    }

    fn note(&mut self, message: impl Into<String>, role: Role) {
        self.status = message.into();
        self.status_role = role;
    }

    // ---- Focus -------------------------------------------------------------------------

    /// Rebuild the focus ring for the current tab.
    ///
    /// Called on every tab change, because the focusable set genuinely differs per tab. `set_ring`
    /// keeps focus where it is when the control still exists, so tabbing away and back does not
    /// throw you to the top of the screen.
    fn retarget(&mut self) {
        let fields: &[Id] = match self.tab.selected() {
            0 => &[Id::Palette, Id::Bars, Id::Label],
            1 => &[Id::Density, Id::Sidebar],
            _ => &[Id::About],
        };
        let ring: Vec<Id> = std::iter::once(Id::Tabs)
            .chain(fields.iter().copied())
            .chain([Id::Revert, Id::Apply])
            .collect();
        self.focus.set_ring(ring);
    }

    /// The dropdown behind a focus id, and the choices it is showing.
    fn dropdown(&self, id: Id) -> Option<(&Dropdown, &'static [&'static str])> {
        match id {
            Id::Palette => Some((&self.palette, &PALETTES)),
            Id::Bars => Some((&self.bars, &BARS)),
            Id::Density => Some((&self.density, &DENSITY)),
            Id::Sidebar => Some((&self.sidebar, &SIDEBAR)),
            _ => None,
        }
    }

    fn dropdown_mut(&mut self, id: Id) -> Option<&mut Dropdown> {
        match id {
            Id::Palette => Some(&mut self.palette),
            Id::Bars => Some(&mut self.bars),
            Id::Density => Some(&mut self.density),
            Id::Sidebar => Some(&mut self.sidebar),
            _ => None,
        }
    }

    /// The one open list, if any. Only the focused dropdown can be open, so this is unambiguous.
    fn open_menu(&self) -> Option<(&Dropdown, &'static [&'static str])> {
        let id = self.focus.current()?;
        self.dropdown(id).filter(|(dropdown, _)| dropdown.is_open())
    }

    // ---- Input -------------------------------------------------------------------------

    fn handle(&mut self, event: &Event) -> Flow {
        // An open list covers other controls, so it must be dismissed before anything else can be
        // reached. This early return is the entire modality mechanism: it takes every key, `Tab`
        // included, and nothing below it runs.
        if let Some(id) = self.focus.current() {
            if self.dropdown(id).is_some_and(|(dropdown, _)| dropdown.is_open()) {
                let len = self.dropdown(id).map_or(0, |(_, options)| options.len());
                let screen = self.screen.get();
                if let Some(key) = event.as_key() {
                    if let Some(dropdown) = self.dropdown_mut(id) {
                        dropdown.handle(key, len);
                    }
                } else if let Some(mouse) = event.as_mouse() {
                    // The same rule for the mouse: a click on an option chooses it, a click
                    // anywhere else dismisses the list and goes no further.
                    if let Some(dropdown) = self.dropdown_mut(id) {
                        dropdown.handle_mouse(mouse, len, screen);
                    }
                }
                return Flow::Continue;
            }
        }

        if let Some(mouse) = event.as_mouse() {
            return self.handle_mouse(mouse);
        }

        let Some(key) = event.as_key() else {
            if let (Event::Paste(text), Some(Id::Label)) = (event, self.focus.current()) {
                self.label.insert_str(text);
            }
            return Flow::Continue;
        };

        if self.focus.handle(key) {
            return Flow::Continue;
        }
        let focused = self.focus.current();

        // `q` is a shortcut everywhere except inside the text field, where it is a letter. There is
        // no way for a framework to decide that for you, which is why it is decided here.
        if focused != Some(Id::Label) && key.is_key('q') {
            return Flow::Quit;
        }
        if key.code == KeyCode::Escape {
            return Flow::Quit;
        }

        match focused {
            Some(Id::Tabs) => {
                match key.code {
                    KeyCode::Left => self.tab.cycle_up(TABS.len()),
                    KeyCode::Right => self.tab.cycle_down(TABS.len()),
                    KeyCode::Char('h') => self.tab.cycle_up(TABS.len()),
                    KeyCode::Char('l') => self.tab.cycle_down(TABS.len()),
                    _ => return Flow::Continue,
                }
                self.retarget();
            }
            Some(Id::Label) => {
                self.label.handle(key);
            }
            Some(Id::About) => {
                // Scrolled, not navigated: there is nothing in the pane to select. None of this
                // needs to know how long the text is or how tall the pane was — the viewport
                // clamps against what the last frame actually showed, which is the only thing
                // that knows.
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => self.about.scroll(-1),
                    KeyCode::Down | KeyCode::Char('j') => self.about.scroll(1),
                    KeyCode::PageUp => self.about.page_up(),
                    KeyCode::PageDown => self.about.page_down(),
                    KeyCode::Home => self.about.top(),
                    KeyCode::End => self.about.bottom(),
                    _ => {}
                }
            }
            Some(Id::Revert) if key.code == KeyCode::Enter => self.revert(),
            Some(Id::Apply) if key.code == KeyCode::Enter => return self.apply(),
            Some(id) => {
                let len = self.dropdown(id).map_or(0, |(_, options)| options.len());
                // Left and right step a closed select without opening it. The dropdown itself
                // deliberately does not claim the arrow keys while closed, so this is the screen's
                // call to make rather than the widget's.
                match key.code {
                    KeyCode::Left => {
                        if let Some(dropdown) = self.dropdown_mut(id) {
                            let index = dropdown.selected();
                            dropdown.set_selected(if index == 0 { len - 1 } else { index - 1 });
                        }
                    }
                    KeyCode::Right => {
                        if let Some(dropdown) = self.dropdown_mut(id) {
                            let index = (dropdown.selected() + 1) % len.max(1);
                            dropdown.set_selected(index);
                        }
                    }
                    _ => {
                        if let Some(dropdown) = self.dropdown_mut(id) {
                            dropdown.handle(key, len);
                        }
                    }
                }
            }
            None => {}
        }
        Flow::Continue
    }

    /// A click, resolved against where things were drawn last frame.
    ///
    /// The wheel belongs to whatever is under the pointer, and it does not move focus: scrolling
    /// something to read it is not the same as saying "talk to this one". Over anything that is not
    /// the ABOUT pane it does nothing at all, because a form that changes a value because the
    /// pointer happened to be over it is a bug with a friendly face.
    fn handle_mouse(&mut self, mouse: &MouseEvent) -> Flow {
        let at = Pos::new(mouse.column, mouse.row);
        if let Some(delta) = mouse.kind.scroll() {
            if self.hits.at(at) == Some(Id::About) {
                self.about.scroll(delta);
            }
            return Flow::Continue;
        }
        if !mouse.is_click() {
            return Flow::Continue;
        }
        let Some(id) = self.hits.at(at) else {
            // Chrome: the header, a rule, the preview panel. Nothing to do, and deliberately not
            // "clear the focus" — losing your place because you clicked a border would be rude.
            return Flow::Continue;
        };
        // Focus follows the click. Clicking a control is the plainest possible way of saying "talk
        // to this one", and it means the keyboard picks up exactly where the mouse left off.
        self.focus.focus(id);

        match id {
            Id::Tabs => {
                // Which label, not just "the tab bar": the bar is one region with several targets,
                // so the widget resolves the column and the caller supplies it.
                let local = self.hits.local(Id::Tabs, at);
                if let Some(index) = local.and_then(|local| self.tab_bar().index_at(local.x)) {
                    self.tab.set_selected(index);
                    self.retarget();
                }
            }
            Id::Label => {} // Focus only. Placing the caret would want Editor to take a column.
            // Focus only, so the arrow keys carry on scrolling from where the wheel stopped.
            Id::About => {}
            Id::Revert => self.revert(),
            Id::Apply => return self.apply(),
            select => {
                let len = self.dropdown(select).map_or(0, |(_, options)| options.len());
                let screen = self.screen.get();
                // The dropdown decides what a click on its own field means, the same way it
                // decides what `Enter` means. The screen only says where the click was.
                if let Some(dropdown) = self.dropdown_mut(select) {
                    dropdown.handle_mouse(mouse, len, screen);
                }
            }
        }
        Flow::Continue
    }

    // ---- Drawing -----------------------------------------------------------------------

    fn compose(&self, frame: &mut Frame<'_>) {
        // Last frame's geometry is gone the moment this one starts. A stale hit points at where a
        // control used to be, which is the bug that makes a UI feel haunted.
        self.hits.clear();
        self.screen.set(frame.area());

        let heading = if self.is_modified() { "MODIFIED" } else { "IN SYNC" };
        let screen = Column::new()
            .child(
                Row::new()
                    .child(Text::new("CONUI  /  SETTINGS").accent().flex(1))
                    .child(Text::new(heading).muted().right().flex(1))
                    .length(1),
            )
            .child(self.tab_bar().hit(&self.hits, Id::Tabs).length(2))
            .child(Spacer::new().length(1))
            .child(self.body().flex(1))
            .child(Rule::new().length(1))
            .child(self.footer().length(1))
            .padding(conui::Padding::xy(2, 1));
        frame.render_full(&screen);

        // Second pass, over the top of everything the first pass drew. Nothing about `Menu` knows
        // it is an overlay — it is on top because it is drawn last.
        if let Some((dropdown, options)) = self.open_menu() {
            let area = dropdown.popup_area(options.len(), frame.area());
            frame.render(&Menu::new(dropdown.selection(), options.iter().copied()), area);
        }
    }

    /// The tab bar, built once and used twice: composed into the frame, and asked by the click
    /// handler which label a column belongs to. Two constructions would drift the first time the
    /// gap changed, and the drift would show up as clicks landing on the wrong tab.
    fn tab_bar(&self) -> Tabs<'_> {
        Tabs::new(TABS).selection(&self.tab).focused(self.focus.is(Id::Tabs))
    }

    fn body(&self) -> Box<dyn View + '_> {
        match self.tab.selected() {
            0 => Box::new(
                Row::new()
                    .gap(3)
                    .child(
                        Column::new()
                            .gap(1)
                            .child(self.select_row("Palette", Id::Palette, &PALETTES))
                            .child(self.select_row("Bars", Id::Bars, &BARS))
                            .child(self.label_row())
                            .child(Spacer::new().flex(1))
                            .child(self.buttons().length(1))
                            .flex(1),
                    )
                    .child(self.preview().length(PREVIEW_WIDTH))
                    .flex(1),
            ),
            1 => Box::new(
                Column::new()
                    .gap(1)
                    .child(self.select_row("Density", Id::Density, &DENSITY))
                    .child(self.select_row("Sidebar", Id::Sidebar, &SIDEBAR))
                    .child(Spacer::new().flex(1))
                    .child(self.buttons().length(1)),
            ),
            _ => Box::new(
                Column::new()
                    .gap(1)
                    .child(
                        Scroll::new(&self.about, self.about_text())
                            .hit(&self.hits, Id::About)
                            .flex(1),
                    )
                    .child(self.buttons().length(1)),
            ),
        }
    }

    /// The ABOUT text: more of it than fits, on purpose.
    ///
    /// `fit()` is the whole trick. A plain `Column` asks for a share of its parent, which is
    /// another way of saying "I will be whatever height you give me" — and something that adapts to
    /// its region has nothing to scroll. A fitted one adds its children up and asks for *that*, so
    /// [`Scroll`] has two numbers to compare. Every child here reports its own height without being
    /// told: a `Text` counts its lines, a `Rule` is one row, and the nested column adds up its two.
    fn about_text(&self) -> Column<'_> {
        Column::new()
            .gap(1)
            .child(Text::new("conui").accent())
            .child(Text::new(ABOUT).muted())
            .child(
                Column::new()
                    .child(Field::new("VERSION", env!("CARGO_PKG_VERSION")).value_column(10))
                    .child(Field::new("LICENSE", "MIT OR Apache-2.0").value_column(10))
                    .fit(),
            )
            .child(Rule::titled("LAYERS"))
            .child(Text::new(LAYERS).muted())
            .child(Rule::titled("STATE"))
            .child(Text::new(STATE).muted())
            .child(Rule::titled("SCROLLING"))
            .child(Text::new(SCROLLING).muted())
            // A column of clear air between the text and the scrollbar. `Scroll` reserves the bar's
            // column before the content is laid out rather than painting over it afterwards, so
            // without this the rules would run right up against the track and read as joined to it.
            .padding(conui::Padding { right: 1, ..conui::Padding::ZERO })
            .fit()
    }

    /// One form row: a label, then a select. Both need an explicit width, because in a `Row` a
    /// view's `constraint()` is read as a *width* and a widget's default is its height.
    fn select_row(&self, label: &str, id: Id, options: &'static [&'static str]) -> impl View + '_ {
        let dropdown = self.dropdown(id).map(|(dropdown, _)| dropdown).expect("a select id");
        Row::new()
            .child(Text::new(label).muted().length(LABEL_WIDTH))
            .child(
                Select::new(dropdown, options.iter().copied())
                    .focused(self.focus.is(id))
                    .hit(&self.hits, id)
                    .length(CONTROL_WIDTH),
            )
            .child(Spacer::new().flex(1))
            .length(1)
    }

    fn label_row(&self) -> impl View + '_ {
        Row::new()
            .child(Text::new("Heading").muted().length(LABEL_WIDTH))
            .child(
                Input::new(&self.label)
                    .placeholder("untitled")
                    .hit(&self.hits, Id::Label)
                    .length(CONTROL_WIDTH),
            )
            .child(Spacer::new().flex(1))
            .length(1)
    }

    fn buttons(&self) -> Row<'_> {
        let modified = self.is_modified();
        let mut revert = Button::new("Revert").focused(self.focus.is(Id::Revert));
        if !modified {
            revert = revert.disabled();
        }
        Row::new()
            .gap(2)
            .child(revert.hit(&self.hits, Id::Revert).length(Button::width("Revert")))
            .child(
                Button::new("Apply")
                    .accent()
                    .focused(self.focus.is(Id::Apply))
                    .hit(&self.hits, Id::Apply)
                    .length(Button::width("Apply")),
            )
            .child(Spacer::new().flex(1))
    }

    /// The draft palette, drawn with the draft palette's own colours.
    ///
    /// A `Role` resolves against the *frame's* theme, which is the applied one — so previewing a
    /// different theme is exactly the case roles cannot express, and exactly what the canvas
    /// escape hatch is for.
    fn preview(&self) -> Panel<'_> {
        let draft = self.draft_theme();
        let swatches = Paint::new(move |canvas: &mut conui::Canvas<'_>| {
            let colours = [
                ("accent", draft.accent),
                ("warn", draft.warn),
                ("danger", draft.danger),
                ("info", draft.info),
                ("text", draft.text),
                ("muted", draft.muted),
            ];
            for (row, (name, colour)) in colours.iter().enumerate() {
                let y = row as i32;
                canvas.put_styled(0, y, "████", Style::new().fg(*colour).bg(draft.background));
                canvas.put(5, y, name, Role::Muted);
            }
        });

        Panel::new("PREVIEW")
            .border(Border::Line)
            .padding(conui::Padding::xy(1, 0))
            .child(swatches.length(6))
            .child(Spacer::new().length(1))
            .child(
                Gauge::new(0.62)
                    .label("BARS")
                    .label_width(6)
                    .style(self.bar_style())
                    .readout(Readout::Percent)
                    .length(1),
            )
            .child(Spacer::new().length(1))
            .child(Text::new(self.heading_preview()).accent().length(1))
            .child(Spacer::new().flex(1))
    }

    fn heading_preview(&self) -> String {
        if self.label.is_empty() {
            String::from("(no heading)")
        } else {
            self.label.value().to_owned()
        }
    }

    fn footer(&self) -> Row<'_> {
        let hints = match self.focus.current() {
            Some(Id::Tabs) => Hints::new().key("←/→", "tab").key("TAB", "next"),
            Some(Id::Label) => Hints::new().key("TYPE", "edit").key("TAB", "next"),
            Some(Id::About) => {
                Hints::new().key("↑/↓", "scroll").key("PGUP/PGDN", "page").key("TAB", "next")
            }
            Some(Id::Revert | Id::Apply) => Hints::new().key("↵", "press").key("TAB", "next"),
            _ => Hints::new().key("←/→", "change").key("↵", "open").key("TAB", "next"),
        };
        Row::new()
            .child(hints.key("ESC", "quit").flex(1))
            .child(Text::new(&self.status).role(self.status_role).right().flex(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conui::{KeyEvent, Rect};

    fn press(ui: &mut Ui, code: KeyCode) -> Flow {
        ui.handle(&Event::Key(KeyEvent::plain(code)))
    }

    fn tab_key(ui: &mut Ui) {
        press(ui, KeyCode::Tab);
    }

    fn screen(ui: &Ui, width: u16, height: u16) -> String {
        let mut buffer = Buffer::new(width, height);
        let mut frame = Frame::new(&mut buffer, ui.theme());
        ui.compose(&mut frame);
        (0..buffer.height()).map(|row| buffer.row_text(row)).collect::<Vec<_>>().join("\n")
    }

    /// Draw, then click where something actually is. Both halves matter: a click resolves against
    /// last frame's geometry, so a test that skipped the draw would be clicking at an empty screen.
    fn click_at(ui: &mut Ui, column: u16, row: u16) -> Flow {
        let _ = screen(ui, 88, 24);
        ui.handle(&Event::Mouse(MouseEvent {
            kind: conui::MouseKind::Down(conui::MouseButton::Left),
            column,
            row,
            modifiers: conui::Modifiers::NONE,
        }))
    }

    /// The middle of a control, from where it drew itself.
    fn centre_of(ui: &mut Ui, id: Id) -> (u16, u16) {
        let _ = screen(ui, 88, 24);
        let area = ui.hits.area_of(id).unwrap_or_else(|| panic!("{id:?} did not draw"));
        (area.x + area.width / 2, area.y + area.height / 2)
    }

    // ---- Focus ---------------------------------------------------------------------------

    #[test]
    fn tab_walks_the_ring_and_wraps() {
        let mut ui = Ui::new();
        assert_eq!(ui.focus.current(), Some(Id::Tabs));
        for expected in [Id::Palette, Id::Bars, Id::Label, Id::Revert, Id::Apply, Id::Tabs] {
            tab_key(&mut ui);
            assert_eq!(ui.focus.current(), Some(expected));
        }
    }

    #[test]
    fn shift_tab_walks_it_backwards() {
        let mut ui = Ui::new();
        ui.handle(&Event::Key(KeyEvent::plain(KeyCode::BackTab)));
        assert_eq!(ui.focus.current(), Some(Id::Apply));
    }

    #[test]
    fn switching_tabs_changes_which_controls_are_focusable() {
        let mut ui = Ui::new();
        assert!(ui.focus.ring().contains(&Id::Palette));
        press(&mut ui, KeyCode::Right);
        assert_eq!(ui.tab.selected(), 1);
        assert!(!ui.focus.ring().contains(&Id::Palette));
        assert!(ui.focus.ring().contains(&Id::Density));
    }

    #[test]
    fn a_tab_switch_keeps_focus_where_it_was_when_the_control_survives() {
        let mut ui = Ui::new();
        // Revert exists on every tab, so focus should still be on it after switching.
        ui.focus.focus(Id::Revert);
        press(&mut ui, KeyCode::Right);
        assert_eq!(ui.focus.current(), Some(Id::Revert));
    }

    // ---- Dropdowns -----------------------------------------------------------------------

    #[test]
    fn enter_opens_a_select_and_the_arrows_then_move_within_it() {
        let mut ui = Ui::new();
        tab_key(&mut ui);
        assert_eq!(ui.focus.current(), Some(Id::Palette));
        press(&mut ui, KeyCode::Enter);
        assert!(ui.palette.is_open());
        press(&mut ui, KeyCode::Down);
        assert_eq!(ui.palette.selected(), 1);
        press(&mut ui, KeyCode::Enter);
        assert!(!ui.palette.is_open());
        assert_eq!(ui.palette.selected(), 1);
    }

    #[test]
    fn an_open_list_swallows_tab_so_focus_cannot_walk_out_from_under_it() {
        let mut ui = Ui::new();
        ui.focus.focus(Id::Palette);
        press(&mut ui, KeyCode::Enter);
        tab_key(&mut ui);
        assert_eq!(ui.focus.current(), Some(Id::Palette), "focus moved while the list was open");
        assert!(ui.palette.is_open());
    }

    #[test]
    fn escape_dismisses_a_list_and_puts_the_choice_back() {
        let mut ui = Ui::new();
        ui.focus.focus(Id::Palette);
        press(&mut ui, KeyCode::Enter);
        press(&mut ui, KeyCode::Down);
        assert_eq!(ui.palette.selected(), 1);
        let flow = press(&mut ui, KeyCode::Escape);
        assert_eq!(flow, Flow::Continue, "escape closed the list, it must not also quit");
        assert!(!ui.palette.is_open());
        assert_eq!(ui.palette.selected(), 0, "a dismissed list must not leave a half-made choice");
    }

    #[test]
    fn the_arrows_step_a_closed_select_without_opening_it() {
        let mut ui = Ui::new();
        ui.focus.focus(Id::Palette);
        press(&mut ui, KeyCode::Right);
        assert_eq!(ui.palette.selected(), 1);
        assert!(!ui.palette.is_open());
        press(&mut ui, KeyCode::Left);
        assert_eq!(ui.palette.selected(), 0);
    }

    // ---- The text field ------------------------------------------------------------------

    #[test]
    fn q_quits_everywhere_except_inside_the_text_field() {
        let mut ui = Ui::new();
        assert_eq!(press(&mut ui, KeyCode::Char('q')), Flow::Quit);

        let mut ui = Ui::new();
        ui.focus.focus(Id::Label);
        ui.label.clear();
        assert_eq!(press(&mut ui, KeyCode::Char('q')), Flow::Continue);
        assert_eq!(ui.label.value(), "q");
    }

    #[test]
    fn typing_in_the_heading_marks_the_screen_modified() {
        let mut ui = Ui::new();
        assert!(!ui.is_modified());
        ui.focus.focus(Id::Label);
        press(&mut ui, KeyCode::Char('!'));
        assert!(ui.is_modified());
    }

    // ---- Apply and revert ----------------------------------------------------------------

    #[test]
    fn applying_a_palette_asks_the_loop_to_retheme() {
        let mut ui = Ui::new();
        assert_eq!(ui.theme().accent, Theme::LAYA.accent);
        ui.palette.set_selected(1);
        ui.focus.focus(Id::Apply);
        assert_eq!(press(&mut ui, KeyCode::Enter), Flow::Retheme);
        assert_eq!(ui.theme().accent, Theme::EMBER.accent);
        assert!(!ui.is_modified());
    }

    #[test]
    fn applying_nothing_is_not_a_retheme() {
        let mut ui = Ui::new();
        ui.focus.focus(Id::Apply);
        assert_eq!(press(&mut ui, KeyCode::Enter), Flow::Continue);
    }

    #[test]
    fn revert_puts_every_field_back() {
        let mut ui = Ui::new();
        ui.palette.set_selected(2);
        ui.bars.set_selected(3);
        ui.focus.focus(Id::Label);
        press(&mut ui, KeyCode::Char('?'));
        assert!(ui.is_modified());

        ui.focus.focus(Id::Revert);
        press(&mut ui, KeyCode::Enter);
        assert!(!ui.is_modified());
        assert_eq!(ui.palette.selected(), 0);
        assert_eq!(ui.label.value(), "LOCAL INTELLIGENCE");
    }

    // ---- What ends up on screen ----------------------------------------------------------

    #[test]
    fn the_focused_control_is_the_only_one_wearing_the_highlight() {
        let ui = Ui::demo();
        let mut buffer = Buffer::new(88, 24);
        let mut frame = Frame::new(&mut buffer, ui.theme());
        ui.compose(&mut frame);

        // Find the two select rows by their labels, then compare their backgrounds.
        let rows: Vec<String> = (0..buffer.height()).map(|row| buffer.row_text(row)).collect();
        let palette_row = rows.iter().position(|row| row.contains("Palette")).expect("palette row");
        let bars_row = rows.iter().position(|row| row.contains("Bars")).expect("bars row");
        let field_x = 2 + LABEL_WIDTH;

        let focused = buffer.get(field_x, palette_row as u16).expect("cell").style.bg;
        let unfocused = buffer.get(field_x, bars_row as u16).expect("cell").style.bg;
        assert_eq!(focused, Some(Theme::LAYA.surface));
        assert_ne!(unfocused, Some(Theme::LAYA.surface));
    }

    #[test]
    fn an_open_list_is_drawn_over_the_content_beneath_it() {
        let mut ui = Ui::new();
        ui.focus.focus(Id::Palette);
        let closed = screen(&ui, 88, 24);
        assert!(closed.contains("Shaded"), "the Bars field should start out visible: {closed}");

        press(&mut ui, KeyCode::Enter);
        let open = screen(&ui, 88, 24);
        // Every choice shows, and what the list covers is gone rather than showing through.
        for palette in PALETTES {
            assert!(open.contains(palette), "{palette} missing from {open}");
        }
        assert!(!open.contains("Shaded"), "the list did not cover the field below it: {open}");
    }

    #[test]
    fn a_list_with_no_room_below_flips_above_its_field() {
        let mut ui = Ui::new();
        ui.focus.focus(Id::Sidebar);
        ui.tab.set_selected(1);
        ui.retarget();
        ui.focus.focus(Id::Sidebar);
        press(&mut ui, KeyCode::Enter);

        let screen_area = Rect::sized(88, 24);
        let field = {
            let mut buffer = Buffer::new(88, 24);
            let mut frame = Frame::new(&mut buffer, ui.theme());
            ui.compose(&mut frame);
            ui.sidebar.field()
        };
        let below = ui.sidebar.popup_area(SIDEBAR.len(), screen_area);
        assert_eq!(below.y, field.bottom(), "there is room below, so it should open downwards");

        // Now pretend the field sits on the last usable row.
        ui.sidebar.set_field(Rect::new(field.x, screen_area.bottom() - 1, field.width, 1));
        let above = ui.sidebar.popup_area(SIDEBAR.len(), screen_area);
        assert!(above.bottom() <= screen_area.bottom(), "the list ran off the bottom: {above:?}");
        assert!(above.y < screen_area.bottom() - 1, "the list did not flip above the field");
    }

    // ---- Mouse ---------------------------------------------------------------------------

    #[test]
    fn clicking_a_control_moves_focus_to_it() {
        let mut ui = Ui::new();
        assert_eq!(ui.focus.current(), Some(Id::Tabs));
        let (x, y) = centre_of(&mut ui, Id::Label);
        click_at(&mut ui, x, y);
        assert_eq!(ui.focus.current(), Some(Id::Label));
        // And the keyboard carries on from where the mouse left it.
        press(&mut ui, KeyCode::Char('q'));
        assert_eq!(ui.label.value(), "LOCAL INTELLIGENCEq", "q is a letter in the field");
    }

    #[test]
    fn clicking_a_tab_label_selects_that_tab_and_a_gap_selects_nothing() {
        let mut ui = Ui::new();
        let _ = screen(&ui, 88, 24);
        let bar = ui.hits.area_of(Id::Tabs).expect("the tab bar drew");
        // "APPEARANCE" is ten columns, then three of gap, then "LAYOUT".
        click_at(&mut ui, bar.x + 14, bar.y);
        assert_eq!(ui.tab.selected(), 1);
        assert_eq!(ui.focus.ring(), &[Id::Tabs, Id::Density, Id::Sidebar, Id::Revert, Id::Apply]);

        click_at(&mut ui, bar.x + 11, bar.y);
        assert_eq!(ui.tab.selected(), 1, "the gap between labels belongs to neither");
    }

    #[test]
    fn clicking_a_select_opens_it_and_clicking_an_option_chooses_it() {
        let mut ui = Ui::new();
        let (x, y) = centre_of(&mut ui, Id::Bars);
        click_at(&mut ui, x, y);
        assert!(ui.bars.is_open());
        assert_eq!(ui.focus.current(), Some(Id::Bars));

        // The list is drawn below the field with a row of border, so option 3 is three rows down.
        let popup = ui.bars.popup_area(BARS.len(), Rect::sized(88, 24));
        click_at(&mut ui, popup.x + 2, popup.y + 1 + 3);
        assert!(!ui.bars.is_open());
        assert_eq!(ui.bars.selected(), 3, "Smooth");
    }

    #[test]
    fn clicking_outside_an_open_list_dismisses_it_without_reaching_what_is_under_the_pointer() {
        let mut ui = Ui::new();
        let (x, y) = centre_of(&mut ui, Id::Palette);
        click_at(&mut ui, x, y);
        assert!(ui.palette.is_open());

        // Straight at the Apply button, which must not fire: the list is modal, for the mouse
        // exactly as much as for the keyboard.
        let (apply_x, apply_y) = centre_of(&mut ui, Id::Apply);
        let flow = click_at(&mut ui, apply_x, apply_y);
        assert_eq!(flow, Flow::Continue, "Apply must not have been pressed");
        assert!(!ui.palette.is_open());
        assert_eq!(ui.focus.current(), Some(Id::Palette), "and focus stayed put");
    }

    #[test]
    fn clicking_apply_saves_and_clicking_a_disabled_revert_does_nothing() {
        let mut ui = Ui::demo();
        assert!(ui.is_modified());
        let (x, y) = centre_of(&mut ui, Id::Apply);
        assert_eq!(click_at(&mut ui, x, y), Flow::Retheme);
        assert!(!ui.is_modified());

        // Revert is drawn disabled now, so clicking it must not claim to have done anything.
        let (rx, ry) = centre_of(&mut ui, Id::Revert);
        click_at(&mut ui, rx, ry);
        assert_eq!(ui.status, "nothing to revert");
        assert!(!ui.is_modified());
    }

    #[test]
    fn a_click_on_the_chrome_changes_nothing() {
        let mut ui = Ui::new();
        tab_key(&mut ui);
        assert_eq!(ui.focus.current(), Some(Id::Palette));
        // The header line: no control there, so focus must survive it.
        click_at(&mut ui, 4, 1);
        assert_eq!(ui.focus.current(), Some(Id::Palette));
        assert_eq!(ui.status, "ready");
    }

    #[test]
    fn a_click_lands_on_what_is_there_now_not_on_what_used_to_be() {
        let mut ui = Ui::new();
        let (x, y) = centre_of(&mut ui, Id::Palette);
        // The LAYOUT tab puts Density in exactly the same place Palette was. The click must find
        // the control that drew this frame, not the one that drew last.
        ui.tab.set_selected(1);
        ui.retarget();
        click_at(&mut ui, x, y);
        assert!(!ui.palette.is_open(), "a stale hit re-opened a control that is not on screen");
        assert!(ui.density.is_open());
        assert_eq!(ui.focus.current(), Some(Id::Density));
    }

    #[test]
    fn the_about_tab_has_no_form_fields_but_still_has_its_buttons() {
        let ui = about_tab();
        assert_eq!(ui.focus.ring(), &[Id::Tabs, Id::About, Id::Revert, Id::Apply]);
        let rendered = screen(&ui, 88, 24);
        assert!(rendered.contains("Apply"), "got {rendered}");
        assert!(rendered.contains("MIT OR Apache-2.0"), "got {rendered}");
    }

    // ---- Scrolling -----------------------------------------------------------------------

    /// The ABOUT tab, with its pane focused and drawn once so the viewport knows its own size.
    fn about_tab() -> Ui {
        let mut ui = Ui::new();
        ui.tab.set_selected(2);
        ui.retarget();
        ui.focus.focus(Id::About);
        let _ = screen(&ui, 88, 24);
        ui
    }

    fn wheel(ui: &mut Ui, column: u16, row: u16, down: bool) {
        let _ = screen(ui, 88, 24);
        ui.handle(&Event::Mouse(MouseEvent {
            kind: if down { conui::MouseKind::ScrollDown } else { conui::MouseKind::ScrollUp },
            column,
            row,
            modifiers: conui::Modifiers::NONE,
        }));
    }

    #[test]
    fn the_about_pane_holds_more_than_it_shows() {
        let ui = about_tab();
        assert!(ui.about.is_scrollable(), "nothing below the fold, so nothing to demonstrate");
        // The buttons are outside the scrolled region, so they stay put while the text moves.
        assert!(screen(&ui, 88, 24).contains("Revert"));
    }

    #[test]
    fn the_end_of_the_text_is_reachable_and_the_pane_stops_there() {
        let mut ui = about_tab();
        press(&mut ui, KeyCode::End);
        let rendered = screen(&ui, 88, 24);
        assert!(rendered.contains("HOME and END"), "the last line never came into view");
        assert!(rendered.contains("Revert"), "the buttons scrolled away with the text");
        assert_eq!(ui.about.offset(), ui.about.overflow(), "the end is the end");

        // And it is the end: pressing on does not reveal blank rows below the text.
        for _ in 0..40 {
            press(&mut ui, KeyCode::Down);
        }
        assert_eq!(ui.about.offset(), ui.about.overflow());
        assert_eq!(screen(&ui, 88, 24), rendered);
    }

    #[test]
    fn scrolling_up_from_the_top_is_a_no_op_rather_than_an_underflow() {
        let mut ui = about_tab();
        assert!(ui.about.is_at_top());
        for _ in 0..5 {
            press(&mut ui, KeyCode::Up);
        }
        assert_eq!(ui.about.offset(), 0);
        press(&mut ui, KeyCode::PageUp);
        assert_eq!(ui.about.offset(), 0);
    }

    #[test]
    fn a_page_is_most_of_a_paneful_and_a_row_is_a_row() {
        let mut ui = about_tab();
        press(&mut ui, KeyCode::Down);
        assert_eq!(ui.about.offset(), 1);
        press(&mut ui, KeyCode::PageDown);
        // One row of overlap, so the line you were reading is still on screen.
        assert_eq!(ui.about.offset(), 1 + ui.about.height() - 1);
        press(&mut ui, KeyCode::Home);
        assert!(ui.about.is_at_top());
    }

    #[test]
    fn the_text_that_scrolls_away_is_gone_rather_than_left_behind() {
        let mut ui = about_tab();
        let top = screen(&ui, 88, 24);
        assert!(top.contains("A devkit for console applications"));
        press(&mut ui, KeyCode::End);
        let bottom = screen(&ui, 88, 24);
        assert!(
            !bottom.contains("A devkit for console applications"),
            "the first paragraph is still there after scrolling past it: {bottom}"
        );
    }

    #[test]
    fn the_wheel_scrolls_the_pane_it_is_over_without_taking_focus() {
        let mut ui = about_tab();
        ui.focus.focus(Id::Apply);
        let pane = ui.hits.area_of(Id::About).expect("the pane drew");
        wheel(&mut ui, pane.x + 2, pane.y + 2, true);
        assert_eq!(ui.about.offset(), 1);
        // Reading something is not the same as saying "talk to this one".
        assert_eq!(ui.focus.current(), Some(Id::Apply), "the wheel moved focus");
        wheel(&mut ui, pane.x + 2, pane.y + 2, false);
        assert_eq!(ui.about.offset(), 0);
    }

    #[test]
    fn the_wheel_away_from_the_pane_scrolls_nothing() {
        let mut ui = about_tab();
        // The footer, which is below the pane and belongs to no control.
        wheel(&mut ui, 4, 22, true);
        assert_eq!(ui.about.offset(), 0);
    }

    #[test]
    fn the_wheel_over_a_form_does_not_change_a_value() {
        let mut ui = Ui::new();
        let (x, y) = centre_of(&mut ui, Id::Bars);
        let before = ui.values();
        wheel(&mut ui, x, y, true);
        wheel(&mut ui, x, y, false);
        assert_eq!(ui.values(), before, "a wheel over a select changed the setting under it");
        assert!(!ui.is_modified());
    }

    #[test]
    fn switching_tabs_away_and_back_keeps_your_place_in_the_text() {
        let mut ui = about_tab();
        press(&mut ui, KeyCode::PageDown);
        let offset = ui.about.offset();
        assert!(offset > 0);

        ui.focus.focus(Id::Tabs);
        press(&mut ui, KeyCode::Left);
        assert_eq!(ui.tab.selected(), 1);
        let _ = screen(&ui, 88, 24);
        press(&mut ui, KeyCode::Right);
        let _ = screen(&ui, 88, 24);
        assert_eq!(ui.about.offset(), offset, "the pane forgot where you had got to");
    }

    #[test]
    fn a_pane_taller_than_its_text_has_nothing_to_scroll() {
        let mut ui = about_tab();
        // Tall enough for every line at once, so there is no bar and no offset to have.
        let _ = screen(&ui, 88, 60);
        assert!(!ui.about.is_scrollable());
        press(&mut ui, KeyCode::PageDown);
        assert_eq!(ui.about.offset(), 0);
    }

    #[test]
    fn a_pane_that_shrinks_under_a_scrolled_offset_shows_the_end_not_a_blank() {
        let mut ui = about_tab();
        press(&mut ui, KeyCode::End);
        let offset = ui.about.offset();
        // A much taller pane can show more at once, so the same offset would leave blank rows at
        // the bottom. The next draw is what corrects it, because the draw is what knows.
        let rendered = screen(&ui, 88, 40);
        assert!(ui.about.offset() < offset, "the offset survived a pane that outgrew it");
        assert!(rendered.contains("HOME and END"), "got {rendered}");
        let last = rendered.lines().filter(|line| line.contains("HOME and END")).count();
        assert_eq!(last, 1);
    }
}
