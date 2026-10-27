//! The shape tools' key (U) never cycles through its group and defaults to Custom Shape
//! (fork-local, see MY_CHANGES.md).

use crate::state::Tool;

/// The tool a press of a group's key gives. For U (the shape tools) the press never cycles: the
/// current tool of the group stays, and from outside the group it is the group's last-used tool
/// (picked in the toolbar), Custom Shape if none. Other keys keep the stock behaviour (`None`).
pub fn pick(key: char, group: &[Tool], current: Tool, last_used: Option<Tool>) -> Option<Tool> {
    if key != 'U' {
        return None;
    }
    if group.contains(&current) {
        return Some(current);
    }
    let last = last_used.filter(|t| group.contains(t));
    last.or_else(|| group.contains(&Tool::CustomShape).then_some(Tool::CustomShape)).or_else(|| group.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUP: [Tool; 3] = [Tool::Rectangle, Tool::EllipseShape, Tool::CustomShape];

    #[test]
    fn u_never_cycles_and_defaults_to_custom_shape() {
        assert_eq!(pick('U', &GROUP, Tool::CustomShape, None), Some(Tool::CustomShape));
        assert_eq!(pick('U', &GROUP, Tool::Rectangle, Some(Tool::Rectangle)), Some(Tool::Rectangle));
        assert_eq!(pick('U', &GROUP, Tool::Brush, None), Some(Tool::CustomShape));
    }

    #[test]
    fn u_from_elsewhere_gives_the_last_toolbar_choice() {
        assert_eq!(pick('U', &GROUP, Tool::Brush, Some(Tool::Rectangle)), Some(Tool::Rectangle));
        assert_eq!(pick('U', &GROUP, Tool::Brush, Some(Tool::Move)), Some(Tool::CustomShape));
    }

    #[test]
    fn other_keys_keep_the_stock_behaviour() {
        assert_eq!(pick('M', &[Tool::RectMarquee, Tool::EllipseMarquee], Tool::RectMarquee, None), None);
    }
}
