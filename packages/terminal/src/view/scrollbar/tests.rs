use super::*;

#[test]
fn proportional_thumb_maps_drag_and_scroll_offset_to_the_same_position() {
    let metrics = ScrollbarMetrics::new(200.0, 100, 20);
    assert_eq!(metrics.thumb_height, 40.0);
    assert_eq!(metrics.thumb_top(0), 0.0);
    assert_eq!(metrics.thumb_top(40), 80.0);
    assert_eq!(metrics.thumb_top(80), 160.0);
    assert_eq!(metrics.offset_for_drag(90.0, 10.0), 40);
    assert_eq!(metrics.offset_for_drag(-10.0, 10.0), 0);
    assert_eq!(metrics.offset_for_drag(300.0, 10.0), 80);
}

#[test]
fn minimum_height_thumb_uses_its_remaining_travel_for_dragging() {
    let metrics = ScrollbarMetrics::new(200.0, 2000, 20);
    assert_eq!(metrics.thumb_height, 20.0);
    assert_eq!(metrics.thumb_top(990), 90.0);
    assert_eq!(metrics.thumb_top(1980), 180.0);
    assert_eq!(metrics.offset_for_drag(95.0, 5.0), 990);
    assert_eq!(metrics.offset_for_drag(185.0, 5.0), 1980);
}

#[test]
fn thumb_filling_the_track_has_no_drag_travel() {
    let metrics = ScrollbarMetrics::new(20.0, 2000, 20);
    assert_eq!(metrics.thumb_height, 20.0);
    assert_eq!(metrics.thumb_top(1980), 0.0);
    assert_eq!(metrics.offset_for_drag(5.0, 5.0), 0);
}
