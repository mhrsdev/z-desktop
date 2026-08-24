//! Drag-to-dock drop indicators.
//!
//! Pure geometry: given the cursor position over a resolved [`ShellFrame`],
//! which docked panel would receive the drop, which edge it would dock to, and
//! the rectangle a renderer should highlight. No drag state lives here — this
//! runs once per pointer move and the caller draws whatever comes back.

use crate::panel::PanelId;
use crate::region::{Rect, ShellFrame};
use serde::{Deserialize, Serialize};

/// Which part of a panel a drop would land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropZone {
    Before,
    After,
    Left,
    Right,
    Top,
    Bottom,
}

/// Where a drop would land: target panel, zone under the cursor, and the
/// highlight rectangle (always inside the panel's bounds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropIndicator {
    pub panel: PanelId,
    pub zone: DropZone,
    pub rect: Rect,
}

/// Resolve the drop indicator for a cursor position.
///
/// The panel whose rect contains the point wins; over no docked surface the
/// default is chat, centred (`After`). Inside a panel the cursor's normalized
/// position picks the zone: outer left/right thirds dock to that side, outer
/// top/bottom sixths dock above/below, anything else drops *into* the panel.
pub fn compute_drop_indicator(frame: &ShellFrame, cursor_x: f32, cursor_y: f32) -> DropIndicator {
    match frame
        .docked()
        .into_iter()
        .find(|&(_, r)| !r.is_empty() && contains(r, cursor_x, cursor_y))
    {
        Some((panel, rect)) => {
            let (zone, rect) = region(rect, cursor_x, cursor_y);
            DropIndicator { panel, zone, rect }
        }
        None => {
            // Off-shell: chat, centre. Recomputing through `region` keeps the
            // highlight rect consistent with the hovered case.
            let (_, rect) = region(frame.chat, frame.chat.center_x(), frame.chat.center_y());
            DropIndicator { panel: PanelId::Chat, zone: DropZone::After, rect }
        }
    }
}

/// Half-open point containment, so shared edges belong to exactly one panel.
fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

/// Split `rect` into thirds × sixths around the normalized cursor position.
fn region(rect: Rect, x: f32, y: f32) -> (DropZone, Rect) {
    if rect.is_empty() {
        return (DropZone::After, Rect::ZERO);
    }
    let tx = ((x - rect.x) / rect.width).clamp(0.0, 1.0);
    let ty = ((y - rect.y) / rect.height).clamp(0.0, 1.0);
    if tx < 1.0 / 3.0 {
        (DropZone::Left, Rect::new(rect.x, rect.y, rect.width / 3.0, rect.height))
    } else if tx > 2.0 / 3.0 {
        (
            DropZone::Right,
            Rect::new(rect.x + rect.width * 2.0 / 3.0, rect.y, rect.width / 3.0, rect.height),
        )
    } else if ty < 1.0 / 6.0 {
        (DropZone::Top, Rect::new(rect.x, rect.y, rect.width, rect.height / 2.0))
    } else if ty > 5.0 / 6.0 {
        (
            DropZone::Bottom,
            Rect::new(rect.x, rect.y + rect.height / 2.0, rect.width, rect.height / 2.0),
        )
    } else {
        (
            DropZone::After,
            Rect::new(
                rect.x + rect.width / 3.0,
                rect.y + rect.height / 6.0,
                rect.width / 3.0,
                rect.height * 4.0 / 6.0,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LayoutState;
    use crate::panel::PanelRegistry;

    fn frame(width: f32, height: f32) -> ShellFrame {
        let registry = PanelRegistry::personal_default();
        let layout = LayoutState::personal_default(&registry);
        crate::region::solve(&registry, &layout, width, height)
    }

    #[test]
    fn cursor_at_the_centre_of_chat_drops_into_chat() {
        let frame = frame(1536.0, 1024.0);
        let ind = compute_drop_indicator(&frame, frame.chat.center_x(), frame.chat.center_y());
        assert_eq!(ind.panel, PanelId::Chat);
        assert_eq!(ind.zone, DropZone::After);
        // Middle-ish: strictly inside the panel on both axes.
        assert!(ind.rect.x > frame.chat.x && ind.rect.right() < frame.chat.right());
        assert!(ind.rect.y > frame.chat.y && ind.rect.bottom() < frame.chat.bottom());
    }

    #[test]
    fn cursor_far_left_of_the_sidebar_docks_left() {
        let frame = frame(1536.0, 1024.0);
        let x = frame.sidebar.x + 10.0;
        let ind = compute_drop_indicator(&frame, x, frame.sidebar.center_y());
        assert_eq!(ind.panel, PanelId::Sidebar);
        assert_eq!(ind.zone, DropZone::Left);
        let third = frame.sidebar.width / 3.0;
        assert_eq!(ind.rect, Rect::new(frame.sidebar.x, frame.sidebar.y, third, frame.sidebar.height));
    }

    #[test]
    fn outer_sixths_of_a_panel_dock_top_or_bottom() {
        let frame = frame(1536.0, 1024.0);
        let mid_x = frame.chat.center_x();
        let top = compute_drop_indicator(&frame, mid_x, frame.chat.y + 1.0);
        assert_eq!(top.zone, DropZone::Top);
        assert_eq!(top.panel, PanelId::Chat);
        let bottom =
            compute_drop_indicator(&frame, mid_x, frame.chat.bottom() - 1.0);
        assert_eq!(bottom.zone, DropZone::Bottom);
    }

    #[test]
    fn a_cursor_outside_every_panel_defaults_to_chat_centred() {
        let frame = frame(1536.0, 1024.0);
        for (x, y) in [(-50.0, -50.0), (9999.0, 9999.0)] {
            let ind = compute_drop_indicator(&frame, x, y);
            assert_eq!(ind.panel, PanelId::Chat, "({x}, {y})");
            assert_eq!(ind.zone, DropZone::After, "({x}, {y})");
        }
    }

    #[test]
    fn every_indicator_rect_stays_within_its_panel() {
        let frame = frame(1536.0, 1024.0);
        for (id, panel_rect) in frame.docked() {
            if panel_rect.is_empty() {
                continue;
            }
            let steps = 9;
            for i in 0..=steps {
                for j in 0..=steps {
                    let x = panel_rect.x + panel_rect.width * i as f32 / steps as f32;
                    let y = panel_rect.y + panel_rect.height * j as f32 / steps as f32;
                    let ind = compute_drop_indicator(&frame, x, y);
                    let target = frame.rect(ind.panel);
                    assert!(
                        ind.rect.x >= target.x - 0.01
                            && ind.rect.y >= target.y - 0.01
                            && ind.rect.right() <= target.right() + 0.01
                            && ind.rect.bottom() <= target.bottom() + 0.01,
                        "{id:?} at ({x}, {y}) highlighted {:?} outside {target:?}",
                        ind.rect
                    );
                    assert!(ind.rect.width >= 0.0 && ind.rect.height >= 0.0);
                }
            }
        }
    }

    #[test]
    fn zones_round_trip_through_snake_case_json() {
        for (json, zone) in
            [("\"before\"", DropZone::Before), ("\"top\"", DropZone::Top), ("\"bottom\"", DropZone::Bottom)]
        {
            assert_eq!(serde_json::from_str::<DropZone>(json).unwrap(), zone);
            assert_eq!(serde_json::to_string(&zone).unwrap(), json);
        }
    }
}
