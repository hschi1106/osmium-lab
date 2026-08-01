use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode},
};
use market_types::{BookLevel, MatchTime, Price};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, Paragraph, Row, Table},
};

use crate::market_replay::{MarketReplay, MarketReplayError, PlaybackStatus};

pub fn run(config: &Path) -> Result<(), MarketReplayError> {
    let mut replay = MarketReplay::from_config(config)?;
    enable_raw_mode().map_err(MarketReplayError::Io)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(MarketReplayError::Io)?;
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(MarketReplayError::Io)?;

    loop {
        let now = Instant::now();
        replay.tick(now)?;
        terminal
            .draw(|frame| draw(frame, &replay))
            .map_err(MarketReplayError::Io)?;
        if event::poll(Duration::from_millis(33)).map_err(MarketReplayError::Io)?
            && let Event::Key(key) = event::read().map_err(MarketReplayError::Io)?
            && handle_key(key, &mut replay, Instant::now())?
        {
            break;
        }
    }
    terminal.show_cursor().map_err(MarketReplayError::Io)?;
    Ok(())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}

fn handle_key(
    key: KeyEvent,
    replay: &mut MarketReplay,
    now: Instant,
) -> Result<bool, MarketReplayError> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => Ok(true),
        KeyCode::Left => {
            replay.select_previous();
            Ok(false)
        }
        KeyCode::Right => {
            replay.select_next();
            Ok(false)
        }
        KeyCode::Char(' ') => {
            replay.toggle_at(now);
            Ok(false)
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            replay.faster();
            Ok(false)
        }
        KeyCode::Char('-') => {
            replay.slower();
            Ok(false)
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            replay.reset(now)?;
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn draw(frame: &mut Frame<'_>, replay: &MarketReplay) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(16),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, vertical[0], replay);
    draw_progress(frame, vertical[1], replay);

    let charts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(8)])
        .split(vertical[2]);
    draw_price_chart(frame, charts[0], replay);
    draw_volume_chart(frame, charts[1], replay);

    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vertical[3]);
    draw_order_book(frame, panels[0], replay);
    draw_trades(frame, panels[1], replay);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" ←/→ ", Style::default().fg(Color::Yellow)),
        Span::raw("Previous/Next symbol   "),
        Span::styled("Space", Style::default().fg(Color::Yellow)),
        Span::raw(" Pause/Play   "),
        Span::styled("+/-", Style::default().fg(Color::Yellow)),
        Span::raw(" Faster/Slower   "),
        Span::styled("R", Style::default().fg(Color::Yellow)),
        Span::raw(" Reset 1x   "),
        Span::styled("Q", Style::default().fg(Color::Yellow)),
        Span::raw(" Quit"),
    ]));
    frame.render_widget(footer, vertical[4]);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, replay: &MarketReplay) {
    let instrument = replay.selected_instrument();
    let status = match replay.status() {
        PlaybackStatus::Playing => ("▶ PLAYING", Color::Green),
        PlaybackStatus::Paused => ("Ⅱ PAUSED", Color::Yellow),
        PlaybackStatus::Finished => ("■ FINISHED", Color::Cyan),
    };
    let line = Line::from(vec![
        Span::styled(
            format!(
                "[{}/{}] {:?}/{}  {}  ",
                replay.selected_index() + 1,
                replay.instruments().len(),
                instrument.market(),
                instrument.symbol(),
                format_datetime(replay.current_time()),
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(status.0, Style::default().fg(status.1)),
        Span::raw(format!("    Speed {}", replay.speed().label())),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .borders(Borders::ALL)
                .title("osmium-lab display"),
        ),
        area,
    );
}

fn draw_progress(frame: &mut Frame<'_>, area: Rect, replay: &MarketReplay) {
    let start = replay.replay_start();
    let end = replay.replay_end();
    let total = end
        .as_unix_microseconds()
        .saturating_sub(start.as_unix_microseconds())
        .max(1) as f64;
    let elapsed = replay
        .current_time()
        .as_unix_microseconds()
        .saturating_sub(start.as_unix_microseconds())
        .max(0) as f64;
    let ratio = (elapsed / total).clamp(0.0, 1.0);
    let label = format!(
        "Replay: {} ─ {} ─ {}",
        format_time(start),
        format_time(replay.current_time()),
        format_time(end)
    );
    frame.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL))
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(label),
        area,
    );
}

fn draw_price_chart(frame: &mut Frame<'_>, area: Rect, replay: &MarketReplay) {
    let history = replay.selected_history();
    let points = history.price_points();
    let (minimum, maximum) = price_bounds(points);
    let x_max = replay_seconds(replay.replay_start(), replay.replay_end()).max(1.0);
    let title = format!(
        "PRICE  {}",
        history
            .latest_price()
            .map(format_price)
            .unwrap_or_else(|| "—".to_owned())
    );
    let dataset = Dataset::default()
        .name("PRICE")
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Green))
        .data(points);
    let chart = Chart::new(vec![dataset])
        .block(Block::default().borders(Borders::ALL).title(title))
        .x_axis(Axis::default().bounds([0.0, x_max]).labels(vec![
            Span::raw(format_time(replay.replay_start())),
            Span::raw(format_time(replay.current_time())),
            Span::raw(format_time(replay.replay_end())),
        ]))
        .y_axis(Axis::default().bounds([minimum, maximum]).labels(vec![
            Span::raw(format!("{minimum:.2}")),
            Span::raw(format!("{maximum:.2}")),
        ]));
    frame.render_widget(chart, area);
}

