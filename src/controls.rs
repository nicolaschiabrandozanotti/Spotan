use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Player(&'static str),
    Follow,
    Background,
    OffsetDown,
    OffsetUp,
    ScrollUp,
    ScrollDown,
    Close,
}

#[derive(Clone, Debug)]
pub struct Button {
    pub area: Rect,
    pub label: String,
    pub action: Action,
    pub active: bool,
}
pub struct Footer {
    pub buttons: Vec<Button>,
    pub progress: Rect,
    pub top: u16,
    pub status: u16,
}

pub fn layout(
    width: u16,
    height: u16,
    margin: u16,
    playing: bool,
    follow: bool,
    background: bool,
) -> Footer {
    let available = width.saturating_sub(margin * 2).max(1);
    let groups = [
        vec![
            ("Anterior", Action::Player("previous"), false),
            (
                if playing { "Pausa" } else { "Play" },
                Action::Player("playpause"),
                true,
            ),
            ("Siguiente", Action::Player("next"), false),
            ("-5s", Action::Player("seek -5s"), false),
            ("+5s", Action::Player("seek +5s"), false),
            ("Vol-", Action::Player("voldown 5"), false),
            ("Vol+", Action::Player("volup 5"), false),
        ],
        vec![
            ("Leer↑", Action::ScrollUp, false),
            ("Leer↓", Action::ScrollDown, false),
            (
                if follow { "Auto ✓" } else { "Seguir" },
                Action::Follow,
                follow,
            ),
            (
                if background { "Fondo ✓" } else { "Fondo" },
                Action::Background,
                background,
            ),
            ("Sync-", Action::OffsetDown, false),
            ("Sync+", Action::OffsetUp, false),
            ("Cerrar", Action::Close, false),
        ],
    ];
    let mut rows: Vec<Vec<(&str, Action, bool)>> = Vec::new();
    for group in groups {
        let mut row = Vec::new();
        let mut used = 0u16;
        for item in group {
            let item_width = (item.0.width() + 2).min(available as usize) as u16;
            if !row.is_empty() && used + 1 + item_width > available {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            if !row.is_empty() {
                used += 1;
            }
            used += item_width;
            row.push(item);
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }
    let top = height.saturating_sub(rows.len() as u16 + 2);
    let mut buttons = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let used = row
            .iter()
            .map(|item| (item.0.width() + 2).min(available as usize) as u16)
            .sum::<u16>()
            + row.len().saturating_sub(1) as u16;
        let mut x = margin + (available - used) / 2;
        for (label, action, active) in row {
            let button_width = (label.width() + 2).min(available as usize) as u16;
            buttons.push(Button {
                area: Rect::new(x, top + 1 + index as u16, button_width, 1),
                label: format!("[{label}]"),
                action: *action,
                active: *active,
            });
            x += button_width + 1;
        }
    }
    Footer {
        buttons,
        progress: Rect::new(margin + 7, top, available.saturating_sub(14), 1),
        top,
        status: height.saturating_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buttons_wrap_without_overlapping_and_keep_the_same_actions() {
        for width in [20, 30, 60, 80, 150] {
            let footer = layout(width, 40, 1, true, true, true);
            assert_eq!(footer.buttons.len(), 14);
            for (i, button) in footer.buttons.iter().enumerate() {
                assert!(button.area.right() <= width && button.area.bottom() <= 40);
                for other in &footer.buttons[i + 1..] {
                    assert!(button.area.intersection(other.area).is_empty());
                }
            }
            assert!(footer
                .buttons
                .iter()
                .any(|button| button.action == Action::Player("playpause")));
        }
    }
}
