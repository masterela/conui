//! A cell-for-cell port of the `laya-mlx` snake dashboard, the design conui was built to hit.
//!
//! Run it with `cargo run -p conui --example snake`. It plays itself; the keys change speed and
//! pause it. Nothing on this screen is fake: where the original showed a language model's output
//! this shows the local heuristic policy that actually chose the move, with the real time it took
//! and the real number of cells its flood fill visited. A dashboard that invents numbers to look
//! busy is worse than one with fewer rows.
//!
//! What it demonstrates, in the order you meet it:
//!
//! - [`App`] for the terminal lifecycle and the event loop's waiting half.
//! - The **canvas** layer for the parts that are a fixed grid: the board, the header, the panel
//!   labels. This design positions everything absolutely on a 104×35 composition, and pretending
//!   otherwise by wrapping each label in a nested layout would be a worse program.
//! - The **widget** layer for the parts that are genuinely widgets, placed at absolute spots via
//!   `Canvas::sub`: [`Stat`] for the score readouts, [`Gauge`] for every bar, [`Field`] for the
//!   telemetry rows, [`Hints`] for the footer. The two layers share one buffer and one theme.

use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use conui::view::View;
use conui::widget::{Field, Gauge, Hints, Readout, Stat};
use conui::{
    App, BarStyle, Canvas, Color, Config, Frame, KeyCode, Rect, Role, Style, Theme, centered,
    text_width,
};

// ---- Geometry ----------------------------------------------------------------------------
//
// Straight from the reference composition. These are the numbers the whole screen hangs off, so
// they live here as named constants rather than as arithmetic scattered through the draw code.

const BOARD_WIDTH: i32 = 24;
const BOARD_HEIGHT: i32 = 16;

/// `max(104, width * 2 + 50)` and `max(35, height + 19)` from the original.
const LAYOUT_WIDTH: u16 = 104;
const LAYOUT_HEIGHT: u16 = 35;

const LEFT: i32 = 3;
/// `max(58, width * 2 + 10)`: where the right-hand column starts.
const RIGHT: i32 = 58;
const TOP: i32 = 6;
/// Columns available to the right-hand column, less its margin.
const SIDE: i32 = LAYOUT_WIDTH as i32 - RIGHT - 4;
/// The board's closing rule.
const BOTTOM: i32 = TOP + BOARD_HEIGHT + 1;
/// Width of the telemetry block: label at 0, bar at 9 for 18 cells, readout ending at 33.
const PANEL_WIDTH: u16 = 33;

const HEAD_COLOR: Color = Color::hex("#dcfff0");

fn main() -> io::Result<()> {
    // `--dump [steps]` prints one composed frame as plain text and exits. Useful for a README,
    // for a diff against the reference design, and for checking the layout without a terminal.
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--dump") {
        let steps = args.next().and_then(|value| value.parse().ok()).unwrap_or(40);
        dump(steps);
        return Ok(());
    }

    let mut session = Session::new(seed_from_clock());
    let mut app = App::with(
        Config::new()
            .theme(Theme::LAYA)
            // The composition is a fixed size, so there is no point redrawing faster than the
            // eye resolves; and below its size there is nothing sensible to draw at all.
            .fps(30)
            .min_size(LAYOUT_WIDTH, LAYOUT_HEIGHT),
    )?;

    while app.is_running() {
        for event in app.poll()? {
            if let Some(key) = event.as_key() {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Escape => app.quit(),
                    KeyCode::Char(' ') => session.paused = !session.paused,
                    KeyCode::Char('r') | KeyCode::Char('R') => session.restart(true),
                    KeyCode::Char('g') | KeyCode::Char('G') => session.guarded = !session.guarded,
                    KeyCode::Up => session.faster(),
                    KeyCode::Down => session.slower(),
                    _ => {}
                }
            }
        }
        session.advance();
        app.draw(|frame| compose(frame, &session))?;
    }
    Ok(())
}

/// Play `steps` moves from a fixed seed and print the resulting frame as text.
fn dump(steps: usize) {
    let mut session = Session::new(0x5eed);
    for _ in 0..steps {
        session.step_now(Instant::now());
    }
    // A `Frame` needs nothing but a buffer, which is what makes a whole screen testable.
    let mut buffer = conui::Buffer::new(LAYOUT_WIDTH, LAYOUT_HEIGHT);
    let mut frame = Frame::new(&mut buffer, Theme::LAYA);
    compose(&mut frame, &session);
    for row in 0..LAYOUT_HEIGHT {
        println!("{}", buffer.row_text(row));
    }
}

// ---- The frame ---------------------------------------------------------------------------

