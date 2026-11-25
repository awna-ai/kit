use awna_intake::Status;

#[test]
fn every_status_pair_matches_the_contract() {
    let expected = [
        [false, true, false, true, false, false, false, false, true],
        [false, false, true, true, false, false, false, false, true],
        [false, false, false, false, true, false, false, false, true],
        [false, false, true, false, false, false, false, true, true],
        [false, false, true, false, false, true, false, false, true],
        [false, false, false, false, false, false, true, false, false],
        [false; 9],
        [false; 9],
        [false; 9],
    ];
    for (row, from) in Status::ALL.into_iter().enumerate() {
        for (column, to) in Status::ALL.into_iter().enumerate() {
            assert_eq!(
                from.can_move_to(to),
                expected[row][column],
                "{from} -> {to}"
            );
            assert_eq!(from.check_move(to).is_ok(), expected[row][column]);
        }
        assert_eq!(from.is_terminal(), row >= 6);
        assert_eq!(from.as_str().parse::<Status>().unwrap(), from);
        assert_eq!(from.to_string(), from.as_str());
    }
    assert!("IN_PROGRESS".parse::<Status>().is_err());
}