fn draw_volume_chart(frame: &mut Frame<'_>, area: Rect, replay: &MarketReplay) {
    let history = replay.selected_history();
    let points = history.volume_points();
    let maximum = history.maximum_volume().max(1) as f64;
    let x_max = replay_seconds(replay.replay_start(), replay.replay_end()).max(1.0);
    let dataset = Dataset::default()
        .name("VOLUME")
        .marker(symbols::Marker::HalfBlock)
        .graph_type(GraphType::Bar)
        .style(Style::default().fg(Color::Yellow))
        .data(points);
    let chart = Chart::new(vec![dataset])
        .block(Block::default().borders(Borders::ALL).title(format!(
            "VOLUME 1m  {} ─ {}",
            format_time(replay.replay_start()),
            format_time(replay.current_time())
        )))
        .x_axis(Axis::default().bounds([0.0, x_max]).labels(vec![
            Span::raw(format_time(replay.replay_start())),
            Span::raw(format_time(replay.replay_end())),
        ]))
        .y_axis(
            Axis::default()
                .bounds([0.0, maximum])
                .labels(vec![Span::raw("0"), Span::raw(format!("{maximum:.0}"))]),
        );
    frame.render_widget(chart, area);
}

fn draw_order_book(frame: &mut Frame<'_>, area: Rect, replay: &MarketReplay) {
    let instrument = replay.selected_instrument();
    let book = replay
        .selected_state()
        .and_then(|state| state.book().known());
    let mut rows = Vec::with_capacity(13);
    if let Some(book) = book {
        for (index, level) in book.asks().slots().iter().enumerate().rev() {
            rows.push(book_row(format!("Ask{}", index + 1), *level));
        }
    }
    rows.push(Row::new(vec!["────", "────────", "──────"]));
    if let Some(last) = replay.selected_history().trades().front() {
        rows.push(Row::new(vec![
            "Last".to_owned(),
            format_price(last.price()),
            last.quantity().value().to_string(),
        ]));
    } else {
        rows.push(Row::new(vec!["Last", "—", "—"]));
    }
    rows.push(Row::new(vec!["────", "────────", "──────"]));
    if let Some(book) = book {
        for (index, level) in book.bids().slots().iter().enumerate() {
            rows.push(book_row(format!("Bid{}", index + 1), *level));
        }
    }
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(vec!["LEVEL", "PRICE", "QTY"]).style(Style::default().fg(Color::Cyan)))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("ORDER BOOK — {}", instrument.symbol())),
    );
    frame.render_widget(table, area);
}

fn draw_trades(frame: &mut Frame<'_>, area: Rect, replay: &MarketReplay) {
    let instrument = replay.selected_instrument();
    let history = replay.selected_history();
    let rows = history
        .trades()
        .iter()
        .map(|trade| {
            Row::new(vec![
                format_time_of_day(trade.match_time()),
                format_price(trade.price()),
                trade.quantity().value().to_string(),
                "—".to_owned(),
            ])
        })
        .take(8)
        .collect::<Vec<_>>();
    let table = Table::new(
        rows,
        [
            Constraint::Length(15),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(vec!["TIME", "PRICE", "QTY", "SIDE"]).style(Style::default().fg(Color::Cyan)))
    .block(Block::default().borders(Borders::ALL).title(format!(
        "TRADES — {}  shown: {}",
        instrument.symbol(),
        history.trades().len()
    )));
    frame.render_widget(table, area);
}

fn book_row(label: String, level: Option<BookLevel>) -> Row<'static> {
    match level {
        Some(level) => Row::new(vec![
            label,
            format_price(level.price()),
            level.displayed_quantity().value().to_string(),
        ]),
        None => Row::new(vec![label, "—".to_owned(), "—".to_owned()]),
    }
}

fn price_bounds(points: &[(f64, f64)]) -> (f64, f64) {
    let Some((first, rest)) = points.split_first() else {
        return (0.0, 1.0);
    };
    let (mut minimum, mut maximum) = (first.1, first.1);
    for (_, value) in rest {
        minimum = minimum.min(*value);
        maximum = maximum.max(*value);
    }
    if (maximum - minimum).abs() < f64::EPSILON {
        let padding = (maximum.abs() * 0.001).max(1.0);
        (minimum - padding, maximum + padding)
    } else {
        let padding = (maximum - minimum) * 0.1;
        (minimum - padding, maximum + padding)
    }
}

fn replay_seconds(start: MatchTime, end: MatchTime) -> f64 {
    end.as_unix_microseconds()
        .saturating_sub(start.as_unix_microseconds()) as f64
        / 1_000_000.0
}

fn format_datetime(value: MatchTime) -> String {
    let formatted = value.to_iso8601(480);
    format!("{} {}", &formatted[0..10], &formatted[11..23])
}

fn format_time(value: MatchTime) -> String {
    value.to_iso8601(480)[11..23].to_owned()
}

fn format_time_of_day(value: MatchTime) -> String {
    format_time(value)
}

fn format_price(value: Price) -> String {
    format_decimal_atoms(value.atoms())
}

fn format_decimal_atoms(atoms: i128) -> String {
    let negative = atoms.is_negative();
    let absolute = atoms.unsigned_abs();
    let whole = absolute / market_types::Decimal::SCALE_FACTOR as u128;
    let fraction = absolute % market_types::Decimal::SCALE_FACTOR as u128;
    if fraction == 0 {
        return if negative {
            format!("-{whole}")
        } else {
            whole.to_string()
        };
    }
    let mut fractional = format!("{fraction:018}");
    while fractional.ends_with('0') {
        fractional.pop();
    }
    if negative {
        format!("-{whole}.{fractional}")
    } else {
        format!("{whole}.{fractional}")
    }
}