/// Draw the whole screen. Called once per frame with no memory of the last one.
fn compose(frame: &mut Frame<'_>, session: &Session) {
    // The composition is a fixed 104×35 regardless of the window, so it is centred rather than
    // stretched: a design laid out in absolute cells cannot be scaled without redesigning it.
    let area = centered(frame.area(), LAYOUT_WIDTH, LAYOUT_HEIGHT);
    let mut canvas = frame.surface(area);
    let game = &session.game;
    let decision = &session.decision;
    let width = i32::from(LAYOUT_WIDTH);
    let height = i32::from(LAYOUT_HEIGHT);

    // ---- Header
    let state = session.state_label();
    canvas.put(LEFT, 1, "CONUI  /  LOCAL INTELLIGENCE", Role::Muted);
    let state_role = if game.alive { Role::Accent } else { Role::Danger };
    canvas.put(width - i32::from(text_width(state)) - 3, 1, state, state_role);
    canvas.rule(LEFT, 2, LAYOUT_WIDTH - 6, Role::Dim);
    canvas.put(LEFT, 4, "S N A K E", Role::Text);
    canvas.put(LEFT + 31, 4, &format!("ROUND {:02}", session.round), Role::Muted);

    // ---- Board
    let span = game.width * 2;
    let fence = "─".repeat(span as usize);
    canvas.put(LEFT, TOP, &format!("┌{fence}┐"), Role::Dim);
    canvas.put(LEFT, BOTTOM, &format!("└{fence}┘"), Role::Dim);
    for row in 0..game.height {
        canvas.put(LEFT, TOP + 1 + row, "│", Role::Dim);
        canvas.put(LEFT + span + 1, TOP + 1 + row, "│", Role::Dim);
    }
    // The mesh gives the eye a grid to measure distance against while staying dark enough to
    // read as empty space.
    canvas.mesh(
        Rect::new((LEFT + 1) as u16, (TOP + 1) as u16, span as u16, game.height as u16),
        "· ",
        Role::Surface,
    );

    // Tail first so the head's brighter cap lands on top where the body doubles back.
    let ground = canvas.theme().background;
    let length = game.body.len().max(1) as f32;
    for (index, &(x, y)) in game.body.iter().enumerate().rev() {
        let fraction = 1.0 - index as f32 / length;
        let color = if index == 0 {
            HEAD_COLOR
        } else {
            Color::rgb(
                (18.0 + 64.0 * fraction) as u8,
                (73.0 + 150.0 * fraction) as u8,
                (57.0 + 102.0 * fraction) as u8,
            )
        };
        // Two columns per square: a terminal cell is about twice as tall as it is wide, so one
        // cell per board square would draw a board twice as tall as it is wide.
        canvas.put_styled(
            LEFT + 1 + 2 * x,
            TOP + 1 + y,
            conui::canvas::SQUARE,
            Style::EMPTY.fg(color).bg(ground),
        );
    }
    if let Some((x, y)) = game.food {
        canvas.put(LEFT + 1 + 2 * x, TOP + 1 + y, "● ", Role::Warn);
    }

    // ---- Score readouts
    //
    // Three `Stat` widgets dropped onto the absolute grid. Each is a muted label above a
    // zero-padded three-digit block number, which is exactly what this design does by hand.
    let best = session.best.max(game.score);
    let stats: [(i32, &str, i64, Role); 3] = [
        (0, "SCORE", i64::from(game.score), Role::Accent),
        (18, "LENGTH", game.body.len() as i64, Role::Text),
        (36, "BEST", i64::from(best), Role::Muted),
    ];
    for (offset, label, value, role) in stats {
        let stat = Stat::new(label, value).digits(3).role(role);
        place(&mut canvas, LEFT + offset, BOTTOM + 2, 12, 4, &stat);
    }

    // Board occupancy. Drawn on the canvas rather than as a `Gauge` because the readout sits at
    // a fixed column here, not flush right.
    let capacity = (game.width * game.height) as f32;
    let filled = game.body.len() as f32 / capacity;
    canvas.bar(LEFT, BOTTOM + 7, filled, 41, Role::Accent);
    canvas.put(LEFT + 43, BOTTOM + 7, &format!("{:4.1}%", filled * 100.0), Role::Muted);

    // ---- Right column: what the policy decided and why
    canvas.put(RIGHT, 4, "conui · snake", Role::Accent);
    canvas.put(RIGHT, 5, &format!("{} · Rust", hardware()), Role::Muted);
    canvas.put(RIGHT, 7, "NEXT MOVE", Role::Text);
    canvas.put(RIGHT + 15, 7, "POLICY WEIGHTS", Role::Muted);

    for (index, direction) in Direction::ALL.into_iter().enumerate() {
        let selected = direction == decision.proposed;
        let role = if selected { Role::Accent } else { Role::Muted };
        let gauge = Gauge::new(decision.probabilities[index])
            .label(direction.label())
            .label_width(6)
            .label_role(role)
            .role(role)
            .track(Role::Dim)
            .style(BarStyle::Shaded)
            .bar_width(18)
            .readout(Readout::Fraction)
            .selected(selected);
        place(&mut canvas, RIGHT, 9 + index as i32, PANEL_WIDTH, 1, &gauge);
    }

    canvas.put(RIGHT, 14, "EXECUTING", Role::Muted);
    canvas.put(RIGHT + 12, 14, decision.executed.label(), Role::Accent);
    if decision.intervened {
        // Naming the override is the point: a supervisor that silently corrects the thing it
        // supervises teaches you nothing about either of them.
        canvas.put(RIGHT + 20, 14, "SHIELD", Role::Warn);
    }

    let risk = decision.dead_end_risk;
    canvas.put(RIGHT, 16, "DEAD-END RISK", Role::Muted);
    let risk_gauge = Gauge::new(risk)
        .role(canvas.theme().escalate(risk, 0.5))
        .bar_width(24.min(SIDE as u16 - 9))
        .readout(Readout::Fraction);
    place(&mut canvas, RIGHT, 17, PANEL_WIDTH, 1, &risk_gauge);

    canvas.put(RIGHT, 19, "FOOD REACHABLE", Role::Muted);
    let food_gauge = Gauge::new(decision.food_reachable)
        .role(Role::Info)
        .bar_width(24.min(SIDE as u16 - 9))
        .readout(Readout::Fraction);
    place(&mut canvas, RIGHT, 20, PANEL_WIDTH, 1, &food_gauge);

    // ---- Telemetry
    let inference = decision.inference.as_secs_f32() * 1000.0;
    let rows: [(&str, String, Role); 5] = [
        ("INFERENCE", format!("{inference:5.2} ms"), Role::Text),
        ("DECISIONS", format!("{:5.1} /s", session.decisions_per_second()), Role::Text),
        ("CELLS VISITED", decision.visited.to_string(), Role::Text),
        ("NETWORK", "OFFLINE".to_string(), Role::Accent),
        ("ENGINE", "conui · Rust".to_string(), Role::Muted),
    ];
    for (index, (label, value, role)) in rows.into_iter().enumerate() {
        let field = Field::new(label, value).value_role(role).value_column(18);
        place(&mut canvas, RIGHT, 22 + index as i32, PANEL_WIDTH, 1, &field);
    }

    let shield = if session.guarded { "conui + cycle safety" } else { "conui · shield OFF" };
    canvas.put(RIGHT, 28, shield, Role::Muted);
    canvas.put(
        RIGHT,
        29,
        &format!("Shield interventions  {:04}", session.interventions),
        Role::Warn,
    );

    // ---- Footer
    canvas.rule(LEFT, height - 3, LAYOUT_WIDTH - 6, Role::Dim);
    let hints = Hints::new()
        .key("SPACE", "pause")
        .key("↑/↓", "speed")
        .key("R", "reset")
        .key("Q", "quit")
        .key("G", "shield");
    place(&mut canvas, LEFT, height - 2, (RIGHT - LEFT - 2) as u16, 1, &hints);

    let seconds = session.started.elapsed().as_secs();
    let clock = format!("{:02}:{:02}", seconds / 60, seconds % 60);
    canvas.put(RIGHT, height - 2, "LOCAL HEURISTIC POLICY", Role::Muted);
    canvas.put_right(RIGHT + 34, height - 2, &clock, Role::Muted);
}

