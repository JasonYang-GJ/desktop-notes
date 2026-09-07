use desktop_notes_windows::{
    DockEdge, MonitorLayout, PersistedPlacement, Rect, VisualState, detect_edge_snap,
    recover_placement,
};

fn monitor(
    fingerprint: &str,
    name: &str,
    work_area: Rect,
    scale_milli: u32,
    primary: bool,
) -> MonitorLayout {
    MonitorLayout {
        fingerprint: fingerprint.to_owned(),
        fallback_name: name.to_owned(),
        work_area,
        scale_milli,
        primary,
    }
}

fn saved(identity: &str, rect: Rect, state: VisualState) -> PersistedPlacement {
    PersistedPlacement {
        monitor_fingerprint: Some(identity.to_owned()),
        monitor_fallback_name: None,
        rect,
        saved_scale_milli: 1_000,
        visual_state: state,
        dock_edge: None,
        vertical_offset_milli: None,
    }
}

#[test]
fn valid_single_monitor_placement_is_preserved() {
    let primary = monitor(
        "display-a",
        "DISPLAY1",
        Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        },
        1_000,
        true,
    );
    let placement = saved(
        "display-a",
        Rect {
            x: 120,
            y: 80,
            width: 900,
            height: 720,
        },
        VisualState::Normal,
    );

    let resolved = recover_placement(Some(&placement), &[primary], VisualState::Normal).unwrap();

    assert_eq!(resolved.rect, placement.rect);
    assert!(!resolved.recovered_to_primary);
    assert!(!resolved.resized_to_work_area);
}

#[test]
fn missing_monitor_and_rdp_topology_recover_to_primary() {
    let primary = monitor(
        "rdp-primary",
        "RDP",
        Rect {
            x: 0,
            y: 0,
            width: 1366,
            height: 728,
        },
        1_250,
        true,
    );
    let placement = saved(
        "unplugged-display",
        Rect {
            x: 2600,
            y: 90,
            width: 1000,
            height: 700,
        },
        VisualState::Normal,
    );

    let resolved = recover_placement(Some(&placement), &[primary], VisualState::Normal).unwrap();

    assert!(resolved.recovered_to_primary);
    assert_eq!(resolved.monitor_fingerprint, "rdp-primary");
    assert_fully_visible(
        resolved.rect,
        Rect {
            x: 0,
            y: 0,
            width: 1366,
            height: 728,
        },
    );
}

#[test]
fn fallback_device_name_is_used_before_primary() {
    let monitors = [
        monitor(
            "primary-new-id",
            "DISPLAY1",
            Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            1_000,
            true,
        ),
        monitor(
            "secondary-new-id",
            "DISPLAY2",
            Rect {
                x: -1600,
                y: 0,
                width: 1600,
                height: 860,
            },
            1_000,
            false,
        ),
    ];
    let mut placement = saved(
        "old-fingerprint",
        Rect {
            x: -1500,
            y: 50,
            width: 800,
            height: 650,
        },
        VisualState::Normal,
    );
    placement.monitor_fallback_name = Some("DISPLAY2".to_owned());

    let resolved = recover_placement(Some(&placement), &monitors, VisualState::Normal).unwrap();

    assert_eq!(resolved.monitor_fingerprint, "secondary-new-id");
    assert!(!resolved.recovered_to_primary);
    assert!(resolved.rect.x < 0);
    assert_fully_visible(resolved.rect, monitors[1].work_area);
}

#[test]
fn negative_coordinate_monitor_keeps_accessible_window() {
    let secondary = monitor(
        "display-left",
        "DISPLAY2",
        Rect {
            x: -2560,
            y: -120,
            width: 2560,
            height: 1320,
        },
        1_500,
        false,
    );
    let primary = monitor(
        "display-primary",
        "DISPLAY1",
        Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        },
        1_000,
        true,
    );
    let placement = saved(
        "display-left",
        Rect {
            x: -2300,
            y: -90,
            width: 600,
            height: 500,
        },
        VisualState::Normal,
    );

    let resolved = recover_placement(
        Some(&placement),
        &[primary, secondary.clone()],
        VisualState::Normal,
    )
    .unwrap();

    assert_eq!(resolved.scale_milli, 1_500);
    assert_fully_visible(resolved.rect, secondary.work_area);
}

