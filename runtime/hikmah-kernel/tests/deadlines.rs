use hikmah_kernel::trace::parse_deadline;

#[test]
fn deadlines_are_parsed_and_validated() {
    assert_eq!(
        parse_deadline("1700000000000", 0).unwrap(),
        1_700_000_000_000
    );
    assert_eq!(parse_deadline("+2h", 1_000).unwrap(), 1_000 + 7_200_000);
    assert_eq!(
        parse_deadline("2024-02-29T12:00:00Z", 0).unwrap(),
        1_709_208_000_000
    );
    assert_eq!(parse_deadline("1970-01-01", 0).unwrap(), 0);
    for bad in [
        "2026-02-31",
        "2025-02-29",
        "2026-04-31",
        "2026-01-01T-5:-30",
        "600000000-01-01",
        "1969-12-31",
        "+5ä",
        "+ä",
        "+5w",
        "",
        "tomorrow",
    ] {
        assert!(parse_deadline(bad, 0).is_err(), "{bad}");
    }
}