/// Render a widget at an absolute spot on the composition.
///
/// This is the whole trick of a layered toolkit: a `View` only ever needs a region, and a region
/// is something the canvas can hand out. The widget does not know whether a layout engine or a
/// hand-written coordinate put it there, and cannot draw outside it either way.
fn place(canvas: &mut Canvas<'_>, x: i32, y: i32, width: u16, height: u16, view: &dyn View) {
    let area = Rect::new(x.max(0) as u16, y.max(0) as u16, width, height);
    let mut slot = canvas.sub(area);
    view.render(&mut slot);
}

/// A description of the machine, with no dependency on anything that would need one.
fn hardware() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

// ---- Session -----------------------------------------------------------------------------

/// Everything the screen shows that is not the board itself.
struct Session {
    game: Game,
    decision: Decision,
    rng: Rng,
    best: u32,
    round: u32,
    interventions: u32,
    paused: bool,
    guarded: bool,
    /// Index into [`Session::SPEEDS`].
    speed: usize,
    started: Instant,
    last_step: Instant,
    /// When the game ended, so the next round can start after a pause long enough to read.
    ended: Option<Instant>,
    /// Timestamps of recent decisions, for an honest decisions-per-second figure.
    recent: VecDeque<Instant>,
}

impl Session {
    /// Steps per second, from "watch the policy think" to "watch it run".
    const SPEEDS: [u32; 7] = [3, 6, 9, 14, 20, 30, 45];
    const RESTART_DELAY: Duration = Duration::from_millis(1600);

    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let game = Game::new(BOARD_WIDTH, BOARD_HEIGHT, seed);
        let decision = decide(&game, true, &mut rng);
        let now = Instant::now();
        Self {
            game,
            decision,
            rng,
            best: 0,
            round: 1,
            interventions: 0,
            paused: false,
            guarded: true,
            speed: 3,
            started: now,
            last_step: now,
            ended: None,
            recent: VecDeque::new(),
        }
    }

    fn state_label(&self) -> &'static str {
        if self.paused {
            "PAUSED"
        } else if self.game.won {
            "BOARD CLEAR"
        } else if !self.game.alive {
            "GAME OVER"
        } else {
            "LIVE"
        }
    }

    fn faster(&mut self) {
        self.speed = (self.speed + 1).min(Self::SPEEDS.len() - 1);
    }

    fn slower(&mut self) {
        self.speed = self.speed.saturating_sub(1);
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1) / Self::SPEEDS[self.speed]
    }

    fn decisions_per_second(&self) -> f32 {
        self.recent.len() as f32
    }

    /// Start a new round, keeping the best score and the running interventions count.
    fn restart(&mut self, manual: bool) {
        self.best = self.best.max(self.game.score);
        self.game = Game::new(BOARD_WIDTH, BOARD_HEIGHT, self.rng.next_u64());
        self.decision = decide(&self.game, self.guarded, &mut self.rng);
        self.round += 1;
        self.ended = None;
        self.last_step = Instant::now();
        if manual {
            self.paused = false;
        }
    }

    /// Step the game if it is due. Called every frame; does nothing most of the time.
    fn advance(&mut self) {
        let now = Instant::now();
        // Trim the rolling window first, so the rate reads correctly even while paused.
        while self.recent.front().is_some_and(|at| now.duration_since(*at) > Duration::from_secs(1))
        {
            self.recent.pop_front();
        }
        if self.paused {
            self.last_step = now;
            return;
        }
        if !self.game.alive || self.game.won {
            let ended = *self.ended.get_or_insert(now);
            if now.duration_since(ended) >= Self::RESTART_DELAY {
                self.restart(false);
            }
            return;
        }
        if now.duration_since(self.last_step) < self.interval() {
            return;
        }
        self.last_step = now;
        self.step_now(now);
    }

    /// Apply the decision currently on screen and compute the next one.
    ///
    /// The decision shown describes the board shown, so it is applied before the next is made.
    /// Displaying a move that was chosen from a different board is the easiest way for a
    /// dashboard to look right and be wrong.
    fn step_now(&mut self, now: Instant) {
        if !self.game.alive || self.game.won {
            return;
        }
        if self.decision.intervened {
            self.interventions += 1;
        }
        self.game.step(self.decision.executed);
        self.best = self.best.max(self.game.score);
        self.decision = decide(&self.game, self.guarded, &mut self.rng);
        self.recent.push_back(now);
    }
}

// ---- Game rules --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    const ALL: [Self; 4] = [Self::Up, Self::Down, Self::Left, Self::Right];

    const fn label(self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }

    const fn delta(self) -> (i32, i32) {
        match self {
            Self::Up => (0, -1),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
        }
    }
}

/// Deterministic snake rules, plus the Hamiltonian cycle the safety shield reasons over.
struct Game {
    width: i32,
    height: i32,
    /// A tour visiting every square once, used to decide whether a move can still be survived.
    cycle_index: Vec<usize>,
    body: VecDeque<(i32, i32)>,
    food: Option<(i32, i32)>,
    score: u32,
    alive: bool,
    won: bool,
    rng: Rng,
}

impl Game {
    fn new(width: i32, height: i32, seed: u64) -> Self {
        let cycle = hamiltonian_cycle(width, height);
        let capacity = (width * height) as usize;
        let mut cycle_index = vec![0usize; capacity];
        for (index, &(x, y)) in cycle.iter().enumerate() {
            cycle_index[(y * width + x) as usize] = index;
        }
        let start = cycle_index[((height / 2) * width + width / 2) as usize];
        let body: VecDeque<(i32, i32)> =
            (0..6).map(|offset| cycle[(start + capacity - offset) % capacity]).collect();

        let mut game = Self {
            width,
            height,
            cycle_index,
            body,
            food: None,
            score: 0,
            alive: true,
            won: false,
            rng: Rng::new(seed),
        };
        game.food = game.spawn_food();
        game
    }

    fn capacity(&self) -> usize {
        (self.width * self.height) as usize
    }

    fn head(&self) -> (i32, i32) {
        self.body[0]
    }

    fn index_of(&self, (x, y): (i32, i32)) -> usize {
        self.cycle_index[(y * self.width + x) as usize]
    }

    fn contains(&self, cell: (i32, i32)) -> bool {
        (0..self.width).contains(&cell.0) && (0..self.height).contains(&cell.1)
    }

