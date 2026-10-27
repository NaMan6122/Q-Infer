//! `qinfer` — the terminal surface.
//!
//! ADR-0009: this is a *view* over the core's state, not a second product.
//! It reads `cfp@1` and the spike results from disk and renders them. It opens
//! no socket, spawns no daemon, and holds no privilege.
//!
//! `--snapshot WxH` renders one frame to an offscreen buffer and prints it.
//! That exists so the layout can be reviewed in CI and over a pipe, and so
//! "does the dashboard look right" never requires a human at a TTY.

mod cli;

use std::io::{self, Stdout};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use qinfer_core::catalog::Catalog;
use qinfer_core::planner::{Constraints, PlanRequest, PlanSet, Planner};
use qinfer_core::predict::{fit, Observation};
use qinfer_core::setup::{self, SetupEvent, Stage};
use qinfer_core::sweetspot::{self, SweetSpot, DEFAULT_USABLE_TPS};
use qinfer_core::{Fingerprint, Space, Workspace};

/// The catalogue ships with the binary. A network fetch belongs in the
/// provisioner (M1), not in the view.
const CATALOG_JSON: &str = include_str!("../../../catalog/qinfer-catalog-2026.10.27.json");
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame, Terminal,
};

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    /// Where a new user starts: the measurement ladder, before any number on
    /// any other tab means anything.
    Setup,
    Bench,
    /// The capability/speed curve, and the recommendation drawn from it.
    /// This is the tab that answers "what should I run", which the ranked plan
    /// list alone cannot: throughput ordering always favours the smallest model.
    Frontier,
    Plans,
    Help,
}

impl Tab {
    const ALL: [Tab; 6] =
        [Tab::Overview, Tab::Setup, Tab::Bench, Tab::Frontier, Tab::Plans, Tab::Help];

    /// The tab bar, in display order. One list, so the arrow keys, the number
    /// keys and the rendered bar cannot disagree about what exists.
    const LABELS: [&'static str; 6] =
        ["Overview", "Setup", "Bench", "Frontier", "Plans", "Help"];

    fn index(self) -> usize {
        match self {
            Tab::Overview => 0,
            Tab::Setup => 1,
            Tab::Bench => 2,
            Tab::Frontier => 3,
            Tab::Plans => 4,
            Tab::Help => 5,
        }
    }

    fn next(self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    fn prev(self) -> Tab {
        Tab::ALL[(self.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }
}

struct App {
    ws: Workspace,
    tab: Tab,
    table: TableState,
    plan_table: TableState,
    /// Ranked plans for this machine, computed once at load. `None` means we
    /// could not plan, and `plan_error` says why — never an empty list pretending
    /// there was nothing to offer.
    plans: Option<PlanSet>,
    plan_error: Option<String>,
    /// The frontier computed once at load, alongside the plans. `None` means we
    /// could not build one, and `plan_error` says why.
    spot: Option<SweetSpot>,
    /// The resolved memory budget, so the dashboard and the planner quote the
    /// same number (T-012). `None` only when there is no fingerprint at all.
    space: Option<Space>,
    /// A ladder run in flight. The receiver is the entire interface to the
    /// worker thread (ADR-0009: no socket, no daemon, no shared state).
    run_rx: Option<Receiver<SetupEvent>>,
    run_events: Vec<SetupEvent>,
    /// The fingerprint the current (or last) ladder run collected. Preferred
    /// over the one on disk, because the whole point of pressing Enter is that
    /// the file is older than the machine (T-016).
    live_fp: Option<Fingerprint>,
    status: String,
    refreshed: Instant,
}

/// What a rung of the ladder is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rung {
    Pending,
    Running,
    Done,
    Failed,
}

impl App {
    fn new(ws: Workspace) -> Self {
        let status = if ws.has_data() {
            format!("loaded {}", short_path(&ws.root))
        } else {
            ws.error.clone().unwrap_or_else(|| "no data".into())
        };
        let a = analyse(ws.fingerprint.as_ref());
        let mut plan_table = TableState::default();
        if a.plans.as_ref().map(|p| !p.plans.is_empty()).unwrap_or(false) {
            plan_table.select(Some(0));
        }
        App {
            ws,
            tab: Tab::Overview,
            table: TableState::default(),
            plan_table,
            plans: a.plans,
            plan_error: a.error,
            spot: a.spot,
            space: a.space,
            run_rx: None,
            run_events: Vec::new(),
            live_fp: None,
            status,
            refreshed: Instant::now(),
        }
    }

    fn reload(&mut self) {
        self.ws.reload();
        self.status = if self.ws.has_data() {
            format!("reloaded {} at {}", short_path(&self.ws.root), clock())
        } else {
            self.ws.error.clone().unwrap_or_else(|| "no data".into())
        };
        self.reanalyse();
        self.refreshed = Instant::now();
    }

    /// Re-derive every tab from the freshest fingerprint available.
    ///
    /// A live fingerprint beats the one on disk: the reason to press Enter is
    /// that the file predates the machine. `r` deliberately keeps that
    /// preference rather than silently reverting to a staler source.
    fn reanalyse(&mut self) {
        let fp = self.live_fp.clone().or_else(|| self.ws.fingerprint.clone());
        let a = analyse(fp.as_ref());
        self.plans = a.plans;
        self.plan_error = a.error;
        self.spot = a.spot;
        self.space = a.space;
    }

    fn plan_rows(&self) -> usize {
        self.plans.as_ref().map(|p| p.plans.len()).unwrap_or(0)
    }

    fn move_plan(&mut self, delta: isize) {
        let len = self.plan_rows();
        if len == 0 {
            return;
        }
        let cur = self.plan_table.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        self.plan_table.select(Some(next));
    }

    fn move_row(&mut self, delta: isize) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        self.table.select(Some(next));
    }

    fn row_count(&self) -> usize {
        self.ws.results.as_ref().map(|r| r.rows.len()).unwrap_or(0)
    }

    /// Start a measurement run. Refuses to start a second one: two ladders
    /// competing for the memory bandwidth they are trying to measure would
    /// corrupt both (PRD §5.5 noise discipline).
    fn start_run(&mut self) {
        if self.run_rx.is_some() {
            self.status = "a measurement is already running".into();
            return;
        }
        let catalog = match Catalog::from_json(CATALOG_JSON) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("catalogue parse failed: {e}");
                return;
            }
        };
        self.run_events.clear();
        self.status = "measuring this machine …".into();
        self.run_rx = Some(setup::spawn(catalog, DEFAULT_USABLE_TPS));
    }

