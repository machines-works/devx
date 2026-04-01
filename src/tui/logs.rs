use std::collections::VecDeque;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub struct LogEntry {
    pub service: String,
    pub line: String,
    pub is_stderr: bool,
    pub color: Color,
}

pub fn service_color(index: usize) -> Color {
    const COLORS: &[Color] = &[
        Color::Cyan,
        Color::Magenta,
        Color::Yellow,
        Color::Green,
        Color::Blue,
        Color::Red,
    ];
    COLORS[index % COLORS.len()]
}

pub fn render_logs(
    frame: &mut Frame,
    area: Rect,
    entries: &VecDeque<LogEntry>,
    scroll_offset: usize,
    filter: Option<&str>,
) {
    let visible_height = area.height.saturating_sub(2) as usize;

    // Count matching entries to compute scroll offset without collecting into a Vec.
    let total = match filter {
        Some(f) => entries.iter().filter(|e| e.service == f).count(),
        None => entries.len(),
    };

    let skip = if scroll_offset == 0 {
        total.saturating_sub(visible_height)
    } else {
        total
            .saturating_sub(visible_height)
            .saturating_sub(scroll_offset)
    };

    let iter: Box<dyn Iterator<Item = &LogEntry>> = match filter {
        Some(f) => Box::new(entries.iter().filter(move |e| e.service == f)),
        None => Box::new(entries.iter()),
    };

    let visible_lines: Vec<Line> = iter
        .skip(skip)
        .take(visible_height)
        .map(|entry| {
            let label = if entry.service.chars().count() <= 12 {
                format!("{:<12}", &entry.service)
            } else {
                let truncated: String = entry.service.chars().take(11).collect();
                format!("{}\u{2026}", truncated)
            };
            let prefix = format!("[{}]", label);
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(entry.color)),
                Span::raw(format!(" {}", entry.line)),
            ])
        })
        .collect();

    let title = if let Some(f) = filter {
        format!(" Logs [{}] ", f)
    } else {
        " Logs ".to_string()
    };

    let paragraph = Paragraph::new(visible_lines)
        .block(Block::default().borders(Borders::TOP).title(title));

    frame.render_widget(paragraph, area);
}