    fn spawn_food(&mut self) -> Option<(i32, i32)> {
        let empty: Vec<(i32, i32)> = (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .filter(|cell| !self.body.contains(cell))
            .collect();
        if empty.is_empty() {
            return None;
        }
        Some(empty[self.rng.below(empty.len())])
    }

    fn target(&self, direction: Direction) -> (i32, i32) {
        let (dx, dy) = direction.delta();
        let (x, y) = self.head();
        (x + dx, y + dy)
    }

    /// Why a move is not available, or `"legal"`.
    fn legal_reason(&self, direction: Direction) -> &'static str {
        let cell = self.target(direction);
        if !self.contains(cell) {
            return "wall";
        }
        if self.body.len() > 1 && cell == self.body[1] {
            return "reverse";
        }
        // The tail vacates its square on a step that does not grow the snake, so following it is
        // legal — the single rule that separates a snake that can turn from one that cannot.
        let growing = Some(cell) == self.food;
        let last = self.body.len() - 1;
        let hits_body = self
            .body
            .iter()
            .enumerate()
            .any(|(index, &occupied)| occupied == cell && (growing || index != last));
        if hits_body { "body" } else { "legal" }
    }

    /// Whether a move keeps the snake on the safe side of the Hamiltonian cycle.
    ///
    /// The whole shield in one comparison: never advance further along the tour than the tail
    /// has, and never skip past the food, and the snake can always follow the tour home.
    fn is_safe(&self, direction: Direction) -> bool {
        if self.legal_reason(direction) != "legal" {
            return false;
        }
        let Some(food) = self.food else { return false };
        let capacity = self.capacity();
        let head = self.index_of(self.head());
        let tail_distance =
            (self.index_of(self.body[self.body.len() - 1]) + capacity - head) % capacity;
        let food_distance = (self.index_of(food) + capacity - head) % capacity;
        let target = self.target(direction);
        let advance = (self.index_of(target) + capacity - head) % capacity;
        let eats = target == food;

        if advance > tail_distance || (advance == tail_distance && eats) {
            return false;
        }
        advance != 0 && advance <= food_distance
    }

    /// Flood fill the free space from `origin`, treating the body as solid but the tail as
    /// about to move. Returns whether the food is reachable and how many cells were visited.
    fn reachable(&self, origin: (i32, i32)) -> (bool, usize) {
        let mut blocked = vec![false; self.capacity()];
        for (index, &(x, y)) in self.body.iter().enumerate() {
            if index + 1 == self.body.len() {
                continue;
            }
            blocked[(y * self.width + x) as usize] = true;
        }
        if !self.contains(origin) || blocked[(origin.1 * self.width + origin.0) as usize] {
            return (false, 0);
        }
        let mut seen = vec![false; self.capacity()];
        let mut queue = VecDeque::from([origin]);
        seen[(origin.1 * self.width + origin.0) as usize] = true;
        let mut count = 0usize;
        while let Some((x, y)) = queue.pop_front() {
            count += 1;
            for direction in Direction::ALL {
                let (dx, dy) = direction.delta();
                let cell = (x + dx, y + dy);
                if !self.contains(cell) {
                    continue;
                }
                let slot = (cell.1 * self.width + cell.0) as usize;
                if blocked[slot] || seen[slot] {
                    continue;
                }
                seen[slot] = true;
                queue.push_back(cell);
            }
        }
        let food_found = self.food.is_some_and(|(x, y)| seen[(y * self.width + x) as usize]);
        (food_found, count)
    }

    fn step(&mut self, direction: Direction) {
        if !self.alive || self.won {
            return;
        }
        if self.legal_reason(direction) != "legal" {
            self.alive = false;
            return;
        }
        let target = self.target(direction);
        self.body.push_front(target);
        if Some(target) == self.food {
            self.score += 1;
            if self.body.len() == self.capacity() {
                self.won = true;
                self.food = None;
            } else {
                self.food = self.spawn_food();
            }
        } else {
            self.body.pop_back();
        }
    }
}

/// A tour of every square, each step to an adjacent square, closing back to the start.
///
/// Boiled down: snake down each row in alternating directions, then return up column zero.
/// Requires an even height (or an even width, handled by transposing).
fn hamiltonian_cycle(width: i32, height: i32) -> Vec<(i32, i32)> {
    assert!(width >= 4 && height >= 4, "board must be at least 4×4");
    if height % 2 == 1 {
        assert!(width % 2 == 0, "at least one dimension must be even");
        return hamiltonian_cycle(height, width).into_iter().map(|(x, y)| (y, x)).collect();
    }
    let mut path = vec![(0, 0)];
    for y in 0..height {
        if y % 2 == 0 {
            path.extend((1..width).map(|x| (x, y)));
        } else {
            path.extend((1..width).rev().map(|x| (x, y)));
        }
    }
    path.extend((1..height).rev().map(|y| (0, y)));
    path
}

// ---- The policy --------------------------------------------------------------------------

/// What the policy decided, and the evidence the panel shows for it.
struct Decision {
    probabilities: [f32; 4],
    proposed: Direction,
    executed: Direction,
    intervened: bool,
    dead_end_risk: f32,
    food_reachable: f32,
    inference: Duration,
    visited: usize,
}

