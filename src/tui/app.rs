use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{self, Event};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;

use crate::config::ConfigDiff;
use crate::events::{DevxEvent, ServiceState};
use crate::orchestrator::OrchestratorCommand;
use crate::tui::keys::{Action, handle_key};
use crate::tui::logs::{LogEntry, service_color};
use crate::tui::status::{ServiceInfo, render_status};

pub struct App {
    project_name: String,
    branch: Option<String>,
    service_names: Vec<String>,
    service_states: HashMap<String, ServiceState>,
    service_ports: HashMap<String, u16>,
    proxy_ports: HashMap<String, u16>,
    started_at: HashMap<String, Instant>,
    log_entries: VecDeque<LogEntry>,
    service_colors: HashMap<String, Color>,
    vhost_port: Option<u16>,
    vhost_tls: bool,
    service_domains: HashMap<String, String>,
    selected_service: usize,
    scroll_offset: usize,
    filter: Option<String>,
    should_quit: bool,
    cmd_tx: mpsc::Sender<OrchestratorCommand>,
    event_rx: mpsc::Receiver<DevxEvent>,
}

impl App {
    pub fn new(
        project_name: String,
        service_names: Vec<String>,
        branch: Option<String>,
        cmd_tx: mpsc::Sender<OrchestratorCommand>,
        event_rx: mpsc::Receiver<DevxEvent>,
    ) -> Self {
        let mut service_colors = HashMap::new();
        for (i, name) in service_names.iter().enumerate() {
            service_colors.insert(name.clone(), service_color(i));
        }

        // Filter out main/master — no branch badge needed
        let branch = branch.filter(|b| b != "main" && b != "master" && b != "HEAD");

        Self {
            project_name,
            branch,
            service_names,
            service_states: HashMap::new(),
            service_ports: HashMap::new(),
            proxy_ports: HashMap::new(),
            started_at: HashMap::new(),
            log_entries: VecDeque::new(),
            service_colors,
            vhost_port: None,
            vhost_tls: false,
            service_domains: HashMap::new(),
            selected_service: 0,
            scroll_offset: 0,
            filter: None,
            should_quit: false,
            cmd_tx,
            event_rx,
        }
    }

    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            // Drain all pending DevxEvents
            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_devx_event(event);
            }

            if self.log_entries.len() > 6000 {
                self.log_entries.drain(0..2000);
            }

            // Draw frame
            terminal.draw(|frame| self.render(frame))?;

            // Poll crossterm events with 50ms timeout
            if event::poll(std::time::Duration::from_millis(50))?
                && let Event::Key(key) = event::read()?
            {
                match handle_key(key) {
                    Action::Quit => {
                        let _ = self.cmd_tx.try_send(OrchestratorCommand::Shutdown);
                        self.should_quit = true;
                        return Ok(());
                    }
                    Action::Restart => {
                        let name = self
                            .service_names
                            .get(self.selected_service)
                            .cloned()
                            .unwrap_or_default();
                        if !name.is_empty() {
                            let _ = self
                                .cmd_tx
                                .try_send(OrchestratorCommand::Restart { service: name });
                        }
                    }
                    Action::ToggleFilter => {
                        let name = self
                            .service_names
                            .get(self.selected_service)
                            .cloned()
                            .unwrap_or_default();
                        if !name.is_empty() {
                            if self.filter.as_deref() == Some(&name) {
                                // Already filtering this service — toggle off
                                self.filter = None;
                            } else {
                                self.filter = Some(name);
                            }
                            self.scroll_offset = 0;
                        }
                    }
                    Action::ClearFilter => {
                        self.filter = None;
                        self.scroll_offset = 0;
                    }
                    Action::ScrollUp => {
                        self.scroll_offset = self.scroll_offset.saturating_add(1);
                    }
                    Action::ScrollDown => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    }
                    Action::PrevService => {
                        if !self.service_names.is_empty() {
                            self.selected_service = if self.selected_service == 0 {
                                self.service_names.len() - 1
                            } else {
                                self.selected_service - 1
                            };
                        }
                    }
                    Action::NextService => {
                        if !self.service_names.is_empty() {
                            self.selected_service =
                                (self.selected_service + 1) % self.service_names.len();
                        }
                    }
                    Action::None => {}
                }
            }

            if self.should_quit {
                return Ok(());
            }
        }
    }

    fn handle_devx_event(&mut self, event: DevxEvent) {
        match event {
            DevxEvent::LogLine {
                service,
                line,
                is_stderr,
            } => {
                let color = self
                    .service_colors
                    .get(&service)
                    .copied()
                    .unwrap_or(Color::White);
                self.log_entries.push_back(LogEntry {
                    service,
                    line,
                    is_stderr,
                    color,
                });
            }
            DevxEvent::StateChange {
                service,
                state,
                started_at,
            } => {
                self.service_states.insert(service.clone(), state);
                if let Some(t) = started_at {
                    self.started_at.insert(service, t);
                }
            }
            DevxEvent::ProxyBound {
                service,
                proxy_port,
                target_port,
            } => {
                self.proxy_ports.insert(service.clone(), proxy_port);
                self.service_ports.insert(service, target_port);
            }
            DevxEvent::VhostBound { port, domains, tls } => {
                self.vhost_port = Some(port);
                self.vhost_tls = tls;
                for (domain, svc_name) in domains {
                    self.service_domains.insert(svc_name, domain);
                }
            }
            DevxEvent::AllStarted => {}
            DevxEvent::Tick => {}
            DevxEvent::FileChanged { service } => {
                let color = self
                    .service_colors
                    .get(&service)
                    .copied()
                    .unwrap_or(Color::White);
                self.log_entries.push_back(LogEntry {
                    service: service.clone(),
                    line: "file changed, restarting...".to_string(),
                    is_stderr: false,
                    color,
                });
                let _ = self
                    .cmd_tx
                    .try_send(OrchestratorCommand::Restart { service });
            }
            DevxEvent::ConfigReloaded { diff } => {
                self.handle_config_reloaded(diff);
            }
            DevxEvent::ControlShutdown => {
                let _ = self.cmd_tx.try_send(OrchestratorCommand::Shutdown);
                self.should_quit = true;
            }
            DevxEvent::ControlRestart { service } => {
                let _ = self
                    .cmd_tx
                    .try_send(OrchestratorCommand::Restart { service });
            }
        }
    }

    fn handle_config_reloaded(&mut self, diff: ConfigDiff) {
        let mut parts = Vec::new();
        for name in &diff.changed {
            parts.push(format!("restarted {}", name));
        }
        for name in &diff.added {
            parts.push(format!("added {}", name));
            self.service_colors
                .insert(name.clone(), service_color(self.service_names.len()));
            self.service_names.push(name.clone());
        }
        for name in &diff.removed {
            parts.push(format!("removed {}", name));
            self.service_names.retain(|n| n != name);
            self.service_states.remove(name);
            self.service_ports.remove(name);
            self.proxy_ports.remove(name);
            self.started_at.remove(name);
            self.service_domains.remove(name);
        }

        let summary = if parts.is_empty() {
            "no changes".to_string()
        } else {
            parts.join(", ")
        };

        self.log_entries.push_back(LogEntry {
            service: "devx".to_string(),
            line: format!("[devx] config reloaded: {}", summary),
            is_stderr: false,
            color: Color::Cyan,
        });
    }

    fn render(&self, frame: &mut Frame) {
        let service_count = self.service_names.len() as u16;
        let areas: [ratatui::layout::Rect; 4] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(service_count + 2),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        // Title bar
        let branch_badge = self
            .branch
            .as_deref()
            .map(|b| format!(" [{}]", b))
            .unwrap_or_default();
        let title = if let Some(port) = self.vhost_port {
            let proto = if self.vhost_tls { "https" } else { "http" };
            format!(
                " devx · {}{} · {}://*.localhost:{}",
                self.project_name, branch_badge, proto, port
            )
        } else {
            format!(" devx · {}{}", self.project_name, branch_badge)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(title, Style::default().fg(Color::Cyan))),
            areas[0],
        );

        // Status table
        let services: Vec<ServiceInfo> = self
            .service_names
            .iter()
            .map(|name| {
                let state = self
                    .service_states
                    .get(name)
                    .cloned()
                    .unwrap_or(ServiceState::Stopped);
                let uptime = self.started_at.get(name).map(|t| {
                    let secs = t.elapsed().as_secs();
                    if secs < 60 {
                        format!("{}s", secs)
                    } else if secs < 3600 {
                        format!("{}m{}s", secs / 60, secs % 60)
                    } else {
                        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
                    }
                });
                ServiceInfo {
                    name: name.clone(),
                    domain: self.service_domains.get(name).cloned(),
                    state,
                    actual_port: self.service_ports.get(name).copied(),
                    proxy_port: self.proxy_ports.get(name).copied(),
                    uptime,
                }
            })
            .collect();

        render_status(frame, areas[1], &services, self.selected_service);

        // Logs
        crate::tui::logs::render_logs(
            frame,
            areas[2],
            &self.log_entries,
            self.scroll_offset,
            self.filter.as_deref(),
        );

        // Help bar
        let help = " q quit  r restart  ↑↓ select  enter/f filter  esc all  shift+↑↓ scroll";
        frame.render_widget(
            Paragraph::new(Span::styled(help, Style::default().fg(Color::DarkGray))),
            areas[3],
        );
    }
}
