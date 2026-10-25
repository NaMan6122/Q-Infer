//! `qinfer` — the terminal surface.
//!
//! ADR-0009: this is a *view* over the core's state, not a second product.
//! It reads `cfp@1` and the spike results from disk and renders them. It opens
//! no socket, spawns no daemon, and holds no privilege.
//!
//! `--snapshot WxH` renders one frame to an offscreen buffer and prints it.
//! That exists so the layout can be reviewed in CI and over a pipe, and so
//! "does the dashboard look right" never requires a human at a TTY.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use qinfer_core::{Fingerprint, Workspace};
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs, Wrap},
    Frame, Terminal,
};

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Bench,
    Plans,
    Help,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Overview, Tab::Bench, Tab::Plans, Tab::Help];

    fn index(self) -> usize {
        match self {
            Tab::Overview => 0,
            Tab::Bench => 1,
            Tab::Plans => 2,
            Tab::Help => 3,
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
    status: String,
    refreshed: Instant,
}

impl App {
    fn new(ws: Workspace) -> Self {
        let status = if ws.has_data() {
            format!("loaded {}", short_path(&ws.root))
        } else {
            ws.error.clone().unwrap_or_else(|| "no data".into())
        };
        App { ws, tab: Tab::Overview, table: TableState::default(), status, refreshed: Instant::now() }
    }

    fn reload(&mut self) {
        self.ws.reload();
        self.status = if self.ws.has_data() {
            format!("reloaded {} at {}", short_path(&self.ws.root), clock())
        } else {
            self.ws.error.clone().unwrap_or_else(|| "no data".into())
        };
        self.refreshed = Instant::now();
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

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

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
        return snapshot(w, h, tab);
    }

    run_tui()
}

fn parse_dims(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn snapshot(width: u16, height: u16, tab: &str) -> io::Result<()> {
    let mut app = App::new(Workspace::discover());
    app.tab = match tab {
        "bench" => Tab::Bench,
        "plans" => Tab::Plans,
        "help" => Tab::Help,
        _ => Tab::Overview,
    };

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
                    KeyCode::Char('2') => app.tab = Tab::Bench,
                    KeyCode::Char('3') => app.tab = Tab::Plans,
                    KeyCode::Char('?') => app.tab = Tab::Help,
                    KeyCode::Char('r') => app.reload(),
                    KeyCode::Down | KeyCode::Char('j') => app.move_row(1),
                    KeyCode::Up | KeyCode::Char('k') => app.move_row(-1),
                    _ => {}
                }
            }
        }

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
            Constraint::Length(4), // title row + tabs row
            Constraint::Min(10),   // body
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);

    match app.tab {
        Tab::Overview => render_overview(f, app, chunks[1]),
        Tab::Bench => render_bench(f, app, chunks[1]),
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

    let title = Line::from(vec![
        Span::styled(" Q-Infer ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  ·  "),
        Span::styled(tier, Style::default().fg(tier_colour)),
    ]);

    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    f.render_widget(Paragraph::new(title), rows[0]);

    let tabs = Tabs::new(vec!["Overview", "Bench", "Plans", "Help"])
        .select(app.tab.index())
        .divider(Span::raw(" │ "))
        .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(tabs, rows[1]);
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let hint = match app.tab {
        Tab::Bench => "  ↑↓  ·  r  ·  q",
        _ => "  tab  ·  ?  ·  q",
    };
    let line = Line::from(vec![
        Span::styled(format!(" {} ", app.status), Style::default().fg(Color::DarkGray)),
        Span::styled(hint, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
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
        render_bandwidth(f, fp, rows[1]);
        render_headline(f, app, fp, rows[2]);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Length(5), Constraint::Min(5)])
            .split(area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);
        render_machine(f, fp, cols[0]);
        render_capabilities(f, fp, cols[1]);
        render_bandwidth(f, fp, rows[1]);
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

    let mem = format!(
        "{:.1} GiB   swap {:.1} GiB{}",
        fp.total_gib(),
        fp.swap_gib(),
        if fp.memory_pressured() { "  ⚠ under pressure" } else { "" }
    );

    // Panel width is not ours to assume; every value is fitted explicitly so
    // it can never bleed into the border (ADR-0009).
    let avail = (inner.width as usize).saturating_sub(15).max(6);
    let lines = vec![
        kv("CPU", &truncate(&format!("{}  ({}P/{}L)", fp.cpu.brand, fp.cpu.physical_cores, fp.cpu.logical_cores), avail)),
        kv("Clusters", &truncate(&fp.perf_summary(), avail)),
        kv("GPU", &truncate(gpu_line.trim_end(), avail)),
        kv("Memory", &truncate(&mem, avail)),
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

fn render_bandwidth(f: &mut Frame, fp: &Fingerprint, area: Rect) {
    let block = panel(" Memory bandwidth (measured, not declared) ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sat = fp.saturated_gbs();
    let st = fp.single_thread_gbs();
    let gain = fp.thread_gain().unwrap_or(1.0);
    let bar_w = (inner.width as usize).saturating_sub(34).clamp(6, 40);
    let show_gain = inner.width >= 84;

    let mut second = vec![
        Span::styled(
            format!(" {:<2} threads  ", fp.bandwidth.best_threads.unwrap_or(0)),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(bar(sat, sat, bar_w), Style::default().fg(Color::Cyan)),
        Span::styled(
            format!("  {:>6.1} GB/s", sat),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
    ];
    if show_gain {
        second.push(Span::styled(
            format!("   ({:.2}× single-thread)", gain),
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
        for v in app.ws.violations.iter().take(3) {
            lines.push(Line::from(Span::styled(
                format!("   {}  {}", v.code(), truncate(&v.to_string(), inner.width as usize - 10)),
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

    match app.ws.results.as_ref() {
        Some(r) => {
            lines.push(Line::from(vec![
                Span::styled(" Predictor  ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    "t₀ {:.2} ms · k_cal {:.3} · R² {:.4}",
                    r.fit.t0_ms, r.fit.eps, r.fit.r2
                )),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" Accuracy   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("MAPE {:.2}%", r.mape.two_param_pct),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
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
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
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

fn render_plans(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(" Plans ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![
        Line::from(Span::styled(
            "  The planner lands in M0 (docs/milestones/m0-advisor.md).",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled("  When it exists, this view will show, per plan:", Style::default())),
        Line::from(""),
        Line::from("    · the selected engine, artifact and quantisation"),
        Line::from("    · predicted TTFT / decode with p10–p90 intervals"),
        Line::from("    · why  — the rationale, generated from the fingerprint"),
        Line::from("    · risks — known runtime gaps for this exact machine"),
        Line::from("    · fallbacks, pre-computed for when verification fails"),
        Line::from(""),
        Line::from(Span::styled(
            "  No plans on disk.",
            Style::default().fg(Color::Yellow),
        )),
    ];
    if let Some(err) = &app.ws.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("  {err}"), Style::default().fg(Color::DarkGray))));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// --- Help ------------------------------------------------------------------

fn render_help(f: &mut Frame, area: Rect) {
    let block = panel(" Keys and meaning ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(vec![Span::styled("  tab / ←→   ", Style::default().fg(Color::Cyan)), Span::raw("switch view")]),
        Line::from(vec![Span::styled("  1 2 3 ?    ", Style::default().fg(Color::Cyan)), Span::raw("jump to Overview / Bench / Plans / Help")]),
        Line::from(vec![Span::styled("  ↑ ↓ j k    ", Style::default().fg(Color::Cyan)), Span::raw("move selection in a table")]),
        Line::from(vec![Span::styled("  r          ", Style::default().fg(Color::Cyan)), Span::raw("reload fingerprint and results from disk")]),
        Line::from(vec![Span::styled("  q / esc    ", Style::default().fg(Color::Cyan)), Span::raw("quit — never bound to anything destructive")]),
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


