use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use crate::events::ServiceState;

pub struct ServiceInfo {
    pub name: String,
    pub domain: Option<String>,
    pub state: ServiceState,
    pub actual_port: Option<u16>,
    pub proxy_port: Option<u16>,
    pub uptime: Option<String>,
}

fn state_color(state: &ServiceState) -> Color {
    match state {
        ServiceState::Healthy => Color::Green,
        ServiceState::Starting | ServiceState::Pending => Color::Yellow,
        ServiceState::Failed(_) | ServiceState::Unhealthy => Color::Red,
        ServiceState::Stopped => Color::DarkGray,
    }
}

pub fn render_status(
    frame: &mut Frame,
    area: Rect,
    services: &[ServiceInfo],
    selected: usize,
) {
    let header_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    let header = Row::new(vec![
        Cell::from("SERVICE").style(header_style),
        Cell::from("DOMAIN").style(header_style),
        Cell::from("STATUS").style(header_style),
        Cell::from("PORT").style(header_style),
        Cell::from("PROXY").style(header_style),
        Cell::from("TIME").style(header_style),
    ]);

    let rows: Vec<Row> = services
        .iter()
        .enumerate()
        .map(|(i, svc)| {
            let color = state_color(&svc.state);
            let status_text = format!("{} {}", svc.state.symbol(), svc.state.label());
            let domain_text = svc.domain.clone().unwrap_or_default();
            let port_text = svc
                .actual_port
                .map(|p| p.to_string())
                .unwrap_or_default();
            let proxy_text = svc
                .proxy_port
                .map(|p| p.to_string())
                .unwrap_or_default();
            let uptime_text = svc.uptime.clone().unwrap_or_default();

            let row_style = if i == selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(svc.name.clone()),
                Cell::from(domain_text),
                Cell::from(status_text).style(Style::default().fg(color)),
                Cell::from(port_text),
                Cell::from(proxy_text),
                Cell::from(uptime_text),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        ratatui::layout::Constraint::Length(16),
        ratatui::layout::Constraint::Length(22),
        ratatui::layout::Constraint::Length(14),
        ratatui::layout::Constraint::Length(8),
        ratatui::layout::Constraint::Length(8),
        ratatui::layout::Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::NONE));

    frame.render_widget(table, area);
}
