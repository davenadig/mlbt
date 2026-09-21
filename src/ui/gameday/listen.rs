use crate::state::gameday::GamedayState;
use tui::prelude::*;
use tui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub fn render(state: &GamedayState, area: Rect, buf: &mut Buffer) {
    let width = area.width.min(66);
    let height = area.height.min(13);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    Clear.render(rect, buf);
    let selected = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let choice = |home, name| {
        Line::styled(
            format!(
                "{} {}  ({})",
                if state.listen.home == home { ">" } else { " " },
                name,
                if home { "home" } else { "away" }
            ),
            if state.listen.home == home {
                selected
            } else {
                Style::default()
            },
        )
    };
    let lines = vec![
        Line::from("Live team radio · Mac audio"),
        Line::from("Audio is live, independent of the pitch cursor."),
        Line::from(""),
        choice(false, state.game.away_team.name),
        choice(true, state.game.home_team.name),
        Line::from(""),
        Line::from("←/→ or Tab: team   Enter: tune   x: stop   Esc: close"),
        Line::from(""),
        Line::from(state.listen.message.clone()),
    ];
    Paragraph::new(lines)
        .style(Style::default().bg(Color::Rgb(24, 24, 27)).fg(Color::White))
        .block(
            Block::default()
                .title(" Listen · SiriusXM ")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true })
        .render(rect, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picker_renders_both_teams_and_errors_at_normal_and_narrow_widths() {
        let mut state = GamedayState::default();
        state.game.away_team.name = "Boston Red Sox";
        state.game.home_team.name = "New York Yankees";
        state.listen.message = "Feed unavailable".into();
        for width in [40, 80, 120] {
            let area = Rect::new(0, 0, width, 24);
            let mut buf = Buffer::empty(area);
            render(&state, area, &mut buf);
            let text: String = buf.content.iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains("Boston Red Sox"));
            assert!(text.contains("New York Yankees"));
            assert!(text.contains("Feed unavailable"));
        }
    }
}