#[test]
fn oversized_or_offscreen_rect_is_resized_and_clamped() {
    let work_area = Rect {
        x: 100,
        y: 40,
        width: 1280,
        height: 680,
    };
    let primary = monitor("primary", "DISPLAY1", work_area, 1_000, true);
    let placement = saved(
        "primary",
        Rect {
            x: i64::MAX,
            y: i64::MIN,
            width: 9000,
            height: 8000,
        },
        VisualState::Expanded,
    );

    let resolved = recover_placement(Some(&placement), &[primary], VisualState::Expanded).unwrap();

    assert_eq!(resolved.rect.width, work_area.width);
    assert_eq!(resolved.rect.height, work_area.height);
    assert!(resolved.resized_to_work_area);
    assert_fully_visible(resolved.rect, work_area);
}

#[test]
fn corrupt_geometry_uses_a_state_specific_safe_default() {
    let work_area = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    let primary = monitor("primary", "DISPLAY1", work_area, 1_000, true);
    let placement = saved(
        "primary",
        Rect {
            x: 100,
            y: 100,
            width: -1,
            height: 0,
        },
        VisualState::Collapsed,
    );

    let resolved = recover_placement(Some(&placement), &[primary], VisualState::Collapsed).unwrap();

    assert_eq!((resolved.rect.width, resolved.rect.height), (360, 132));
    assert_fully_visible(resolved.rect, work_area);
}

#[test]
fn all_three_states_fit_small_primary_work_area() {
    let work_area = Rect {
        x: 0,
        y: 0,
        width: 1024,
        height: 720,
    };
    let primary = monitor("primary", "DISPLAY1", work_area, 1_000, true);

    for state in [
        VisualState::Collapsed,
        VisualState::Normal,
        VisualState::Expanded,
    ] {
        let resolved = recover_placement(None, std::slice::from_ref(&primary), state).unwrap();
        assert_eq!(resolved.visual_state, state);
        assert_fully_visible(resolved.rect, work_area);
    }
}

#[test]
fn scaling_matrix_rebases_dimensions_without_leaving_work_area() {
    let work_area = Rect {
        x: 0,
        y: 0,
        width: 3840,
        height: 2080,
    };
    let placement = saved(
        "display",
        Rect {
            x: 50,
            y: 50,
            width: 800,
            height: 600,
        },
        VisualState::Normal,
    );

    for scale in [1_000, 1_250, 1_500, 2_000] {
        let current = monitor("display", "DISPLAY1", work_area, scale, true);
        let resolved =
            recover_placement(Some(&placement), &[current], VisualState::Normal).unwrap();
        assert_eq!(resolved.rect.width, 800 * i64::from(scale) / 1_000);
        assert_eq!(resolved.rect.height, 600 * i64::from(scale) / 1_000);
        assert_fully_visible(resolved.rect, work_area);
    }
}

#[test]
fn docked_edge_and_offset_are_rebuilt_against_current_work_area() {
    let work_area = Rect {
        x: -1920,
        y: 40,
        width: 1920,
        height: 1000,
    };
    let current = monitor("left", "DISPLAY2", work_area, 1_000, false);
    let mut placement = saved(
        "left",
        Rect {
            x: -8000,
            y: 9000,
            width: 720,
            height: 600,
        },
        VisualState::Normal,
    );
    placement.dock_edge = Some(DockEdge::Right);
    placement.vertical_offset_milli = Some(750);

    let resolved = recover_placement(Some(&placement), &[current], VisualState::Normal).unwrap();

    assert_eq!(resolved.rect.right(), work_area.right());
    assert_eq!(resolved.rect.y, work_area.y + 300);
    assert_fully_visible(resolved.rect, work_area);
}

#[test]
fn edge_snap_uses_nearest_edge_within_threshold() {
    let work_area = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    assert_eq!(
        detect_edge_snap(
            Rect {
                x: 9,
                y: 200,
                width: 600,
                height: 500
            },
            work_area,
            12
        ),
        Some(DockEdge::Left)
    );
    assert_eq!(
        detect_edge_snap(
            Rect {
                x: 700,
                y: 400,
                width: 600,
                height: 500
            },
            work_area,
            12
        ),
        None
    );
    assert_eq!(
        detect_edge_snap(
            Rect {
                x: i64::MIN,
                y: i64::MAX,
                width: 1,
                height: 1,
            },
            work_area,
            12,
        ),
        None
    );
}

#[test]
fn invalid_monitor_set_fails_closed() {
    let invalid = monitor(
        "invalid",
        "DISPLAY0",
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        },
        1_000,
        true,
    );
    assert!(recover_placement(None, &[invalid], VisualState::Normal).is_err());
}

fn assert_fully_visible(rect: Rect, work_area: Rect) {
    assert!(rect.width > 0 && rect.height > 0);
    assert!(rect.x >= work_area.x);
    assert!(rect.y >= work_area.y);
    assert!(rect.right() <= work_area.right());
    assert!(rect.bottom() <= work_area.bottom());
}