    /// Drain whatever the worker has produced since the last frame. Non-blocking
    /// so the UI never stalls behind a slow rung.
    fn poll_run(&mut self) {
        let mut batch: Vec<SetupEvent> = Vec::new();
        let mut latest_fp: Option<Fingerprint> = None;
        let mut finished = false;
        if let Some(rx) = &self.run_rx {
            while let Ok(ev) = rx.try_recv() {
                match &ev {
                    SetupEvent::Done => finished = true,
                    SetupEvent::Fingerprint(fp) => latest_fp = Some((**fp).clone()),
                    _ => {}
                }
                batch.push(ev);
            }
        }
        if batch.is_empty() {
            return;
        }
        if let Some(fp) = latest_fp {
            self.live_fp = Some(fp);
        }
        self.run_events.extend(batch);
        if finished {
            self.run_rx = None;
            self.status = format!("measured at {}", clock());
            // Re-derive from the fingerprint the ladder just collected, so the
            // other tabs stop quoting a budget captured months ago.
            self.reanalyse();
        }
    }

    fn rung(&self, stage: Stage) -> Rung {
        let mut begun = false;
        for e in &self.run_events {
            match e {
                SetupEvent::Begin(s) if *s == stage => begun = true,
                SetupEvent::Stage(p) if p.stage == stage => return Rung::Done,
                SetupEvent::Failed { stage: s, .. } if *s == stage => return Rung::Failed,
                _ => {}
            }
        }
        if begun {
            Rung::Running
        } else {
            Rung::Pending
        }
    }

    fn rung_detail(&self, stage: Stage) -> Option<&str> {
        self.run_events.iter().find_map(|e| match e {
            SetupEvent::Stage(p) if p.stage == stage => Some(p.detail.as_str()),
            SetupEvent::Failed { stage: s, reason } if *s == stage => Some(reason.as_str()),
            _ => None,
        })
    }

    fn is_running(&self) -> bool {
        self.run_rx.is_some()
    }
}

/// Everything the dashboard derives in one pass, so the tabs cannot disagree
/// with each other about what the machine can do.
struct Analysis {
    plans: Option<PlanSet>,
    error: Option<String>,
    spot: Option<SweetSpot>,
    space: Option<Space>,
}

fn analysis_error(msg: impl Into<String>) -> Analysis {
    Analysis { plans: None, error: Some(msg.into()), spot: None, space: None }
}

/// Calibrate from the catalogue's own measurements and rank plans for this
/// machine. Deriving the calibration from the catalogue (rather than from the
/// spike's JSON) means the dashboard and the planner cannot disagree about what
/// the hardware did.
fn analyse(source: Option<&Fingerprint>) -> Analysis {
    let Some(fp) = source else {
        return analysis_error("no fingerprint to plan against");
    };

    let catalog = match Catalog::from_json(CATALOG_JSON) {
        Ok(c) => c,
        Err(e) => return analysis_error(format!("catalogue parse failed: {e}")),
    };
    if let Err(v) = catalog.validate() {
        return analysis_error(format!(
            "catalogue invalid: {}",
            v.first().map(|x| x.to_string()).unwrap_or_default()
        ));
    }

    let bw = fp.saturated_gbs();
    if !(bw > 0.0) || !bw.is_finite() {
        return analysis_error("bandwidth unusable — probe it again");
    }

    let observations: Vec<Observation> = catalog
        .entries
        .iter()
        .filter(|e| e.is_plannable())
        .filter_map(|e| {
            e.measurements.first().map(|m| Observation {
                bytes_per_token: e.bytes_per_token(),
                measured_decode_tps: m.decode_tps,
                shape: e.model.shape,
            })
        })
        .collect();

    if observations.len() < 3 {
        return analysis_error(format!(
            "{} on-device measurements — 3 are needed to calibrate",
            observations.len()
        ));
    }

    let calibration = fit(&observations, bw);
    // Older fingerprints predate `arch_id`. Deriving it from the brand is a
    // deterministic mapping of a stated fact, not a plausible default — and it
    // is far better than refusing to plan against a machine we can identify.
    let arch = fp
        .cpu
        .arch_id
        .clone()
        .unwrap_or_else(|| qinfer_core::collect::slugify(&fp.cpu.brand));

    // The budget comes from `space`, not from "half of RAM" (T-012). The view
    // passes no device report — measuring belongs to the ladder, not the
    // dashboard — so this degrades to `vm_stat` reclaimable or, failing that,
    // a budget the UI labels as assumed.
    let space = qinfer_core::space::resolve(fp, None);
    let available = space.plan_budget();

    let planner = Planner::new(&catalog, &calibration, bw);
    let set = planner.plan(&PlanRequest::default(), &Constraints::new(arch, available));
    let spot = sweetspot::frontier(&set, &catalog);

    Analysis { plans: Some(set), error: None, spot: Some(spot), space: Some(space) }
}

