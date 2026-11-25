//! Geometry primitives, 8-edge floating resize math, and multi-touch active key tracking.

use std::collections::HashMap;

use crate::config::WindowGeometry;
use crate::keycode::{KeyCode, ModifierKind};

pub const MIN_WINDOW_WIDTH: u32 = 420;
pub const MIN_WINDOW_HEIGHT: u32 = 180;

/// 2D floating-point coordinate in surface-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Axis-aligned rectangle in surface-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    #[must_use]
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    #[must_use]
    pub fn inset(&self, amount: f32) -> Self {
        Self {
            x: self.x + amount,
            y: self.y + amount,
            w: (self.w - 2.0 * amount).max(0.0),
            h: (self.h - 2.0 * amount).max(0.0),
        }
    }
}

/// Output display bounds used to keep the floating overlay reachable on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputBounds {
    pub width: u32,
    pub height: u32,
}

impl Default for OutputBounds {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
        }
    }
}

/// All 8 floating resize edges/corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

impl ResizeEdge {
    #[must_use]
    pub const fn moves_left(self) -> bool {
        matches!(self, Self::West | Self::NorthWest | Self::SouthWest)
    }

    #[must_use]
    pub const fn moves_right(self) -> bool {
        matches!(self, Self::East | Self::NorthEast | Self::SouthEast)
    }

    #[must_use]
    pub const fn moves_top(self) -> bool {
        matches!(self, Self::North | Self::NorthWest | Self::NorthEast)
    }

    #[must_use]
    pub const fn moves_bottom(self) -> bool {
        matches!(self, Self::South | Self::SouthWest | Self::SouthEast)
    }
}

/// Hit-test a pointer position against the 8 window edges and bottom-right resize grip.
#[must_use]
pub fn detect_resize_edge(
    p: Point,
    width: f32,
    height: f32,
    edge_px: f32,
    grip_px: f32,
) -> Option<ResizeEdge> {
    if p.x >= width - grip_px && p.y >= height - grip_px {
        return Some(ResizeEdge::SouthEast);
    }

    let left = p.x <= edge_px;
    let right = p.x >= width - edge_px;
    let top = p.y <= edge_px;
    let bottom = p.y >= height - edge_px;

    match (left, right, top, bottom) {
        (true, _, true, _) => Some(ResizeEdge::NorthWest),
        (_, true, true, _) => Some(ResizeEdge::NorthEast),
        (true, _, _, true) => Some(ResizeEdge::SouthWest),
        (_, true, _, true) => Some(ResizeEdge::SouthEast),
        (true, _, _, _) => Some(ResizeEdge::West),
        (_, true, _, _) => Some(ResizeEdge::East),
        (_, _, true, _) => Some(ResizeEdge::North),
        (_, _, _, true) => Some(ResizeEdge::South),
        _ => None,
    }
}

/// Compute a clamped move position inside output bounds (keeping at least 80 px visible).
#[must_use]
pub fn clamp_move(geom: WindowGeometry, dx: i32, dy: i32, bounds: OutputBounds) -> WindowGeometry {
    let max_x = (bounds.width as i32 - 80).max(0);
    let max_y = (bounds.height as i32 - 60).max(0);
    let mut next = geom;
    next.x = (geom.x + dx).clamp(0, max_x);
    next.y = (geom.y + dy).clamp(0, max_y);
    next
}

/// Compute clamped geometry after resizing an edge by `(dx, dy)` logical pixels.
#[must_use]
pub fn apply_resize(
    geom: WindowGeometry,
    edge: ResizeEdge,
    dx: i32,
    dy: i32,
    bounds: OutputBounds,
) -> WindowGeometry {
    let max_w = bounds.width.max(MIN_WINDOW_WIDTH) as i32;
    let max_h = bounds.height.max(MIN_WINDOW_HEIGHT) as i32;

    let mut left = geom.x;
    let mut top = geom.y;
    let mut right = geom.x + geom.width as i32;
    let mut bottom = geom.y + geom.height as i32;

    match edge {
        ResizeEdge::East => right += dx,
        ResizeEdge::South => bottom += dy,
        ResizeEdge::SouthEast => {
            right += dx;
            bottom += dy;
        }
        ResizeEdge::West => left += dx,
        ResizeEdge::North => top += dy,
        ResizeEdge::NorthWest => {
            left += dx;
            top += dy;
        }
        ResizeEdge::NorthEast => {
            right += dx;
            top += dy;
        }
        ResizeEdge::SouthWest => {
            left += dx;
            bottom += dy;
        }
    }

    if edge.moves_left() {
        left = left.clamp(0, right - MIN_WINDOW_WIDTH as i32);
    } else {
        right = right.clamp(left + MIN_WINDOW_WIDTH as i32, max_w);
    }

    if edge.moves_top() {
        top = top.clamp(0, bottom - MIN_WINDOW_HEIGHT as i32);
    } else {
        bottom = bottom.clamp(top + MIN_WINDOW_HEIGHT as i32, max_h);
    }

    WindowGeometry {
        x: left,
        y: top,
        width: (right - left).clamp(MIN_WINDOW_WIDTH as i32, max_w) as u32,
        height: (bottom - top).clamp(MIN_WINDOW_HEIGHT as i32, max_h) as u32,
    }
}