/// Score each direction, turn the scores into probabilities, and let the shield veto.
///
/// This is where the original called a language model. The shape of the output is identical —
/// a distribution over four moves, a proposal, and a supervisor that may override it — which is
/// the point: the dashboard does not care what produced the numbers, only that they are real.
fn decide(game: &Game, guarded: bool, rng: &mut Rng) -> Decision {
    let started = Instant::now();
    let empty = (game.capacity() - game.body.len() + 1) as f32;
    let far = (game.width + game.height) as f32;

    let mut scores = [0f32; 4];
    // Every cell the policy's flood fills touched: what its work actually consists of.
    let mut examined = 0usize;
    for (index, direction) in Direction::ALL.into_iter().enumerate() {
        if game.legal_reason(direction) != "legal" {
            scores[index] = -8.0;
            continue;
        }
        let target = game.target(direction);
        let (food_reachable, visited) = game.reachable(target);
        examined += visited;

        // Room to move matters most: a shorter route into a pocket kills the snake.
        let mut score = 2.6 * (visited as f32 / empty).min(1.0);
        if let Some(food) = game.food {
            let distance = (target.0 - food.0).abs() + (target.1 - food.1).abs();
            score += 1.8 * (1.0 - distance as f32 / far);
            if target == food {
                score += 0.6;
            }
        }
        if !food_reachable {
            score -= 1.2;
        }
        // Deliberately a mild preference, not a rule. A policy that already knows about the
        // safe tour would make its supervisor decorative, and then the panel would be showing
        // you a shield that never does anything — which teaches you nothing about either.
        if game.is_safe(direction) {
            score += 0.3;
        }
        // A little noise, so the bars move the way a real estimator's do rather than snapping
        // between four fixed shapes.
        scores[index] = score + rng.jitter(0.12);
    }

    let probabilities = softmax(&scores, 0.45);
    let proposed = argmax(&probabilities);

    let safe: Vec<usize> = (0..4).filter(|&i| game.is_safe(Direction::ALL[i])).collect();
    let legal: Vec<usize> =
        (0..4).filter(|&i| game.legal_reason(Direction::ALL[i]) == "legal").collect();
    let proposed_index = Direction::ALL.iter().position(|d| *d == proposed).unwrap_or(0);

    // The shield only intervenes when the proposal is not on the safe tour, and it picks the
    // proposal's own next favourite among the moves that are — it corrects, it does not replace.
    let executed_index = if guarded && !safe.contains(&proposed_index) && !safe.is_empty() {
        *safe.iter().max_by(|a, b| probabilities[**a].total_cmp(&probabilities[**b])).unwrap()
    } else if !legal.contains(&proposed_index) && !legal.is_empty() {
        *legal.iter().max_by(|a, b| probabilities[**a].total_cmp(&probabilities[**b])).unwrap()
    } else {
        proposed_index
    };
    let executed = Direction::ALL[executed_index];

    let (food_reachable, visited) = game.reachable(game.target(executed));
    Decision {
        probabilities,
        proposed,
        executed,
        intervened: executed != proposed,
        // A snake dies when the pocket it commits to is smaller than the snake, so the honest
        // risk is room measured against its own length — not against the board. It sits at zero
        // through open play and climbs as the space closes in, which is what you want to watch.
        dead_end_risk: (1.0 - visited as f32 / (game.body.len() + 1) as f32).clamp(0.0, 1.0),
        food_reachable: if food_reachable { (visited as f32 / empty).clamp(0.0, 1.0) } else { 0.0 },
        inference: started.elapsed(),
        visited: examined + visited,
    }
}

/// Scores to probabilities, with a temperature so the distribution has visible shape.
fn softmax(scores: &[f32; 4], temperature: f32) -> [f32; 4] {
    let peak = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut weights = [0f32; 4];
    let mut total = 0f32;
    for (index, score) in scores.iter().enumerate() {
        // Subtract the peak before exponentiating: `exp` of a large positive number overflows
        // to infinity and the whole distribution becomes `NaN`.
        let weight = ((score - peak) / temperature).exp();
        weights[index] = weight;
        total += weight;
    }
    if total <= 0.0 {
        return [0.25; 4];
    }
    weights.map(|weight| weight / total)
}

fn argmax(probabilities: &[f32; 4]) -> Direction {
    let mut best = 0usize;
    for index in 1..4 {
        if probabilities[index] > probabilities[best] {
            best = index;
        }
    }
    Direction::ALL[best]
}

// ---- Randomness --------------------------------------------------------------------------

/// xorshift64*: three lines, no dependency, and far better than good enough to place food.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 { 0 } else { (self.next_u64() % bound as u64) as usize }
    }

    /// A value in `-magnitude..=magnitude`.
    fn jitter(&mut self, magnitude: f32) -> f32 {
        // 24 bits is more than an `f32` can represent distinctly anyway.
        let unit = (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32;
        (unit * 2.0 - 1.0) * magnitude
    }
}

