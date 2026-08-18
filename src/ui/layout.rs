use crate::state::gameday::GamedayPanels;
use crate::ui::input_popup;
use tui::layout::{Constraint, Layout, Rect, Size};

pub struct LayoutAreas {
    pub top_bar: Rect,
    pub main: Rect,
    pub footer: Rect,
}

const TOP_BAR_HEIGHT: u16 = 3; // length
const FOOTER_HEIGHT: u16 = 1;

impl LayoutAreas {
    pub fn new(size: Size) -> Self {
        let rect = Rect::new(0, 0, size.width, size.height);
        let [top, main, footer] = Layout::vertical([
            Constraint::Length(TOP_BAR_HEIGHT),
            Constraint::Fill(1),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .margin(1)
        .areas(rect);

        LayoutAreas {
            top_bar: top,
            main,
            footer,
        }
    }

    pub(crate) fn update(&mut self, size: Rect, full_screen: bool) {
        let constraints = match full_screen {
            true => vec![
                Constraint::Length(0),
                Constraint::Fill(1),
                Constraint::Length(FOOTER_HEIGHT),
            ],
            false => vec![
                Constraint::Length(TOP_BAR_HEIGHT),
                Constraint::Fill(1),
                Constraint::Length(FOOTER_HEIGHT),
            ],
        };
        let [top, main, footer] = Layout::vertical(constraints).areas(size);

        self.top_bar = top;
        self.main = main;
        self.footer = footer;
    }

    /// Create splits for displaying the line score on top and a box score below.
    /// If a game is Final, show the decision pitchers in between.
    pub fn for_boxscore(rect: Rect, decision_pitchers: u16) -> Vec<Rect> {
        let mut constraints = vec![
            Constraint::Length(4), // line score
            Constraint::Fill(1),   // box score
        ];
        if decision_pitchers > 0 {
            constraints.insert(1, Constraint::Length(decision_pitchers + 1)); // +1 for blank line
        }

        Layout::vertical(constraints)
            .horizontal_margin(2)
            .vertical_margin(1)
            .split(rect)
            .to_vec()
    }

    /// Create two splits for displaying the current matchup and the pitches for the at bat.
    pub fn for_at_bat(rect: Rect) -> [Rect; 2] {
        Layout::vertical([
            Constraint::Length(7), // matchup + on deck
            Constraint::Fill(1),   // pitches
        ])
        .areas(rect)
    }

    /// Create two splits for displaying game info for the current inning and the recent win
    /// probability. This is used in the `gameday` tab.
    pub fn for_info(rect: Rect, show_win_probability: bool) -> Vec<Rect> {
        let constraints = match show_win_probability {
            true => vec![Constraint::Fill(1), Constraint::Length(6)],
            false => vec![Constraint::Fill(1)],
        };
        Layout::vertical(constraints)
            .horizontal_margin(2)
            .vertical_margin(1)
            .spacing(1) // one row of padding between the plays and the win probability chart
            .split(rect)
            .to_vec()
    }

    /// Create the Gameday layouts based on how many of the panels are active.
    pub fn generate_gameday_panels(active: &GamedayPanels, area: Rect) -> Vec<Rect> {
        let constraints = match active.count() {
            0 | 1 => vec![Constraint::Percentage(100)],
            2 => vec![Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)],
            3 => vec![
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ],
            _ => vec![],
        };
        Layout::horizontal(constraints.as_slice())
            .split(area)
            .to_vec()
    }

    /// Create a centered rectangle of 4 height and 42% width for the date picker.
    pub fn create_date_picker(area: Rect) -> Rect {
        input_popup::create_popup(area, 4, 42)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_the_last_row_for_key_hints() {
        let size = Size::new(100, 30);
        let mut areas = LayoutAreas::new(size);
        areas.update(Rect::new(0, 0, 100, 30), false);

        assert_eq!(areas.top_bar, Rect::new(0, 0, 100, 3));
        assert_eq!(areas.main, Rect::new(0, 3, 100, 26));
        assert_eq!(areas.footer, Rect::new(0, 29, 100, 1));
    }

    #[test]
    fn full_screen_hides_tabs_but_keeps_key_hints() {
        let size = Size::new(80, 24);
        let mut areas = LayoutAreas::new(size);
        areas.update(Rect::new(0, 0, 80, 24), true);

        assert_eq!(areas.top_bar.height, 0);
        assert_eq!(areas.main, Rect::new(0, 0, 80, 23));
        assert_eq!(areas.footer, Rect::new(0, 23, 80, 1));
    }
}