/// Identifies either the primary pointer (`Pointer`) or an individual multi-touch finger (`Touch(id)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContactId {
    Pointer,
    Touch(i32),
}

/// Information about a currently held key under a pointer or finger contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveContact {
    pub key_id: String,
    pub keycode: Option<KeyCode>,
    pub modifier: Option<ModifierKind>,
    pub right_click_shift: bool,
    pub chording_used: bool,
}

/// Multi-touch contact tracker enabling simultaneous modifier hold + letter tap chording.
#[derive(Debug, Clone, Default)]
pub struct MultiTouchTracker {
    contacts: HashMap<ContactId, ActiveContact>,
}

impl MultiTouchTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            contacts: HashMap::new(),
        }
    }

    pub fn insert(&mut self, id: ContactId, contact: ActiveContact) -> Option<ActiveContact> {
        if contact.modifier.is_none() {
            // A non-modifier key was pressed while another finger may be holding a modifier.
            for existing in self.contacts.values_mut() {
                if existing.modifier.is_some() {
                    existing.chording_used = true;
                }
            }
        }
        self.contacts.insert(id, contact)
    }

    pub fn remove(&mut self, id: ContactId) -> Option<ActiveContact> {
        self.contacts.remove(&id)
    }

    #[must_use]
    pub fn is_key_pressed(&self, key_id: &str) -> bool {
        self.contacts.values().any(|c| c.key_id == key_id)
    }

    #[must_use]
    pub fn has_other_contact_holding_keycode(&self, keycode: KeyCode) -> bool {
        self.contacts.values().any(|c| c.keycode == Some(keycode))
    }

    pub fn clear(&mut self) -> Vec<ActiveContact> {
        self.contacts.drain().map(|(_, v)| v).collect()
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.contacts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_all_eight_resize_edges_and_grip() {
        let w = 800.0;
        let h = 300.0;
        assert_eq!(
            detect_resize_edge(Point::new(790.0, 290.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::SouthEast)
        );
        assert_eq!(
            detect_resize_edge(Point::new(3.0, 3.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::NorthWest)
        );
        assert_eq!(
            detect_resize_edge(Point::new(796.0, 4.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::NorthEast)
        );
        assert_eq!(
            detect_resize_edge(Point::new(4.0, 296.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::SouthWest)
        );
        assert_eq!(
            detect_resize_edge(Point::new(400.0, 2.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::North)
        );
        assert_eq!(
            detect_resize_edge(Point::new(400.0, 298.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::South)
        );
        assert_eq!(
            detect_resize_edge(Point::new(2.0, 150.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::West)
        );
        assert_eq!(
            detect_resize_edge(Point::new(798.0, 150.0), w, h, 8.0, 18.0),
            Some(ResizeEdge::East)
        );
        assert_eq!(
            detect_resize_edge(Point::new(400.0, 150.0), w, h, 8.0, 18.0),
            None
        );
    }

    #[test]
    fn resize_respects_minimum_dimensions_and_bounds() {
        let bounds = OutputBounds {
            width: 1920,
            height: 1080,
        };
        let start = WindowGeometry {
            x: 200,
            y: 500,
            width: 800,
            height: 300,
        };

        let shrunk = apply_resize(start, ResizeEdge::SouthEast, -700, -500, bounds);
        assert_eq!(shrunk.width, MIN_WINDOW_WIDTH);
        assert_eq!(shrunk.height, MIN_WINDOW_HEIGHT);

        let moved_west = apply_resize(start, ResizeEdge::West, -300, 0, bounds);
        assert_eq!(moved_west.x, 0);
        assert_eq!(moved_west.width, 1000);
    }

    #[test]
    fn multi_touch_marks_modifier_as_chording_when_letter_tapped() {
        let mut tracker = MultiTouchTracker::new();
        tracker.insert(
            ContactId::Touch(1),
            ActiveContact {
                key_id: "KEY_LEFTSHIFT".to_owned(),
                keycode: Some(KeyCode::LEFTSHIFT),
                modifier: Some(ModifierKind::Shift),
                right_click_shift: false,
                chording_used: false,
            },
        );
        tracker.insert(
            ContactId::Touch(2),
            ActiveContact {
                key_id: "KEY_A".to_owned(),
                keycode: Some(KeyCode::A),
                modifier: None,
                right_click_shift: false,
                chording_used: false,
            },
        );

        let letter = tracker.remove(ContactId::Touch(2)).expect("letter contact");
        assert_eq!(letter.keycode, Some(KeyCode::A));

        let shift = tracker.remove(ContactId::Touch(1)).expect("shift contact");
        assert!(shift.chording_used);
    }
}
