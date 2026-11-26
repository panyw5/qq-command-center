use crate::app::{App, Overlay, Screen, Tab};
use crate::model::{ChatMessage, Session};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

const HELP_SESSIONS: &str = "j/k 移动  Enter 打开  Tab 切换  / 过滤  r 刷新  q 退出";
const HELP_CHAT: &str = "j/k 选择  o 打开媒体  y 复制链接  i 输入  a @  p 图  G 底  Esc 返回";
const HELP_INPUT: &str = "Enter 发送  Esc 取消  (a 加@提及 / p 附加图片先退出输入)";

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// CJK 近似宽度的列宽估计（全角 2 列、其他 1 列）。
fn disp_width(s: &str) -> usize {
    s.chars()
        .map(|c| if (c as u32) >= 0x2E80 { 2 } else { 1 })
        .sum()
}

/// 按显示列宽把文本切成多行。
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
            continue;
        }
        let w = if (ch as u32) >= 0x2E80 { 2 } else { 1 };
        if cur_w + w > width && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w += w;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn fmt_time(epoch: i64) -> String {
    // msgTime 可能是秒或毫秒
    let secs = if epoch > 10_000_000_000 {
        epoch / 1000
    } else {
        epoch
    };
    chrono_lite(secs)
}

/// 极简 UTC+8 时间格式化（不引 chrono 依赖）。
fn chrono_lite(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = (rem / 3600 + 8) % 24;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;
    // days → 公历日期（civil-from-days 算法）
    let civil = days + 719_468;
    let era = civil.div_euclid(146_097);
    let day_of_era = civil.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn session_line(s: &Session, selected: bool, width: usize) -> Line<'static> {
    let tag = format!("[{}]", s.tag);
    let name = s.display_name();
    let uin = if s.peer_uin.is_empty() {
        s.peer_uid.clone()
    } else {
        s.peer_uin.clone()
    };
    let text = format!("{tag} {name} ({uin})");
    let style = if selected { accent() } else { Style::default() };
    let mut text = text;
    // 截断到可用宽度
    while disp_width(&text) > width && !text.is_empty() {
        text.pop();
    }
    Line::from(Span::styled(text, style))
}

fn message_lines(msg: &ChatMessage, width: usize, selected: bool) -> Vec<Line<'static>> {
    let marker = if selected { "▎" } else { " " };
    let head = format!("{marker}{} {}", fmt_time(msg.time), msg.sender);
    let head_style = if selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if msg.is_self {
        accent()
    } else {
        dim()
    };
    let mut lines = vec![Line::from(Span::styled(head, head_style))];
    for seg in wrap_text(&msg.body, width.saturating_sub(2)) {
        let style = if selected {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("  {seg}"), style)));
    }
    lines
}

