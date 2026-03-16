use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Role};
use crate::gemini::InsightSeverity;

/// Main render function — draws the entire TUI.
pub fn render(frame: &mut Frame, app: &App) {
    // Three-tier vertical layout: header, body, input
    let [header, body, input_area] = Layout::vertical([
        Constraint::Length(1),   // header bar
        Constraint::Fill(1),     // main content
        Constraint::Length(3),   // input box
    ])
    .areas(frame.area());

    render_header(frame, header, app);

    if app.gemini_enabled {
        // With Gemini: body splits into upper (chat + cluster) and lower (insights)
        let [upper, insights_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(7), // Gemini insights panel
        ])
        .areas(body);

        let [chat_area, cluster_area] = Layout::horizontal([
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .areas(upper);

        render_chat(frame, chat_area, app);
        render_cluster(frame, cluster_area, app);
        render_gemini_insights(frame, insights_area, app);
    } else {
        // Without Gemini: original two-pane layout
        let [chat_area, cluster_area] = Layout::horizontal([
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .areas(body);

        render_chat(frame, chat_area, app);
        render_cluster(frame, cluster_area, app);
    }

    render_input(frame, input_area, app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let status = if app.is_loading {
        Span::styled(" ⟳ Thinking... ", Style::default().fg(Color::Yellow))
    } else {
        Span::styled(" ● Connected ", Style::default().fg(Color::Green))
    };

    let gemini_status = if app.gemini_enabled {
        Span::styled(" Gemini ● ", Style::default().fg(Color::Rgb(66, 133, 244))) // Google blue
    } else {
        Span::styled(" Gemini ○ ", Style::default().fg(Color::DarkGray))
    };

    let header = Line::from(vec![
        Span::styled(" KCP Copilot ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("│"),
        Span::styled(" Claude ", Style::default().fg(Color::Rgb(204, 169, 120))), // Claude gold
        status,
        Span::raw("│"),
        gemini_status,
        Span::raw("│"),
        Span::styled(
            " Ctrl+C quit  ↑↓ scroll ",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    frame.render_widget(
        Paragraph::new(header).style(Style::default().bg(Color::Rgb(30, 30, 40))),
        area,
    );
}

fn render_chat(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Chat (Claude) ")
        .title_style(Style::default().fg(Color::Rgb(204, 169, 120)).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)));

    let inner = block.inner(area);

    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.chat_history {
        let (prefix, style) = match msg.role {
            Role::User => (
                "You: ",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Role::Assistant => ("AI: ", Style::default().fg(Color::Rgb(130, 200, 255))),
            Role::System => ("", Style::default().fg(Color::DarkGray).italic()),
        };

        let width = inner.width.saturating_sub(2) as usize;
        let wrapped = textwrap::wrap(&msg.content, width.saturating_sub(prefix.len()));

        for (i, line) in wrapped.iter().enumerate() {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(line.to_string(), style),
                ]));
            } else {
                let indent = " ".repeat(prefix.len());
                lines.push(Line::from(vec![
                    Span::raw(indent),
                    Span::styled(line.to_string(), style),
                ]));
            }
        }
        lines.push(Line::raw(""));
    }

    if app.is_loading {
        lines.push(Line::from(vec![Span::styled(
            "🧠 Analyzing cluster state...",
            Style::default().fg(Color::Yellow).italic(),
        )]));
    }

    let chat = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .scroll((app.scroll_offset, 0));

    frame.render_widget(block, area);
    frame.render_widget(chat, inner);
}

fn render_cluster(frame: &mut Frame, area: Rect, app: &App) {
    let title = match &app.cluster_state.last_updated {
        Some(t) => format!(" Cluster Overview ({}) ", t),
        None => " Cluster Overview ".to_string(),
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)));

    let inner = block.inner(area);

    let mut lines: Vec<Line> = Vec::new();

    // Nodes section
    lines.push(Line::from(Span::styled(
        "Nodes",
        Style::default().fg(Color::White).bold(),
    )));

    for node in &app.cluster_state.nodes {
        let indicator = if node.ready { "●" } else { "○" };
        let color = if node.ready { Color::Green } else { Color::Red };
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", indicator), Style::default().fg(color)),
            Span::styled(&node.name, Style::default().fg(Color::White)),
            Span::styled(
                format!("  {}", node.version),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::raw(""));

    // Namespaces section
    lines.push(Line::from(Span::styled(
        "Namespaces",
        Style::default().fg(Color::White).bold(),
    )));

    for ns in &app.cluster_state.namespaces {
        let has_issues = !ns.issues.is_empty();
        let color = if has_issues { Color::Yellow } else { Color::Green };
        let indicator = if has_issues { "⚠" } else { "●" };

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", indicator), Style::default().fg(color)),
            Span::styled(&ns.name, Style::default().fg(Color::White)),
            Span::styled(
                format!("  ({})", ns.pods_summary),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        for issue in &ns.issues {
            lines.push(Line::from(Span::styled(
                format!("    └ {}", issue),
                Style::default().fg(Color::Red),
            )));
        }
    }

    if !app.cluster_state.warnings.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Recent Warnings",
            Style::default().fg(Color::Yellow).bold(),
        )));

        for warning in &app.cluster_state.warnings {
            let truncated: String = warning.chars().take(inner.width as usize - 4).collect();
            lines.push(Line::from(Span::styled(
                format!("  ⚡ {}", truncated),
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    let cluster = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });

    frame.render_widget(block, area);
    frame.render_widget(cluster, inner);
}

fn render_gemini_insights(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Gemini Insights ")
        .title_style(Style::default().fg(Color::Rgb(66, 133, 244)).bold()) // Google blue
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)));

    let inner = block.inner(area);

    let mut lines: Vec<Line> = Vec::new();

    if app.gemini_insights.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Monitoring cluster events... insights will appear here.",
            Style::default().fg(Color::DarkGray).italic(),
        )));
    } else {
        // Show most recent insights (last N that fit)
        let max_lines = inner.height as usize;
        let start = if app.gemini_insights.len() > max_lines {
            app.gemini_insights.len() - max_lines
        } else {
            0
        };

        for insight in &app.gemini_insights[start..] {
            let color = match insight.severity {
                InsightSeverity::Critical => Color::Red,
                InsightSeverity::Warning => Color::Yellow,
                InsightSeverity::Info => Color::Rgb(100, 180, 255),
            };

            let max_msg_len = inner.width as usize - 15; // room for icon + timestamp
            let truncated: String = insight.message.chars().take(max_msg_len).collect();

            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", insight.icon),
                    Style::default().fg(color),
                ),
                Span::styled(
                    truncated,
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!("  {}", insight.timestamp),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    let insights = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });

    frame.render_widget(block, area);
    frame.render_widget(insights, inner);
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let style = if app.is_loading {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    let input = Paragraph::new(app.input.as_str())
        .style(style)
        .block(
            Block::default()
                .title(" > ")
                .title_style(Style::default().fg(Color::Cyan))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 60, 80))),
        );

    frame.render_widget(input, area);

    if !app.is_loading {
        frame.set_cursor_position((area.x + app.input.len() as u16 + 3, area.y + 1));
    }
}