/// A seed that differs between runs, without pulling in a clock crate.
fn seed_from_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0x51ed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nothing here needs a terminal: the rules are arithmetic, the policy is a pure function of a
    // board and a seed, and a frame is a `Buffer`. A demo is still a program, and this one is the
    // milestone the whole kit was built to hit, so it is tested like one.

    /// A board small enough to reason about by hand, seeded so it replays.
    fn small() -> Game {
        Game::new(6, 4, 1)
    }

    /// Replace the snake outright, for the cases that are awkward to reach by playing.
    fn with_body(game: &mut Game, cells: &[(i32, i32)], food: Option<(i32, i32)>) {
        game.body = cells.iter().copied().collect();
        game.food = food;
    }

    fn screen(session: &Session, width: u16, height: u16) -> Vec<String> {
        let mut buffer = conui::Buffer::new(width, height);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        compose(&mut frame, session);
        (0..height).map(|row| buffer.row_text(row)).collect()
    }

    fn cell(rows: &[String], x: i32, y: i32) -> char {
        rows[y as usize].chars().nth(x as usize).unwrap_or(' ')
    }

    // ---- Rules

    #[test]
    fn a_move_that_does_not_eat_keeps_the_snake_the_same_length() {
        let mut game = small();
        with_body(&mut game, &[(2, 1), (2, 2), (3, 2)], Some((0, 0)));
        game.step(Direction::Right);
        assert_eq!(game.head(), (3, 1));
        assert_eq!(game.body.len(), 3, "the tail leaves as the head arrives");
        assert_eq!(game.score, 0);
        assert!(game.alive);
    }

    #[test]
    fn eating_grows_the_snake_scores_and_puts_the_food_somewhere_else() {
        let mut game = small();
        with_body(&mut game, &[(2, 1), (2, 2), (3, 2)], Some((3, 1)));
        game.step(Direction::Right);
        assert_eq!(game.score, 1);
        assert_eq!(game.body.len(), 4, "the tail stays put on the step that grows");
        let food = game.food.expect("a board with room gets new food");
        assert_ne!(food, (3, 1), "the eaten square is not where the next one appears");
        assert!(!game.body.contains(&food), "and it never appears under the snake");
    }

    #[test]
    fn the_reason_a_move_is_refused_is_named() {
        let mut game = small();
        // Head at the top-left corner, facing right, with the body trailing to its right.
        with_body(&mut game, &[(0, 0), (1, 0), (2, 0), (2, 1)], Some((5, 3)));
        assert_eq!(game.legal_reason(Direction::Left), "wall");
        assert_eq!(game.legal_reason(Direction::Up), "wall");
        assert_eq!(game.legal_reason(Direction::Right), "reverse", "that square is the neck");
        assert_eq!(game.legal_reason(Direction::Down), "legal");
    }

    #[test]
    fn following_the_tail_is_legal_but_eating_first_makes_it_a_collision() {
        let mut game = small();
        // A ring of four: the head's right-hand neighbour is its own tail.
        let ring = [(1, 1), (1, 2), (2, 2), (2, 1)];

        with_body(&mut game, &ring, Some((5, 3)));
        assert_eq!(
            game.legal_reason(Direction::Right),
            "legal",
            "the tail vacates the square on a step that does not grow"
        );

        // Same board, except the step grows the snake — so the tail stays and the square is solid.
        with_body(&mut game, &ring, Some((2, 1)));
        assert_eq!(game.legal_reason(Direction::Right), "body");
    }

    #[test]
    fn walking_into_a_wall_kills_the_snake_rather_than_moving_it() {
        let mut game = small();
        with_body(&mut game, &[(0, 0), (1, 0)], Some((5, 3)));
        game.step(Direction::Left);
        assert!(!game.alive);
        assert_eq!(game.head(), (0, 0), "a fatal move is not also applied");
        assert_eq!(game.body.len(), 2);
    }

    #[test]
    fn a_dead_snake_does_not_move_again() {
        let mut game = small();
        game.alive = false;
        let before = game.body.clone();
        game.step(Direction::Up);
        game.step(Direction::Down);
        assert_eq!(game.body, before);
    }

    #[test]
    fn food_only_ever_appears_on_an_empty_square() {
        let mut game = small();
        with_body(&mut game, &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)], None);
        for _ in 0..200 {
            let food = game.spawn_food().expect("nineteen squares are free");
            assert!(!game.body.contains(&food));
            assert!(game.contains(food), "and on the board");
        }
    }

    #[test]
    fn a_full_board_has_nowhere_to_put_food() {
        let mut game = small();
        let everywhere: Vec<(i32, i32)> =
            (0..4).flat_map(|y| (0..6).map(move |x| (x, y))).collect();
        with_body(&mut game, &everywhere, None);
        assert_eq!(game.spawn_food(), None, "and says so rather than looping forever");
    }

    #[test]
    fn a_pocket_the_snake_has_sealed_off_is_not_reachable() {
        let mut game = small();
        // A wall of body down column 1, with the tail tucked out of the way at (2, 3) so that the
        // wall is solid: the flood fill treats only the *tail* as about to move.
        with_body(&mut game, &[(1, 0), (1, 1), (1, 2), (1, 3), (2, 3)], Some((4, 0)));

        let (food_found, pocket) = game.reachable((0, 0));
        assert_eq!(pocket, 4, "column 0 only: four squares, and the wall holds");
        assert!(!food_found, "the food is on the other side of the snake");

        let (food_found, open) = game.reachable((2, 0));
        assert!(food_found, "from the open side the food is there to be had");
        assert_eq!(open, 16, "columns 2 to 5, including the square the tail is leaving");
    }

    // ---- The tour the shield reasons over

    #[test]
    fn the_tour_visits_every_square_once_and_only_steps_to_neighbours() {
        for (width, height) in [(4, 4), (6, 4), (5, 4), (4, 5), (24, 16)] {
            let cycle = hamiltonian_cycle(width, height);
            assert_eq!(cycle.len(), (width * height) as usize, "{width}x{height}: every square");

            let mut seen = std::collections::HashSet::new();
            for &cell in &cycle {
                assert!((0..width).contains(&cell.0) && (0..height).contains(&cell.1));
                assert!(seen.insert(cell), "{width}x{height}: {cell:?} visited twice");
            }

            // Including the step from the last square back to the first: it is a cycle, and the
            // shield's arithmetic is modular because of it.
            for index in 0..cycle.len() {
                let (x1, y1) = cycle[index];
                let (x2, y2) = cycle[(index + 1) % cycle.len()];
                assert_eq!(
                    (x1 - x2).abs() + (y1 - y2).abs(),
                    1,
                    "{width}x{height}: {:?} -> {:?} is not a step",
                    cycle[index],
                    cycle[(index + 1) % cycle.len()]
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "at least one dimension must be even")]
    fn an_odd_by_odd_board_has_no_tour_and_says_so() {
        hamiltonian_cycle(5, 5);
    }

    // ---- The shield

    #[test]
    fn the_shield_keeps_the_snake_alive_until_the_board_is_full() {
        // The claim the whole right-hand panel makes. A board small enough to finish in a test,
        // three seeds so it is not one lucky game.
        for seed in [1u64, 0x5eed, 99] {
            let mut game = Game::new(6, 4, seed);
            let mut rng = Rng::new(seed);
            let mut steps = 0;
            while game.alive && !game.won && steps < 2000 {
                let decision = decide(&game, true, &mut rng);
                game.step(decision.executed);
                steps += 1;
            }
            assert!(game.alive, "seed {seed:#x}: the shield let it die after {steps} steps");
            assert!(game.won, "seed {seed:#x}: stalled after {steps} steps");
            assert_eq!(game.body.len(), game.capacity(), "the snake is the board");
            assert_eq!(game.food, None, "a full board has no room for more food");
        }
    }

    #[test]
    fn without_the_shield_the_policy_dies_which_is_why_the_shield_exists() {
        // A shield that never changes the outcome would make the panel decorative. The policy is
        // deliberately only mildly aware of the tour, so unguarded it walks into its own tail.
        let mut deaths = 0;
        for seed in [1u64, 0x5eed, 99] {
            let mut game = Game::new(6, 4, seed);
            let mut rng = Rng::new(seed);
            let mut steps = 0;
            while game.alive && !game.won && steps < 2000 {
                let decision = decide(&game, false, &mut rng);
                game.step(decision.executed);
                steps += 1;
            }
            if !game.alive {
                deaths += 1;
            }
        }
        assert_eq!(deaths, 3, "unguarded play survived, so the comparison on screen is empty");
    }

    #[test]
    fn the_snake_never_executes_an_illegal_move_while_a_legal_one_exists() {
        for guarded in [true, false] {
            let mut game = Game::new(BOARD_WIDTH, BOARD_HEIGHT, 0x5eed);
            let mut rng = Rng::new(7);
            for _ in 0..400 {
                if !game.alive || game.won {
                    break;
                }
                let decision = decide(&game, guarded, &mut rng);
                let any_legal = Direction::ALL.iter().any(|&d| game.legal_reason(d) == "legal");
                if any_legal {
                    assert_eq!(
                        game.legal_reason(decision.executed),
                        "legal",
                        "guarded={guarded}: chose {} into a {}",
                        decision.executed.label(),
                        game.legal_reason(decision.executed)
                    );
                }
                // The panel labels an override, so the flag has to mean exactly that.
                assert_eq!(decision.intervened, decision.executed != decision.proposed);
                game.step(decision.executed);
            }
        }
    }

    #[test]
    fn the_shield_is_off_when_it_is_switched_off() {
        // Unguarded, the executed move is the proposal unless the proposal is outright illegal.
        let game = Game::new(BOARD_WIDTH, BOARD_HEIGHT, 3);
        let mut rng = Rng::new(3);
        for _ in 0..50 {
            let decision = decide(&game, false, &mut rng);
            if game.legal_reason(decision.proposed) == "legal" {
                assert_eq!(decision.executed, decision.proposed);
                assert!(!decision.intervened);
            }
        }
    }

    #[test]
    fn the_reported_numbers_are_in_the_range_the_gauges_draw() {
        let mut game = Game::new(BOARD_WIDTH, BOARD_HEIGHT, 11);
        let mut rng = Rng::new(11);
        for _ in 0..300 {
            if !game.alive || game.won {
                break;
            }
            let decision = decide(&game, true, &mut rng);
            let total: f32 = decision.probabilities.iter().sum();
            assert!((total - 1.0).abs() < 1e-4, "probabilities sum to {total}");
            for probability in decision.probabilities {
                assert!((0.0..=1.0).contains(&probability), "{probability} is not a probability");
            }
            assert!((0.0..=1.0).contains(&decision.dead_end_risk));
            assert!((0.0..=1.0).contains(&decision.food_reachable));
            assert!(decision.visited > 0, "a decision that examined nothing is not a decision");
            game.step(decision.executed);
        }
    }

    // ---- The policy's arithmetic

    #[test]
    fn softmax_is_a_distribution_even_when_one_score_dominates() {
        let spread = softmax(&[1000.0, -1000.0, 0.0, 0.5], 0.45);
        let total: f32 = spread.iter().sum();
        assert!((total - 1.0).abs() < 1e-5, "sums to {total}");
        assert!(
            spread.iter().all(|value| value.is_finite()),
            "subtracting the peak avoids inf/NaN"
        );
        assert!(spread[0] > 0.99, "and the dominant score still wins");
    }

    #[test]
    fn softmax_of_equal_scores_is_a_flat_distribution() {
        assert_eq!(softmax(&[1.5; 4], 0.45), [0.25; 4]);
        // The illegal-move sentinel is -8.0, so four illegal moves must not divide by zero.
        let all_refused = softmax(&[-8.0; 4], 0.45);
        assert!(all_refused.iter().all(|value| (value - 0.25).abs() < 1e-6));
    }

    #[test]
    fn argmax_takes_the_peak_and_breaks_a_tie_by_order() {
        assert_eq!(argmax(&[0.1, 0.2, 0.6, 0.1]), Direction::Left);
        assert_eq!(argmax(&[0.25; 4]), Direction::Up, "the first of equals");
    }

    #[test]
    fn jitter_stays_inside_its_magnitude_and_moves_both_ways() {
        let mut rng = Rng::new(0x5eed);
        let mut negative = 0;
        let mut positive = 0;
        for _ in 0..10_000 {
            let value = rng.jitter(0.12);
            assert!((-0.12..=0.12).contains(&value), "{value} escaped the magnitude");
            if value < 0.0 {
                negative += 1;
            } else {
                positive += 1;
            }
        }
        assert!(negative > 4_000 && positive > 4_000, "{negative} down, {positive} up");
    }

    #[test]
    fn a_zero_seed_still_generates() {
        // xorshift is stuck at zero forever, which is why `new` sets the low bit.
        let mut rng = Rng::new(0);
        let values: Vec<u64> = (0..8).map(|_| rng.next_u64()).collect();
        assert!(values.iter().all(|&value| value != 0), "{values:?}");
        assert_eq!(values.iter().collect::<std::collections::HashSet<_>>().len(), 8);
    }

    #[test]
    fn the_same_seed_replays_the_same_game() {
        let play = |seed: u64| {
            let mut session = Session::new(seed);
            let now = Instant::now();
            for _ in 0..80 {
                session.step_now(now);
            }
            (session.game.body.clone(), session.game.score, session.decision.executed)
        };
        assert_eq!(play(0x5eed), play(0x5eed));
        assert_ne!(play(0x5eed).0, play(0x1234).0, "a different seed is a different game");
    }

    // ---- Session

    #[test]
    fn speed_saturates_at_both_ends_rather_than_wrapping() {
        let mut session = Session::new(1);
        for _ in 0..20 {
            session.faster();
        }
        assert_eq!(session.speed, Session::SPEEDS.len() - 1);
        assert_eq!(session.interval(), Duration::from_secs(1) / 45);

        for _ in 0..20 {
            session.slower();
        }
        assert_eq!(session.speed, 0, "and no underflow on the way down");
        assert_eq!(session.interval(), Duration::from_secs(1) / 3);
    }

    #[test]
    fn restarting_keeps_the_best_score_and_counts_the_round() {
        let mut session = Session::new(1);
        session.game.score = 7;
        session.restart(true);
        assert_eq!(session.best, 7, "the best survives the round that set it");
        assert_eq!(session.game.score, 0);
        assert_eq!(session.round, 2);
        assert!(!session.paused, "pressing R unpauses: it is a request to watch");

        session.paused = true;
        session.restart(false);
        assert!(session.paused, "but a round ending on its own does not resume for you");
    }

    #[test]
    fn a_finished_round_waits_long_enough_to_read_before_restarting() {
        let mut session = Session::new(1);
        session.game.alive = false;
        session.advance();
        assert_eq!(session.round, 1, "the game over is still on screen");
        assert!(session.ended.is_some(), "and the clock on it has started");

        session.ended = Instant::now().checked_sub(Session::RESTART_DELAY);
        session.advance();
        assert_eq!(session.round, 2);
        assert!(session.game.alive);
    }

    #[test]
    fn a_paused_session_does_not_advance_the_game() {
        let mut session = Session::new(1);
        session.paused = true;
        let before = session.game.body.clone();
        for _ in 0..50 {
            session.advance();
        }
        assert_eq!(session.game.body, before);
        assert_eq!(session.state_label(), "PAUSED");
    }

    #[test]
    fn the_state_label_says_what_is_actually_on_screen() {
        let mut session = Session::new(1);
        assert_eq!(session.state_label(), "LIVE");
        session.game.alive = false;
        assert_eq!(session.state_label(), "GAME OVER");
        session.game.won = true;
        assert_eq!(session.state_label(), "BOARD CLEAR", "a full board is not a death");
        session.paused = true;
        assert_eq!(session.state_label(), "PAUSED", "paused outranks everything");
    }

    #[test]
    fn the_decision_rate_counts_the_last_second_and_nothing_older() {
        let mut session = Session::new(1);
        session.paused = true;
        let now = Instant::now();
        if let Some(stale) = now.checked_sub(Duration::from_secs(3)) {
            session.recent.push_back(stale);
        }
        session.recent.push_back(now);
        session.advance();
        assert_eq!(session.decisions_per_second(), 1.0, "the three-second-old step is gone");
    }

    // ---- The composition

    #[test]
    fn the_composed_frame_is_the_reference_layout() {
        let session = Session::new(0x5eed);
        let rows = screen(&session, LAYOUT_WIDTH, LAYOUT_HEIGHT);
        assert_eq!(rows.len(), LAYOUT_HEIGHT as usize);

        assert!(rows[1].starts_with("   CONUI  /  LOCAL INTELLIGENCE"));
        assert!(rows[1].trim_end().ends_with("LIVE"), "{}", rows[1]);
        assert!(rows[4].starts_with("   S N A K E"));
        assert!(rows[4].contains("ROUND 01"));

        // The board: a rule of 48 dashes, two columns per square, closed at the bottom.
        // The right-hand column shares these rows, so the board is matched by prefix.
        let fence = "─".repeat(48);
        assert!(rows[TOP as usize].starts_with(&format!("   ┌{fence}┐")), "{}", rows[TOP as usize]);
        assert!(
            rows[BOTTOM as usize].starts_with(&format!("   └{fence}┘")),
            "{}",
            rows[BOTTOM as usize]
        );

        assert!(rows[7].contains("NEXT MOVE") && rows[7].contains("POLICY WEIGHTS"));
        for (index, direction) in Direction::ALL.into_iter().enumerate() {
            assert!(rows[9 + index].contains(direction.label()), "{}", rows[9 + index]);
        }
        assert!(
            rows[14].contains("EXECUTING") && rows[14].contains(session.decision.executed.label())
        );
        assert!(rows[16].contains("DEAD-END RISK"));
        assert!(rows[19].contains("FOOD REACHABLE"));
        assert!(rows[22].contains("INFERENCE") && rows[22].contains("ms"));
        assert!(rows[26].contains("ENGINE") && rows[26].contains("conui · Rust"));
        assert!(rows[28].contains("conui + cycle safety"));
        assert!(rows[29].contains("Shield interventions  0000"));

        // Footer: the keys on the left, the policy and a clock on the right.
        let footer = &rows[LAYOUT_HEIGHT as usize - 2];
        assert!(footer.contains("SPACE pause") && footer.contains("Q quit"));
        assert!(footer.contains("LOCAL HEURISTIC POLICY") && footer.trim_end().ends_with("00:00"));
    }

    #[test]
    fn exactly_one_move_is_marked_as_the_proposal() {
        let session = Session::new(0x5eed);
        let rows = screen(&session, LAYOUT_WIDTH, LAYOUT_HEIGHT);
        let marked: Vec<&str> = (0..4)
            .filter(|index| rows[9 + index].contains('›'))
            .map(|index| Direction::ALL[index].label())
            .collect();
        assert_eq!(marked, vec![session.decision.proposed.label()]);
    }

    #[test]
    fn the_board_draws_the_snake_two_columns_wide_and_the_food_where_it_is() {
        let session = Session::new(0x5eed);
        let rows = screen(&session, LAYOUT_WIDTH, LAYOUT_HEIGHT);

        // Counted inside the board's own columns: the policy gauges to the right are drawn with
        // the same block glyph on the same rows.
        let board_rows = (TOP + 1)..=(TOP + BOARD_HEIGHT);
        let inside = |y: i32, glyph: char| {
            rows[y as usize]
                .chars()
                .skip((LEFT + 1) as usize)
                .take((BOARD_WIDTH * 2) as usize)
                .filter(|found| *found == glyph)
                .count()
        };
        let squares: usize = board_rows.clone().map(|y| inside(y, '█')).sum();
        assert_eq!(squares, session.game.body.len() * 2, "a square is two cells wide");

        let (head_x, head_y) = session.game.head();
        assert_eq!(cell(&rows, LEFT + 1 + 2 * head_x, TOP + 1 + head_y), '█');

        let (food_x, food_y) = session.game.food.expect("a fresh board has food");
        assert_eq!(cell(&rows, LEFT + 1 + 2 * food_x, TOP + 1 + food_y), '●');

        // Empty space is the mesh, not blanks: a board you can measure distance on.
        let mesh: usize = board_rows.map(|y| inside(y, '·')).sum();
        let board = (BOARD_WIDTH * BOARD_HEIGHT) as usize;
        assert_eq!(mesh, board - session.game.body.len() - 1, "every square but the snake's own");
    }

    #[test]
    fn a_fixed_composition_is_centred_in_a_bigger_window_rather_than_stretched() {
        let session = Session::new(1);
        let rows = screen(&session, LAYOUT_WIDTH + 16, LAYOUT_HEIGHT + 6);
        let header = rows
            .iter()
            .position(|row| row.contains("CONUI  /  LOCAL INTELLIGENCE"))
            .expect("the header is drawn somewhere");
        assert_eq!(header, 3 + 1, "pushed down by half the spare height");
        assert_eq!(
            rows[header].find('C'),
            Some(8 + LEFT as usize),
            "and right by half the spare width"
        );
    }

    #[test]
    fn the_header_turns_red_and_says_so_when_the_snake_dies() {
        let mut session = Session::new(1);
        session.game.alive = false;
        let rows = screen(&session, LAYOUT_WIDTH, LAYOUT_HEIGHT);
        assert!(rows[1].trim_end().ends_with("GAME OVER"), "{}", rows[1]);
    }

    #[test]
    fn switching_the_shield_off_is_visible_on_the_panel() {
        let mut session = Session::new(1);
        session.guarded = false;
        let rows = screen(&session, LAYOUT_WIDTH, LAYOUT_HEIGHT);
        assert!(rows[28].contains("conui · shield OFF"), "{}", rows[28]);
    }

    #[test]
    fn a_window_smaller_than_the_composition_clips_instead_of_panicking() {
        // `App` refuses to draw below `min_size`, but a `Frame` is handed whatever exists, and a
        // canvas that panicked at the edge would make every absolute coordinate a hazard.
        let session = Session::new(1);
        for (width, height) in [(1, 1), (40, 12), (LAYOUT_WIDTH - 1, LAYOUT_HEIGHT - 1)] {
            let rows = screen(&session, width, height);
            assert_eq!(rows.len(), height as usize);
        }
    }
}
