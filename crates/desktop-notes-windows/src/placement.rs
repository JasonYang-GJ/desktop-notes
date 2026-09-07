use serde::{Deserialize, Serialize};
use thiserror::Error;

const MIN_SCALE_MILLI: u32 = 500;
const MAX_SCALE_MILLI: u32 = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VisualState {
    Collapsed,
    Normal,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DockEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Rect {
    pub fn right(self) -> i64 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(self) -> i64 {
        self.y.saturating_add(self.height)
    }

    fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorLayout {
    pub fingerprint: String,
    pub fallback_name: String,
    pub work_area: Rect,
    pub scale_milli: u32,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedPlacement {
    pub monitor_fingerprint: Option<String>,
    pub monitor_fallback_name: Option<String>,
    pub rect: Rect,
    pub saved_scale_milli: u32,
    pub visual_state: VisualState,
    pub dock_edge: Option<DockEdge>,
    pub vertical_offset_milli: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPlacement {
    pub monitor_fingerprint: String,
    pub monitor_fallback_name: String,
    pub rect: Rect,
    pub scale_milli: u32,
    pub visual_state: VisualState,
    pub dock_edge: Option<DockEdge>,
    pub recovered_to_primary: bool,
    pub resized_to_work_area: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PlacementError {
    #[error("no valid monitor work area is available")]
    NoValidMonitor,
}

pub fn recover_placement(
    saved: Option<&PersistedPlacement>,
    monitors: &[MonitorLayout],
    requested_state: VisualState,
) -> Result<ResolvedPlacement, PlacementError> {
    let valid_monitors: Vec<&MonitorLayout> = monitors
        .iter()
        .filter(|monitor| {
            monitor.work_area.is_valid()
                && (MIN_SCALE_MILLI..=MAX_SCALE_MILLI).contains(&monitor.scale_milli)
        })
        .collect();
    if valid_monitors.is_empty() {
        return Err(PlacementError::NoValidMonitor);
    }

    let exact = saved.and_then(|placement| {
        placement.monitor_fingerprint.as_ref().and_then(|identity| {
            valid_monitors
                .iter()
                .copied()
                .find(|monitor| monitor.fingerprint == *identity)
        })
    });
    let fallback = saved.and_then(|placement| {
        placement.monitor_fallback_name.as_ref().and_then(|name| {
            valid_monitors
                .iter()
                .copied()
                .find(|monitor| monitor.fallback_name == *name)
        })
    });
    let primary = valid_monitors
        .iter()
        .copied()
        .find(|monitor| monitor.primary)
        .unwrap_or(valid_monitors[0]);
    let monitor = exact.or(fallback).unwrap_or(primary);
    let recovered_to_primary = saved.is_some() && exact.is_none() && fallback.is_none();

    let default = default_rect(requested_state, monitor.work_area, monitor.scale_milli);
    let saved_valid = saved.is_some_and(|placement| {
        placement.rect.is_valid()
            && (MIN_SCALE_MILLI..=MAX_SCALE_MILLI).contains(&placement.saved_scale_milli)
    });
    let mut candidate = if let Some(placement) = saved.filter(|_| saved_valid) {
        Rect {
            x: placement.rect.x,
            y: placement.rect.y,
            width: scale_dimension(
                placement.rect.width,
                placement.saved_scale_milli,
                monitor.scale_milli,
            ),
            height: scale_dimension(
                placement.rect.height,
                placement.saved_scale_milli,
                monitor.scale_milli,
            ),
        }
    } else {
        default
    };

    let minimum = minimum_size(requested_state, monitor.scale_milli);
    let original_size = (candidate.width, candidate.height);
    candidate.width = candidate.width.clamp(
        minimum.0.min(monitor.work_area.width),
        monitor.work_area.width,
    );
    candidate.height = candidate.height.clamp(
        minimum.1.min(monitor.work_area.height),
        monitor.work_area.height,
    );
    let resized_to_work_area = original_size != (candidate.width, candidate.height);

    let dock_edge = saved
        .filter(|_| saved_valid)
        .and_then(|value| value.dock_edge);
    candidate = if let Some(edge) = dock_edge {
        dock_to_work_area(
            candidate,
            monitor.work_area,
            edge,
            saved.and_then(|value| value.vertical_offset_milli),
        )
    } else {
        clamp_inside(candidate, monitor.work_area)
    };

    Ok(ResolvedPlacement {
        monitor_fingerprint: monitor.fingerprint.clone(),
        monitor_fallback_name: monitor.fallback_name.clone(),
        rect: candidate,
        scale_milli: monitor.scale_milli,
        visual_state: requested_state,
        dock_edge,
        recovered_to_primary,
        resized_to_work_area,
    })
}

pub fn detect_edge_snap(rect: Rect, work_area: Rect, threshold: i64) -> Option<DockEdge> {
    if !rect.is_valid() || !work_area.is_valid() || threshold < 0 {
        return None;
    }
    let distances = [
        (coordinate_distance(rect.x, work_area.x), DockEdge::Left),
        (
            coordinate_distance(rect.right(), work_area.right()),
            DockEdge::Right,
        ),
        (coordinate_distance(rect.y, work_area.y), DockEdge::Top),
        (
            coordinate_distance(rect.bottom(), work_area.bottom()),
            DockEdge::Bottom,
        ),
    ];
    distances
        .into_iter()
        .filter(|(distance, _)| *distance <= threshold)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, edge)| edge)
}

fn coordinate_distance(left: i64, right: i64) -> i64 {
    (i128::from(left) - i128::from(right))
        .abs()
        .min(i128::from(i64::MAX)) as i64
}

fn default_rect(state: VisualState, work_area: Rect, scale_milli: u32) -> Rect {
    let (logical_width, logical_height) = match state {
        VisualState::Collapsed => (360_i64, 132_i64),
        VisualState::Normal => (900, 720),
        VisualState::Expanded => (1_280, 820),
    };
    let width = logical_to_physical(logical_width, scale_milli).min(work_area.width);
    let height = logical_to_physical(logical_height, scale_milli).min(work_area.height);
    Rect {
        x: work_area.x.saturating_add((work_area.width - width) / 2),
        y: work_area.y.saturating_add((work_area.height - height) / 2),
        width,
        height,
    }
}

fn minimum_size(state: VisualState, scale_milli: u32) -> (i64, i64) {
    let logical = match state {
        VisualState::Collapsed => (288_i64, 112_i64),
        VisualState::Normal => (640, 520),
        VisualState::Expanded => (900, 620),
    };
    (
        logical_to_physical(logical.0, scale_milli),
        logical_to_physical(logical.1, scale_milli),
    )
}

fn scale_dimension(value: i64, source_scale: u32, target_scale: u32) -> i64 {
    i128::from(value)
        .saturating_mul(i128::from(target_scale))
        .checked_div(i128::from(source_scale))
        .unwrap_or(i128::from(i64::MAX))
        .clamp(1, i128::from(i64::MAX)) as i64
}

fn logical_to_physical(value: i64, scale_milli: u32) -> i64 {
    i128::from(value)
        .saturating_mul(i128::from(scale_milli))
        .checked_div(1_000)
        .unwrap_or(i128::from(i64::MAX))
        .clamp(1, i128::from(i64::MAX)) as i64
}

fn clamp_inside(mut rect: Rect, work_area: Rect) -> Rect {
    let max_x = work_area.right().saturating_sub(rect.width);
    let max_y = work_area.bottom().saturating_sub(rect.height);
    rect.x = rect.x.clamp(work_area.x, max_x.max(work_area.x));
    rect.y = rect.y.clamp(work_area.y, max_y.max(work_area.y));
    rect
}

fn dock_to_work_area(
    mut rect: Rect,
    work_area: Rect,
    edge: DockEdge,
    vertical_offset_milli: Option<u16>,
) -> Rect {
    let offset = i64::from(vertical_offset_milli.unwrap_or(0).min(1_000));
    let vertical_room = work_area.height.saturating_sub(rect.height).max(0);
    let horizontal_room = work_area.width.saturating_sub(rect.width).max(0);
    match edge {
        DockEdge::Left => {
            rect.x = work_area.x;
            rect.y = work_area
                .y
                .saturating_add(vertical_room.saturating_mul(offset) / 1_000);
        }
        DockEdge::Right => {
            rect.x = work_area.right().saturating_sub(rect.width);
            rect.y = work_area
                .y
                .saturating_add(vertical_room.saturating_mul(offset) / 1_000);
        }
        DockEdge::Top => {
            rect.x = work_area.x.saturating_add(horizontal_room / 2);
            rect.y = work_area.y;
        }
        DockEdge::Bottom => {
            rect.x = work_area.x.saturating_add(horizontal_room / 2);
            rect.y = work_area.bottom().saturating_sub(rect.height);
        }
    }
    clamp_inside(rect, work_area)
}