fn draw_sessions(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    // 标题 + 页签
    let tabs = [Tab::Recent, Tab::Friends, Tab::Groups]
        .iter()
        .map(|t| {
            if *t == app.tab {
                Span::styled(format!(" {} ", t.title()), accent())
            } else {
                Span::styled(format!(" {} ", t.title()), dim())
            }
        })
        .collect::<Vec<_>>();
    let mut title_spans = vec![Span::styled(" QCE ", accent()), Span::raw(" ")];
    title_spans.extend(tabs);
    if app.filtering || !app.filter.is_empty() {
        title_spans.push(Span::styled(format!("  /{}", app.filter), dim()));
    }
    let title = Paragraph::new(Line::from(title_spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" QQ Chat Exporter TUI "),
    );
    f.render_widget(title, chunks[0]);

    // 会话列表
    let inner_width = chunks[1].width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .filtered_display()
        .iter()
        .enumerate()
        .map(|(i, s)| ListItem::new(session_line(s, i == app.selected, inner_width)))
        .collect();
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" 会话 "));
    f.render_widget(list, chunks[1]);

    // 状态 + 帮助
    let status = Paragraph::new(Span::styled(
        format!(" {}{}", app.status, if app.busy { " ⏳" } else { "" }),
        dim(),
    ));
    f.render_widget(status, chunks[2]);
    f.render_widget(
        Paragraph::new(Span::styled(format!(" {HELP_SESSIONS}"), dim())),
        chunks[3],
    );
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled(" < ", dim()),
        Span::styled(app.peer_name(), accent()),
    ]))
    .block(Block::default().borders(Borders::ALL).title(" 会话 "));
    f.render_widget(title, chunks[0]);

    // 消息区：预换行后按距底偏移滚动；顺带回填每条消息的行范围和命中区域
    let inner = Rect {
        x: chunks[1].x + 1,
        y: chunks[1].y + 1,
        width: chunks[1].width.saturating_sub(2),
        height: chunks[1].height.saturating_sub(2),
    };
    app.msg_area = inner;
    let w = usize::from(inner.width.max(8));
    let mut lines: Vec<Line> = Vec::new();
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(app.messages.len());
    for (i, msg) in app.messages.iter().enumerate() {
        let begin = lines.len();
        lines.extend(message_lines(msg, w, i == app.sel_msg));
        ranges.push((begin, lines.len()));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(" (无消息)", dim())));
    }
    app.msg_ranges = ranges;

    // 滚动起点：scroll_bottom = 距底行数；再保证选中消息在可视区内
    let view_h = usize::from(inner.height);
    let max_start = lines.len().saturating_sub(view_h);
    let mut start = max_start.saturating_sub(app.scroll_bottom);
    if let Some(&(s_line, e_line)) = app.msg_ranges.get(app.sel_msg) {
        if s_line < start {
            start = s_line;
        } else if e_line > start + view_h {
            start = (e_line + 1).saturating_sub(view_h).min(max_start);
        }
    }
    app.scroll_bottom = max_start.saturating_sub(start);
    app.msg_start = start;

    let start_y = u16::try_from(start).unwrap_or(u16::MAX);
    let para = Paragraph::new(ratatui::text::Text::from(lines))
        .scroll((start_y, 0))
        .block(Block::default().borders(Borders::ALL).title(" 消息 "));
    f.render_widget(para, chunks[1]);

    // 附加内容提示行（图片/提及）
    let mut chips = Vec::new();
    if let Some((name, _)) = &app.image {
        chips.push(format!("📎{name}"));
    }
    for m in &app.mentions {
        chips.push(format!("@{}", m.name));
    }
    let chips_line = if chips.is_empty() {
        String::new()
    } else {
        format!(" {}", chips.join("  "))
    };
    f.render_widget(
        Paragraph::new(Span::styled(chips_line, Style::default().fg(Color::Yellow))),
        chunks[2],
    );

    // 输入行
    let input_line = if app.overlay == Overlay::Input {
        format!(" > {}_", app.input)
    } else if app.overlay == Overlay::ImagePath {
        format!(" 图片路径: {}_", app.image_path)
    } else {
        " 按 i 输入消息，Enter 快捷进入输入".to_string()
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            input_line,
            if app.overlay == Overlay::Input {
                Style::default()
            } else {
                dim()
            },
        )),
        chunks[3],
    );

    // 状态/帮助合并行
    let help = if app.overlay == Overlay::Input {
        HELP_INPUT
    } else {
        HELP_CHAT
    };
    f.render_widget(
        Paragraph::new(Span::styled(format!(" {} | {}", app.status, help), dim())),
        chunks[4],
    );
}

fn draw_mention_picker(f: &mut Frame, app: &App) {
    let area = f.area();
    let w = (area.width * 60 / 100).clamp(20, area.width);
    let h = (area.height * 60 / 100).clamp(8, area.height);
    let rect = Rect {
        x: (area.width - w) / 2,
        y: (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(rect);

    let filter = Paragraph::new(Span::styled(
        format!(" 过滤: {}", app.member_filter),
        Style::default(),
    ));
    f.render_widget(filter, chunks[0]);

    let inner_w = chunks[1].width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .filtered_members()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let text = format!(" {} ({})", m.name, m.uin);
            let mut text = text;
            while disp_width(&text) > inner_w && !text.is_empty() {
                text.pop();
            }
            ListItem::new(Line::from(Span::styled(
                text,
                if i == app.member_selected {
                    accent()
                } else {
                    Style::default()
                },
            )))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if app.members_loading {
            " @ 成员 (加载中…) "
        } else {
            " @ 成员 "
        });
    f.render_widget(List::new(items).block(block), chunks[1]);
}

pub fn draw(f: &mut Frame, app: &mut App) {
    match app.screen {
        Screen::Sessions => draw_sessions(f, app),
        Screen::Chat => draw_chat(f, app, f.area()),
    }
    if app.overlay == Overlay::MentionPicker {
        draw_mention_picker(f, app);
    }
}
