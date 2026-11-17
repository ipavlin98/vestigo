use super::*;

#[test]
fn a_maximized_dialog_gets_the_bar_on_its_bottom_edge_and_the_list_over_it() {
    assert_eq!(dialog_side(Room { below: 0.0, above: 0.0 }), (Side::Over, 8));
}

#[test]
fn a_dialog_without_room_for_one_row_on_either_side_gets_the_list_over_it() {
    assert_eq!(dialog_side(Room { below: 100.0, above: 60.0 }), (Side::Over, 8));
    assert_eq!(dialog_side(Room { below: 60.0, above: 100.0 }), (Side::Over, 8));
}

#[test]
fn a_dialog_with_room_beside_it_keeps_the_list_beside_it() {
    assert_eq!(dialog_side(Room { below: 116.0, above: 50.0 }), (Side::Below, 1));
    assert_eq!(dialog_side(Room { below: 200.0, above: 500.0 }), (Side::Above, 8));
    assert_eq!(dialog_side(Room { below: 400.0, above: 900.0 }), (Side::Below, 8));
}