fn clock() -> String {
    // Deliberately dependency-free: a local HH:MM:SS is enough for a status bar.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

// ---------------------------------------------------------------------------
// entry points
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // A subcommand means "use the CLI"; anything else (including --snapshot)
    // means "open the dashboard". The binary is one surface with two modes,
    // not two programs.
    const SUBCOMMANDS: [&str; 10] =
        ["detect", "plan", "spec-lint", "help", "version", "--help", "-h", "--version", "-V", "--json"];
    if let Some(first) = args.first() {
        if SUBCOMMANDS.contains(&first.as_str()) {
            std::process::exit(cli::run(&args));
        }
    }

    // --snapshot WxH  → render one frame offscreen and print it
    if let Some(pos) = args.iter().position(|a| a == "--snapshot") {
        let dims = args.get(pos + 1).map(String::as_str).unwrap_or("100x30");
        let (w, h) = parse_dims(dims).unwrap_or((100, 30));
        let tab = args
            .iter()
            .position(|a| a == "--tab")
            .and_then(|p| args.get(p + 1))
            .map(|s| s.as_str())
            .unwrap_or("overview");
        if let Err(e) = snapshot(w, h, tab) {
            eprintln!("qinfer: snapshot failed: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = run_tui() {
        eprintln!("qinfer: {e}");
        std::process::exit(1);
    }
}

fn parse_dims(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn snapshot(width: u16, height: u16, tab: &str) -> io::Result<()> {
    let mut app = App::new(Workspace::discover());
    // Matched against `Tab::LABELS` case-insensitively, so a new tab cannot be
    // added without this mapping noticing — the fallback is Overview only for a
    // genuinely unknown name.
    app.tab = Tab::ALL
        .into_iter()
        .find(|t| Tab::LABELS[t.index()].eq_ignore_ascii_case(tab))
        .unwrap_or(Tab::Overview);

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|f| ui(f, &mut app))?;

    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        for x in 0..width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    print!("{out}");
    Ok(())
}

fn run_tui() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let res = event_loop(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let mut app = App::new(Workspace::discover());

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    KeyCode::Tab | KeyCode::Right => app.tab = app.tab.next(),
                    KeyCode::BackTab | KeyCode::Left => app.tab = app.tab.prev(),
                    KeyCode::Char('1') => app.tab = Tab::Overview,
                    KeyCode::Char('2') => app.tab = Tab::Setup,
                    KeyCode::Char('3') => app.tab = Tab::Bench,
                    KeyCode::Char('4') => app.tab = Tab::Frontier,
                    KeyCode::Char('5') => app.tab = Tab::Plans,
                    KeyCode::Char('?') => app.tab = Tab::Help,
                    KeyCode::Enter => {
                        if app.tab == Tab::Setup {
                            app.start_run();
                        }
                    }
                    KeyCode::Char('r') => app.reload(),
                    KeyCode::Down | KeyCode::Char('j') => match app.tab {
                        Tab::Bench => app.move_row(1),
                        Tab::Plans => app.move_plan(1),
                        _ => {}
                    },
                    KeyCode::Up | KeyCode::Char('k') => match app.tab {
                        Tab::Bench => app.move_row(-1),
                        Tab::Plans => app.move_plan(-1),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        // Drain the worker. This is the only mutable path in the dashboard and
        // it goes through a channel, never a socket.
        app.poll_run();

        // Cheap freshness indicator; no filesystem polling yet.
        let _ = Instant::now().duration_since(app.refreshed);
    }
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            // The header is fixed at 4 (border, title, tabs, border) and the
            // status bar at 1, so the body must be allowed to shrink below 10
            // or ratatui satisfies the minimum by starving the header — at
            // 46x12 the machine name and the tab bar disappeared entirely,
            // which is the one thing a degraded layout must never drop.
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);

    match app.tab {
        Tab::Overview => render_overview(f, app, chunks[1]),
        Tab::Setup => render_setup(f, app, chunks[1]),
        Tab::Bench => render_bench(f, app, chunks[1]),
        Tab::Frontier => render_frontier(f, app, chunks[1]),
        Tab::Plans => render_plans(f, app, chunks[1]),
        Tab::Help => render_help(f, chunks[1]),
    }

    render_status(f, app, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let name = app
        .ws
        .fingerprint
        .as_ref()
        .map(|fp| fp.cpu.brand.clone())
        .unwrap_or_else(|| "no machine".into());

    // ADR-0002: never show a confident default. Certification now requires the
    // fingerprint to have passed validation, not merely to exist.
    let (tier, tier_colour) = if app.ws.is_trusted() {
        ("Tier A (certified)", Color::Green)
    } else if app.ws.has_data() {
        ("unverified — see Verdict", Color::Yellow)
    } else {
        ("no data", Color::DarkGray)
    };

    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // FR-59: fit the title to the panel instead of letting it run into the
    // border. The raw Line was clipped mid-word at 46 columns, which reads as a
    // rendering fault rather than as a narrow terminal. The certification tier
    // is dropped before the machine name is, because "which box am I on" is the
    // question this row exists to answer.
    let chip = " Q-Infer ";
    let tier_part = format!("  ·  {tier}");
    let avail = inner.width as usize;
    let remaining = avail.saturating_sub(chip.len() + 1);
    let mut spans = vec![
        Span::styled(chip, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
    ];
    if remaining >= tier_part.chars().count() + 6 {
        let nb = remaining - tier_part.chars().count();
        spans.push(Span::styled(truncate(&name, nb), Style::default().add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(tier_part, Style::default().fg(tier_colour)));
    } else {
        spans.push(Span::styled(
            truncate(&name, remaining.max(1)),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    let title = Line::from(spans);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    f.render_widget(Paragraph::new(title), rows[0]);

    f.render_widget(Paragraph::new(tab_bar(app, inner.width)), rows[1]);
}

/// The tab bar, built as a plain `Line` rather than through the `Tabs` widget.
///
/// `Tabs` pads every label invisibly, so a width computed from the label lengths
/// clipped the final tab no matter how the threshold was tuned. Owning the
/// layout means the fit is exact and the arithmetic is the same one the test
/// asserts against.
///
/// Shedding order: full names, then abbreviations, then the digits that match
/// the number keys — the bar loses labels before it loses the ability to
/// navigate.
fn tab_bar(app: &App, width: u16) -> Line<'static> {
    let full: Vec<&'static str> = Tab::LABELS.to_vec();
    let short: Vec<&'static str> = vec!["Over", "Setup", "Bench", "Front", "Plans", "Help"];
    let nums: Vec<&'static str> = vec!["1", "2", "3", "4", "5", "?"];

    let candidates: [(Vec<&'static str>, &'static str); 6] = [
        (full.clone(), " | "),
        (full, " "),
        (short.clone(), " | "),
        (short, " "),
        (nums.clone(), " | "),
        (nums, ""),
    ];

    let fit = |labels: &[&'static str], div: &str| -> usize {
        labels.iter().map(|l| l.chars().count()).sum::<usize>()
            + div.chars().count() * labels.len().saturating_sub(1)
    };

    let (labels, divider) = candidates
        .into_iter()
        .find(|(l, d)| fit(l, d) <= width as usize)
        .unwrap_or((vec!["1", "2", "3", "4", "5", "?"], ""));

    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(divider, Style::default().fg(Color::DarkGray)));
        }
        let style = if i == app.tab.index() {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(*label, style));
    }
    Line::from(spans)
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let hint = match app.tab {
        Tab::Setup if app.is_running() => "  measuring…  ·  q",
        Tab::Setup => "  enter  run  ·  tab  ·  ?  ·  q",
        Tab::Bench | Tab::Plans => "  ↑↓  ·  r  ·  tab  ·  q",
        _ => "  tab  ·  ?  ·  q",
    };
    let line = Line::from(vec![
        Span::styled(format!(" {} ", app.status), Style::default().fg(Color::DarkGray)),
        Span::styled(hint, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

// --- Setup -----------------------------------------------------------------

/// The ladder: what the tool is about to measure, and what it found.
///
/// This tab is the entry point. Every other tab shows a number that is only
/// meaningful once a run has happened here, so an empty dashboard should lead
/// someone to press Enter rather than to read a stale figure from disk.
fn render_setup(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(" Setup — measure this machine, then choose ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    const PREFIX: usize = 16;
    let avail = (inner.width as usize).saturating_sub(PREFIX).max(6);
    let mut lines: Vec<Line> = Vec::new();

    for (i, stage) in Stage::ALL.iter().enumerate() {
        let state = app.rung(*stage);
        let (marker, style) = match state {
            Rung::Pending => (" ", Style::default().fg(Color::DarkGray)),
            Rung::Running => (">", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Rung::Done => ("+", Style::default().fg(Color::Green)),
            Rung::Failed => ("x", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        };
        let detail = app.rung_detail(*stage).unwrap_or(match state {
            Rung::Pending => "not yet measured",
            Rung::Running => "working...",
            _ => "",
        });
        let detail_style = match state {
            Rung::Pending => Style::default().fg(Color::DarkGray),
            Rung::Failed => Style::default().fg(Color::Red),
            _ => Style::default(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {marker} {}. {:<9} ", i + 1, stage.label()), style),
            Span::styled(truncate(detail, avail), detail_style),
        ]));
    }

    lines.push(Line::from(""));

    if app.run_events.is_empty() {
        // The honest empty state: nothing here is a measurement yet, and saying
        // so is more useful than showing last month's numbers with confidence.
        lines.push(Line::from(Span::styled(
            " Nothing has been measured on this machine in this session.",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            " Press Enter to run the ladder. It takes a few seconds, needs no",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            " privileges, and installs nothing.",
            Style::default().fg(Color::DarkGray),
        )));
    } else if app.is_running() {
        lines.push(Line::from(Span::styled(
            " Measuring. The machine must stay quiet or the figures will be noise.",
            Style::default().fg(Color::Cyan),
        )));
    } else if let Some(spot) = app.spot.as_ref() {
        lines.push(Line::from(Span::styled(
            format!(" {}", spot.usable_reason(DEFAULT_USABLE_TPS)),
            Style::default().fg(Color::Green),
        )));
        for n in spot.notes.iter().take(1) {
            lines.push(Line::from(Span::styled(format!(" {n}"), Style::default().fg(Color::Yellow))));
        }
    } else if let Some(err) = app.plan_error.as_ref() {
        lines.push(Line::from(Span::styled(format!(" {err}"), Style::default().fg(Color::Red))));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// --- Frontier --------------------------------------------------------------

/// The curve, and the recommendation drawn from it.
///
/// This tab exists because the ranked list cannot answer "what should I run":
/// ordering by throughput always crowns the smallest artifact. Here the user
/// sees the trade, the floor that was applied, and — critically — that the
/// capability axis is a **prior** until it has been measured (T-014, T-015).
fn render_frontier(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Frontier — capability against speed ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(spot) = app.spot.as_ref() else {
        let msg = app.plan_error.clone().unwrap_or_else(|| "no frontier to show".into());
        f.render_widget(Paragraph::new(msg).wrap(Wrap { trim: true }), inner);
        return;
    };

    if spot.is_empty() {
        f.render_widget(
            Paragraph::new(spot.usable_reason(DEFAULT_USABLE_TPS)).wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }

    let floor = DEFAULT_USABLE_TPS;
    let best = spot.best_usable(floor);
    let mut lines: Vec<Line> = Vec::new();
    // FR-59: corroborating columns are shed in a defined order rather than
    // wrapping into a second line that reads like a separate row. The order is
    // source, then capability, then the display name — the value the user came
    // for (tok/s and GiB) is the last thing to go.
    const FULL: u16 = 84;
    const MEDIUM: u16 = 64;
    let (show_source, show_cap) = (inner.width >= FULL, inner.width >= MEDIUM);
    // Width consumed by everything except the name:
    //   " <marker> " 4 + "   53 tok/s" 12 + "    4.8 GiB" 11 + "  cap 1.00" 10 + "  prior (params)" 16
    let overhead: u16 = if show_source { 53 } else if show_cap { 37 } else { 27 };
    let name_budget = inner.width.saturating_sub(overhead).max(8) as usize;

    for (i, p) in spot.frontier.iter().enumerate() {
        let chosen = best.map(|b| b.plan.id == p.plan.id).unwrap_or(false);
        let is_knee = Some(i) == spot.knee;
        // ASCII only: a glyph is not a reliable thing to send down a pipe.
        let marker = if chosen {
            "->"
        } else if is_knee {
            " ."
        } else {
            "  "
        };
        let base = if chosen {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let dim = Style::default().fg(Color::DarkGray);
        let name = truncate(&p.plan.display, name_budget.max(8));
        let mut spans = vec![
            Span::styled(format!(" {marker} "), base),
            Span::styled(format!("{name:<width$}", width = name_budget.max(8)), base),
            Span::styled(format!("{:>6.0} tok/s", p.tps()), base),
            Span::styled(
                format!("  {:>5.1} GiB", p.plan.peak_bytes as f64 / (1u64 << 30) as f64),
                dim,
            ),
        ];
        if show_cap {
            spans.push(Span::styled(format!("  cap {:.2}", p.capability), dim));
        }
        if show_source {
            spans.push(Span::styled(format!("  {}", p.source.label()), dim));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(" {:.0}+ tok/s: {}", floor, spot.usable_reason(floor)),
        Style::default().fg(Color::Cyan),
    )));
    for n in &spot.notes {
        lines.push(Line::from(Span::styled(format!(" {n}"), Style::default().fg(Color::Yellow))));
    }

    // `trim: false` matters: with `trim: true` ratatui strips the leading spaces
    // that carry the row markers, and every row silently loses its status.
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}



// --- Overview --------------------------------------------------------------

fn render_overview(f: &mut Frame, app: &App, area: Rect) {
    let Some(fp) = app.ws.fingerprint.as_ref() else {
        render_empty(f, app, area);
        return;
    };

    // ADR-0009: below 76 columns the panels stack, and detail is truncated with
    // an explicit ellipsis rather than allowed to overflow into the border.
    if area.width < 76 {
        // One combined panel: at this width two stacked panels cannot both be
        // given honest height, so we merge rather than clip (ADR-0009).
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Length(4), Constraint::Min(5)])
            .split(area);
        render_machine_compact(f, fp, rows[0]);
        render_bandwidth(f, fp, app.ws.is_trusted(), rows[1]);
        render_headline(f, app, fp, rows[2]);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            // 8, not 7: the pressure warning gets its own row rather than being
            // truncated off the end of the Memory field (Critic round 3, N8).
            .constraints([Constraint::Length(8), Constraint::Length(5), Constraint::Min(5)])
            .split(area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);
        render_machine(f, fp, cols[0]);
        render_capabilities(f, fp, cols[1]);
        render_bandwidth(f, fp, app.ws.is_trusted(), rows[1]);
        render_headline(f, app, fp, rows[2]);
    }
}

fn render_machine(f: &mut Frame, fp: &Fingerprint, area: Rect) {
    let block = panel(" Machine ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let gpu = fp.gpu.model.clone().unwrap_or_else(|| "—".into());
    let gpu_line = format!(
        "{}  {} cores  {}",
        gpu,
        fp.gpu.cores.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        pretty_backend(fp.gpu.metal_support.as_deref())
    );

    // `swap_used_bytes: None` must not render as "0.0 GiB" — that reads as
    // "no swap in use" when the truth is "not reported" (Critic round 3, N8).
    let swap = if fp.memory.swap_used_bytes.is_none() {
        "swap unknown".to_string()
    } else {
        format!("swap {:.1} GiB", fp.swap_gib())
    };


    // Panel width is not ours to assume; every value is fitted explicitly so
    // it can never bleed into the border (ADR-0009).
    let avail = (inner.width as usize).saturating_sub(15).max(6);

    let mut lines = vec![
        kv("CPU", &truncate(&format!("{}  ({}P/{}L)", fp.cpu.brand, fp.cpu.physical_cores, fp.cpu.logical_cores), avail)),
        kv("Clusters", &truncate(&fp.perf_summary(), avail)),
        kv("GPU", &truncate(gpu_line.trim_end(), avail)),
        kv("Memory", &truncate(&format!("{:.1} GiB   {swap}", fp.total_gib()), avail)),
        kv(
            "OS",
            &truncate(
                &format!(
                    "{} {} · {}",
                    pretty_os(&fp.identity.os),
                    fp.identity.os_version.clone().unwrap_or_default(),
                    fp.identity.arch
                ),
                avail,
            ),
        ),
    ];

    // The warning gets its own row: inline it was the first thing truncated.
    match fp.pressure() {
        qinfer_core::PressureState::Pressured { .. } => lines.push(Line::from(Span::styled(
            truncate(" ⚠ swapping — bandwidth provisional", inner.width as usize),
            Style::default().fg(Color::Yellow),
        ))),
        qinfer_core::PressureState::Unknown => lines.push(Line::from(Span::styled(
            truncate(" ⚠ memory pressure not reported", inner.width as usize),
            Style::default().fg(Color::DarkGray),
        ))),
        qinfer_core::PressureState::Relaxed { .. } => {}
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_machine_compact(f: &mut Frame, fp: &Fingerprint, area: Rect) {
    let block = panel(" Machine ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let gpu = fp.gpu.model.clone().unwrap_or_else(|| "—".into());
    let isa = fp.isa_enabled();
    let isa_display: Vec<&str> = isa.iter().map(|s| s.trim_start_matches("FEAT_")).collect();
    let avail = (inner.width as usize).saturating_sub(15).max(4);

    let lines = vec![
        kv("CPU", &format!("{}  ({} cores)", fp.cpu.brand, fp.cpu.physical_cores)),
        kv("GPU", &format!("{} · {} cores", gpu, fp.gpu.cores.map(|c| c.to_string()).unwrap_or_else(|| "?".into()))),
        kv("Memory", &format!("{:.1} GiB · swap {:.1} GiB", fp.total_gib(), fp.swap_gib())),
        kv("OS", &format!("{} {} · {}", pretty_os(&fp.identity.os), fp.identity.os_version.clone().unwrap_or_default(), fp.identity.arch)),
        kv("ISA", &truncate(&isa_display.join(" · "), avail)),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_capabilities(f: &mut Frame, fp: &Fingerprint, area: Rect) {
    let block = panel(" Capabilities ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let isa = fp.isa_enabled();
    let isa_display: Vec<&str> = isa.iter().map(|s| s.trim_start_matches("FEAT_")).collect();
    let isa_join = if isa_display.is_empty() { "—".to_string() } else { isa_display.join(" · ") };
    let avail = (inner.width as usize).saturating_sub(15).max(4);

    let mut lines = vec![
        kv("ISA", &truncate(&isa_join, avail)),
        kv("Unified", if fp.gpu.unified_memory { "yes" } else { "no" }),
    ];
    if let Some(n) = fp.bandwidth.best_threads {
        lines.push(kv("Best threads", &n.to_string()));
    }
    if let Some(l3) = fp.cpu.l3_bytes {
        lines.push(kv("L3", &format!("{} MiB", l3 / (1 << 20))));
    }
    lines.push(kv("Collected", &short_time(&fp.collected_at)));

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_bandwidth(f: &mut Frame, fp: &Fingerprint, trusted: bool, area: Rect) {
    // ADR-0002 / Critic N8: a rejected fingerprint must not be labelled
    // "measured" or rendered in the colour that means "this is fine".
    let title = if trusted {
        " Memory bandwidth (measured, not declared) "
    } else {
        " Memory bandwidth (unverified — do not use for planning) "
    };
    let block = panel(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sat = fp.saturated_gbs();
    let st = fp.single_thread_gbs();
    let gain = fp.thread_gain();
    let bar_w = (inner.width as usize).saturating_sub(34).clamp(6, 40);
    let show_gain = inner.width >= 84;
    let accent = if trusted { Color::Cyan } else { Color::DarkGray };

    let mut second = vec![
        Span::styled(
            format!(" {:<2} threads  ", fp.bandwidth.best_threads.unwrap_or(0)),
            Style::default().fg(accent),
        ),
        Span::styled(bar(sat, sat, bar_w), Style::default().fg(accent)),
        Span::styled(
            format!("  {:>6.1} GB/s", sat),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
    ];
    // Never fabricate a gain: if single-thread is unusable, say nothing.
    if let (true, Some(g)) = (show_gain, gain) {
        second.push(Span::styled(
            format!("   ({g:.2}× single-thread)"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let lines = vec![
        Line::from(vec![
            Span::styled(" 1 thread   ", Style::default().fg(Color::DarkGray)),
            Span::styled(bar(st, sat, bar_w), Style::default().fg(Color::DarkGray)),
            Span::styled(format!("  {:>6.1} GB/s", st), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(second),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_headline(f: &mut Frame, app: &App, fp: &Fingerprint, area: Rect) {
    let block = panel(" Verdict ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();

    // Violations come first: they invalidate everything below them.
    if !app.ws.violations.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" ⚠ {} validation violation(s) — this fingerprint is NOT trusted:", app.ws.violations.len()),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        // Every other width computation in this file uses saturating_sub; this
        // one did not, and panicked the dashboard at 10-11 columns (Critic N2).
        let avail = (inner.width as usize).saturating_sub(10).max(8);
        for v in app.ws.violations.iter().take(3) {
            lines.push(Line::from(Span::styled(
                format!("   {}  {}", v.code(), truncate(&v.to_string(), avail)),
                Style::default().fg(Color::Red),
            )));
        }
        if app.ws.violations.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("   … and {} more", app.ws.violations.len() - 3),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(""));
    }

    if fp.memory_pressured() {
        lines.push(Line::from(Span::styled(
            "⚠  This machine is swapping heavily. Bandwidth figures are unreliable,",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            "   and any plan generated now should be treated as provisional.",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    // The answer, before the evidence for it. Everything above this line is a
    // caveat that invalidates it; everything below is how we know.
    if let Some(best) = app.spot.as_ref().and_then(|s| s.best_usable(DEFAULT_USABLE_TPS)) {
        lines.push(Line::from(vec![
            Span::styled(" Use        ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                truncate(&best.plan.display, (inner.width as usize).saturating_sub(28).max(8)),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {:.0} tok/s", best.tps()), Style::default().fg(Color::Green)),
            Span::styled(
                format!("  {:.1} GiB", best.plan.peak_bytes as f64 / (1u64 << 30) as f64),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            truncate(
                &format!(
                    "            the most capable plan above {:.0} tok/s; tab 4 shows the trade",
                    DEFAULT_USABLE_TPS
                ),
                inner.width as usize,
            ),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
    }

    match app.ws.results.as_ref() {
        Some(r) => {
            lines.push(Line::from(vec![
                Span::styled(" Predictor  ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    "t₀ {:.2} ms · k_cal {:.3} · R² {:.4}",
                    r.fit.t0_ms, r.fit.eps, r.fit.r2
                )),
            ]));
            // Accuracy is only a claim if the machine beneath it is trusted.
            let accuracy_colour = if app.ws.is_trusted() { Color::Green } else { Color::DarkGray };
            lines.push(Line::from(vec![
                Span::styled(" Accuracy   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("MAPE {:.2}%", r.mape.two_param_pct),
                    Style::default().fg(accuracy_colour).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("   (naive roofline {:.1}%)", r.mape.naive_pct),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" Evidence   ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{} dense models, single stream, llama.cpp Metal", r.rows.len())),
            ]));
        }
        None => {
            lines.push(Line::from(Span::styled(
                "No benchmark results yet — run `python3 spikes/m0/spike.py`.",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Press 2 for per-model measurements.",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// --- Bench -----------------------------------------------------------------

fn render_bench(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(r) = app.ws.results.as_ref() else {
        render_empty(f, app, area);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(4)])
        .split(area);

    // Shed the corroborating columns before crushing the important ones.
    let narrow = area.width < 84;

    let mut header_cells = vec![
        Cell::from(" model"),
        Cell::from("GiB").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("measured"),
        Cell::from("predicted"),
        Cell::from("Δ"),
        Cell::from("naive"),
        Cell::from("Δ naive"),
        Cell::from("k_cal"),
    ];
    if narrow {
        header_cells.truncate(5);
    }
    let header = Row::new(header_cells)
        .style(Style::default().fg(Color::DarkGray))
        .height(1);
    // Give the model name a sensible share of the width instead of letting it
    // eat the row on a wide terminal.
    let model_w = if narrow {
        (area.width as usize).saturating_sub(38).clamp(20, 34)
    } else {
        (area.width as usize).saturating_sub(62).clamp(24, 46)
    } as u16;

    let mut widths = vec![
        Constraint::Length(model_w),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(7),
    ];
    if narrow {
        widths.truncate(5);
    }

    let rows: Vec<Row> = r
        .rows
        .iter()
        .map(|row| {
            let d = row.two_param_err_pct;
            let colour = if d.abs() <= 5.0 {
                Color::Green
            } else if d.abs() <= 15.0 {
                Color::Yellow
            } else {
                Color::Red
            };
            let nd = row.naive_err_pct;
            let ncolour = if nd.abs() <= 5.0 {
                Color::Green
            } else if nd.abs() <= 15.0 {
                Color::Yellow
            } else {
                Color::Red
            };
            let mut cells = vec![
                Cell::from(short_model(&row.model)),
                Cell::from(format!("{:.2}", row.gib)),
                Cell::from(format!("{:.1}", row.measured_tps)),
                Cell::from(format!("{:.1}", row.two_param_tps)),
                Cell::from(format!("{:+.1}%", d)).style(Style::default().fg(colour)),
                Cell::from(format!("{:.1}", row.naive_tps)),
                Cell::from(format!("{:+.1}%", nd)).style(Style::default().fg(ncolour)),
                Cell::from(format!("{:.2}", row.implied_eps)),
            ];
            if narrow {
                cells.truncate(5);
            }
            Row::new(cells)
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(panel(" Decode throughput — predicted vs measured "))
        .column_spacing(1)
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(table, chunks[0], &mut app.table);

    let fit = &r.fit;
    let summary = vec![
        Line::from(vec![
            Span::styled(" t_token = t₀ + bytes / (k_cal × BW) ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!(
                "  t₀ {:.2} ms · k_cal {:.3} · R² {:.4} · n={}",
                fit.t0_ms, fit.eps, fit.r2, fit.n_models
            )),
        ]),
        Line::from(vec![
            Span::styled(" MAPE ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.2}%", r.mape.two_param_pct),
                Style::default()
                    .fg(if app.ws.is_trusted() { Color::Green } else { Color::DarkGray })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("   naive roofline {:.1}%   ·   {} reps   ·   build {}", r.mape.naive_pct, r.reps, r.rows.first().map(|x| x.build.as_str()).unwrap_or("—")),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];
    let block = panel(" Calibration ");
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);
    f.render_widget(Paragraph::new(summary).wrap(Wrap { trim: false }), inner);
}

// --- Plans -----------------------------------------------------------------

fn render_plans(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(set) = app.plans.as_ref() else {
        let block = panel(" Plans ");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = app.plan_error.clone().unwrap_or_else(|| "no plan".into());
        let lines = vec![
            Line::from(Span::styled("  Cannot plan yet.", Style::default().fg(Color::Yellow))),
            Line::from(""),
            Line::from(format!("  {msg}")),
            Line::from(""),
            Line::from(Span::styled(
                "  Generate a fingerprint and a benchmark first:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "    python3 spikes/m0/fingerprint.py && python3 spikes/m0/spike.py",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(9)])
        .split(area);

    // Same derivation `compute_plans` used, so the header can never claim a
    // different architecture than the one the plans were ranked against.
    let arch = app
        .ws
        .fingerprint
        .as_ref()
        .map(|f| {
            f.cpu
                .arch_id
                .clone()
                .unwrap_or_else(|| qinfer_core::collect::slugify(&f.cpu.brand))
        })
        .unwrap_or_else(|| "unknown".into());

    let header = Row::new(vec![
        Cell::from(" #"),
        Cell::from("model"),
        Cell::from("tok/s"),
        Cell::from("p10–p90"),
        Cell::from("peak"),
    ])
    .style(Style::default().fg(Color::DarkGray));

    let rows: Vec<Row> = set
        .plans
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let moe = p.risks.iter().any(|r| r.starts_with("MoE"));
            Row::new(vec![
                Cell::from(format!(" {}", i + 1)),
                Cell::from(truncate(&p.display, 30)),
                Cell::from(format!("{:.0}", p.predicted.p50))
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Cell::from(format!("{:.0}–{:.0}", p.predicted.p10, p.predicted.p90)),
                Cell::from(format!("{:.1} GiB", p.peak_bytes as f64 / (1u64 << 30) as f64)),
            ])
            .style(if moe { Style::default().fg(Color::White) } else { Style::default() })
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(24),
        Constraint::Length(7),
        Constraint::Length(11),
        Constraint::Length(10),
    ];

    // The title owns its String; `panel` borrows it, so it must outlive the widget.
    let title = format!(
        " Plans · {arch} · ctx {} · {} excluded ",
        set.ctx,
        set.rejected.len()
    );
    let table = Table::new(rows, widths)
        .header(header)
        .block(panel(&title))
        .column_spacing(1)
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(table, chunks[0], &mut app.plan_table);

    // Detail for the highlighted plan.
    let sel = app.plan_table.selected().unwrap_or(0).min(set.plans.len().saturating_sub(1));
    let block = panel(" Why this plan ");
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    if let Some(p) = set.plans.get(sel) {
        // `saturating_sub`, not `-`: below ten columns this was a usize
        // underflow, which panics in debug and silently wraps to a huge width
        // in release. A terminal can legitimately be that narrow — a tmux pane
        // split four ways is narrower than anyone expects.
        let room = (inner.width as usize).saturating_sub(8).max(1);
        let mut lines: Vec<Line> = p
            .rationale
            .iter()
            .take(3)
            .map(|r| {
                Line::from(vec![
                    Span::styled(" why   ", Style::default().fg(Color::DarkGray)),
                    Span::raw(truncate(r, room)),
                ])
            })
            .collect();
        for r in p.risks.iter().take(2) {
            lines.push(Line::from(vec![
                Span::styled(" risk  ", Style::default().fg(Color::Yellow)),
                Span::raw(truncate(r, room)),
            ]));
        }
        if !p.fallbacks.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(" next  ", Style::default().fg(Color::DarkGray)),
                Span::raw(truncate(&p.fallbacks.join(", "), room)),
            ]));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
}

// --- Help ------------------------------------------------------------------

fn render_help(f: &mut Frame, area: Rect) {
    let block = panel(" Keys and meaning ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(vec![Span::styled("  tab / ←→   ", Style::default().fg(Color::Cyan)), Span::raw("switch view")]),
        Line::from(vec![Span::styled("  1 2 3 4 ?  ", Style::default().fg(Color::Cyan)), Span::raw("jump to Overview / Bench / Frontier / Plans / Help")]),
        Line::from(vec![Span::styled("  ↑ ↓ j k    ", Style::default().fg(Color::Cyan)), Span::raw("move selection in a table")]),
        Line::from(vec![Span::styled("  r          ", Style::default().fg(Color::Cyan)), Span::raw("reload fingerprint and results from disk")]),
        Line::from(vec![Span::styled("  q / esc    ", Style::default().fg(Color::Cyan)), Span::raw("quit — never bound to anything destructive")]),
        Line::from(""),
        Line::from(Span::styled(
            "  Frontier ranks by capability, not throughput. Past ~20 tok/s extra",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  speed buys nothing, so the tab recommends the most capable plan that",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  is still fast enough, and says what it traded away.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  This dashboard reads the same files the CLI writes. It is a view,",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  never the only way to obtain a fact (ADR-0009).",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Headless rendering:  qinfer --snapshot 100x30 [--tab bench]",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

// --- shared ----------------------------------------------------------------

fn render_empty(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(" No data ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled("  No capability fingerprint found.", Style::default().fg(Color::Yellow))),
        Line::from(""),
        Line::from(format!("  looked in: {}", app.ws.root.display())),
        Line::from(""),
        Line::from(Span::styled("  Generate one with:", Style::default().fg(Color::DarkGray))),
        Line::from("    python3 spikes/m0/fingerprint.py"),
        Line::from("    python3 spikes/m0/spike.py"),
        Line::from(""),
        Line::from(Span::styled(
            "  Then press r.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(title.to_string(), Style::default().fg(Color::Gray)))
}

fn kv(key: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<13}", key), Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}

fn pretty_os(os: &str) -> &str {
    match os {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    }
}

/// `spdisplays_metal4` is a plist constant, not something a human should read.
fn pretty_backend(raw: Option<&str>) -> String {
    match raw {
        None => String::new(),
        Some(s) => {
            let cleaned = s.trim_start_matches("spdisplays_").replace("metal", "Metal ");
            cleaned.trim().to_string()
        }
    }
}

/// A fixed-width bar. Deliberately text, not a Gauge, so it survives narrow
/// terminals and renders identically in a snapshot.
fn bar(value: f64, max: f64, width: usize) -> String {
    let ratio = if max > 0.0 { (value / max).clamp(0.0, 1.0) } else { 0.0 };
    let filled = (ratio * width as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width.saturating_sub(filled)))
}

/// Truncate to `max` display columns, appending an ellipsis when cut. Counts
/// chars, not bytes, so it cannot split a multi-byte glyph in half.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Model filenames are long and their distinguishing token is usually the
/// *size*, not the publisher boilerplate. Drop the noise, keep the identity.
fn short_model(name: &str) -> String {
    let trimmed = name.trim_end_matches(".gguf");
    let kept: Vec<&str> = trimmed
        .split('-')
        .filter(|p| !matches!(*p, "instruct" | "it" | "chat" | "gguf"))
        .collect();
    kept.join("-")
}

/// `2026-10-25T21:25:10Z` -> `2026-10-25 21:25`
fn short_time(ts: &str) -> String {
    if ts.len() >= 16 {
        ts[..16].replace('T', " ")
    } else {
        ts.to_string()
    }
}

/// Keep the status bar readable: the last two path components are enough to
/// identify a state directory, and the full path is available via `--json`.
fn short_path(p: &std::path::Path) -> String {
    let parts: Vec<String> = p
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}


