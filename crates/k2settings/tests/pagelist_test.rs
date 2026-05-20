//! Integration tests for pagelist parser — Step 3.2
//!
//! Covers: empty, single, range, open-ended, reverse, even/odd, cover,
//! comma separation, spaces/tabs, total_pages cap, duplicates, edge cases.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use k2settings::pagelist::{includes_cover, includes_page, is_valid, parse};

// ---- basic parsing ----

#[test]
fn pagelist_test_empty_returns_all_pages() {
    let pages = parse("", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
}

#[test]
fn pagelist_test_single_page() {
    let pages = parse("5", 10).unwrap();
    assert_eq!(pages, vec![5]);
}

#[test]
fn pagelist_test_range() {
    let pages = parse("1-5", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5]);
}

#[test]
fn pagelist_test_open_ended_range() {
    let pages = parse("5-", 10).unwrap();
    assert_eq!(pages, vec![5, 6, 7, 8, 9, 10]);
}

#[test]
fn pagelist_test_reverse_range() {
    let pages = parse("5-1", 10).unwrap();
    assert_eq!(pages, vec![5, 4, 3, 2, 1]);
}

#[test]
fn pagelist_test_hyphen_prefix_range() {
    // "-5" means pages 1 through 5
    let pages = parse("-5", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5]);
}

#[test]
fn pagelist_test_hyphen_only_means_all_pages() {
    let pages = parse("-", 5).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5]);
}

// ---- even / odd ----

#[test]
fn pagelist_test_even_only() {
    let pages = parse("e", 10).unwrap();
    assert_eq!(pages, vec![2, 4, 6, 8, 10]);
}

#[test]
fn pagelist_test_odd_only() {
    let pages = parse("o", 10).unwrap();
    assert_eq!(pages, vec![1, 3, 5, 7, 9]);
}

#[test]
fn pagelist_test_even_range_suffix() {
    // "1-5e" = even pages in range 1-5 -> 2, 4
    let pages = parse("1-5e", 10).unwrap();
    assert_eq!(pages, vec![2, 4]);
}

#[test]
fn pagelist_test_odd_range_suffix() {
    // "1-5o" = odd pages in range 1-5 -> 1, 3, 5
    let pages = parse("1-5o", 10).unwrap();
    assert_eq!(pages, vec![1, 3, 5]);
}

#[test]
fn pagelist_test_even_prefix_with_range() {
    // "e1-5" -> even modifier on range 1-5
    let pages = parse("e1-5", 10).unwrap();
    assert_eq!(pages, vec![2, 4]);
}

#[test]
fn pagelist_test_odd_prefix_with_range() {
    let pages = parse("o1-5", 10).unwrap();
    assert_eq!(pages, vec![1, 3, 5]);
}

#[test]
fn pagelist_test_even_suffix_on_single_page_matching() {
    // "2e" -> page 2 if even -> 2
    let pages = parse("2e", 10).unwrap();
    assert_eq!(pages, vec![2]);
}

#[test]
fn pagelist_test_even_suffix_on_single_page_not_matching() {
    // "3e" -> page 3 if even -> not even, empty
    let pages = parse("3e", 10).unwrap();
    assert!(pages.is_empty());
}

#[test]
fn pagelist_test_odd_suffix_on_single_page_matching() {
    // "3o" -> page 3 if odd -> 3
    let pages = parse("3o", 10).unwrap();
    assert_eq!(pages, vec![3]);
}

#[test]
fn pagelist_test_odd_suffix_on_single_page_not_matching() {
    // "2o" -> page 2 if odd -> not odd, empty
    let pages = parse("2o", 10).unwrap();
    assert!(pages.is_empty());
}

// ---- combined / complex ----

#[test]
fn pagelist_test_multiple_comma_separated() {
    let pages = parse("1-5,7,10-", 12).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5, 7, 10, 11, 12]);
}

#[test]
fn pagelist_test_complex_combined() {
    // "1-5e,7,10-o" -> even 1-5 + page 7 + odd 10-12
    let pages = parse("1-5e,7,10-o", 12).unwrap();
    assert_eq!(pages, vec![2, 4, 7, 11]);
}

#[test]
fn pagelist_test_overlapping_ranges_produce_duplicates() {
    // "1-5,3-7" -> [1,2,3,4,5,3,4,5,6,7] — no dedup (matches C)
    let pages = parse("1-5,3-7", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5, 3, 4, 5, 6, 7]);
}

#[test]
fn pagelist_test_reverse_even_range() {
    // "5-1e" -> even pages from 5 down to 1 -> 4, 2
    let pages = parse("5-1e", 10).unwrap();
    assert_eq!(pages, vec![4, 2]);
}

#[test]
fn pagelist_test_reverse_odd_range() {
    // "5-1o" -> odd pages from 5 down to 1 -> 5, 3, 1
    let pages = parse("5-1o", 10).unwrap();
    assert_eq!(pages, vec![5, 3, 1]);
}

#[test]
fn pagelist_test_even_open_ended() {
    // "5-e" -> even pages from 5 to total_pages
    let pages = parse("5-e", 10).unwrap();
    assert_eq!(pages, vec![6, 8, 10]);
}

#[test]
fn pagelist_test_odd_open_ended() {
    // "5-o" -> odd pages from 5 to total_pages
    let pages = parse("5-o", 10).unwrap();
    assert_eq!(pages, vec![5, 7, 9]);
}

#[test]
fn pagelist_test_parity_suffix_on_end_number() {
    // "1-5e" — parity after end number
    let pages = parse("1-5e", 10).unwrap();
    assert_eq!(pages, vec![2, 4]);
}

#[test]
fn pagelist_test_parity_last_wins() {
    // "1e-5o" -> odd wins (last parity seen) -> odd pages 1-5 -> 1,3,5
    let pages = parse("1e-5o", 10).unwrap();
    assert_eq!(pages, vec![1, 3, 5]);
}

// ---- total_pages cap ----

#[test]
fn pagelist_test_page_beyond_total_pages_filtered() {
    let pages = parse("1-100", 5).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5]);
}

#[test]
fn pagelist_test_page_zero_filtered() {
    // page 0 is not valid (pages are 1-based)
    let pages = parse("0", 10).unwrap();
    assert!(pages.is_empty());
}

#[test]
fn pagelist_test_total_pages_zero() {
    let pages = parse("", 0).unwrap();
    assert!(pages.is_empty());
    let pages2 = parse("1-5", 0).unwrap();
    assert!(pages2.is_empty());
}

#[test]
fn pagelist_test_total_pages_one() {
    let pages = parse("", 1).unwrap();
    assert_eq!(pages, vec![1]);
    let pages2 = parse("1", 1).unwrap();
    assert_eq!(pages2, vec![1]);
    let pages3 = parse("e", 1).unwrap();
    assert!(pages3.is_empty()); // page 1 is odd, not even
    let pages4 = parse("o", 1).unwrap();
    assert_eq!(pages4, vec![1]);
}

// ---- whitespace / separators ----

#[test]
fn pagelist_test_with_spaces() {
    let pages = parse(" 1 - 5 , 7 ", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5, 7]);
}

#[test]
fn pagelist_test_with_tabs() {
    let pages = parse("1\t-\t5", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5]);
}

#[test]
fn pagelist_test_trailing_comma() {
    let pages = parse("1-5,", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3, 4, 5]);
}

#[test]
fn pagelist_test_multiple_commas() {
    let pages = parse("1,,3", 10).unwrap();
    assert_eq!(pages, vec![1, 3]);
}

#[test]
fn pagelist_test_only_commas_and_spaces() {
    let pages = parse(", , ,", 10).unwrap();
    assert!(pages.is_empty());
}

// ---- cover page ----

#[test]
fn pagelist_test_cover_in_list() {
    // 'c' is parsed as Cover, not expanded into page numbers
    let pages = parse("c,1-3", 10).unwrap();
    assert_eq!(pages, vec![1, 2, 3]);
    assert!(includes_cover("c,1-3"));
}

#[test]
fn pagelist_test_includes_cover_with_c() {
    assert!(includes_cover("c"));
    assert!(includes_cover("C"));
    assert!(includes_cover("c,1-5"));
}

#[test]
fn pagelist_test_includes_cover_without_c() {
    assert!(!includes_cover("1-5"));
    assert!(!includes_cover("e,o"));
}

// ---- is_valid ----

#[test]
fn pagelist_test_is_valid_normal_input() {
    assert!(is_valid("1-5,7,10-"));
    assert!(is_valid("e,o"));
    assert!(is_valid("1-5e"));
}

#[test]
fn pagelist_test_is_valid_rejects_unknown_chars() {
    assert!(!is_valid("1-5x"));
    assert!(!is_valid("abc"));
    assert!(!is_valid("r1-3")); // 'r' not valid (C has no 'r' syntax)
}

#[test]
fn pagelist_test_is_valid_empty_string() {
    assert!(is_valid(""));
}

// ---- utility functions ----

#[test]
fn pagelist_test_count_matches_parse_length() {
    assert_eq!(k2settings::pagelist_count("1-5", 10), 5);
    assert_eq!(k2settings::pagelist_count("", 10), 10);
    assert_eq!(k2settings::pagelist_count("e", 10), 5);
}

#[test]
fn pagelist_test_includes_page_true() {
    assert!(includes_page("1-5", 3, 10));
    assert!(includes_page("e", 4, 10));
}

#[test]
fn pagelist_test_includes_page_false() {
    assert!(!includes_page("1-5", 7, 10));
    assert!(!includes_page("e", 3, 10));
}

// ---- overflow / error ----

#[test]
fn pagelist_test_overflow_page_number() {
    let result = parse("99999999999", 10);
    assert!(result.is_err());
}

// ---- large range ----

#[test]
fn pagelist_test_large_range_efficient() {
    let pages = parse("1-1000", 1000).unwrap();
    assert_eq!(pages.len(), 1000);
    assert_eq!(pages[0], 1);
    assert_eq!(pages[999], 1000);
}

#[test]
fn pagelist_test_even_large_range() {
    let pages = parse("1-100e", 100).unwrap();
    assert_eq!(pages.len(), 50); // 50 even pages in 1-100
    assert_eq!(pages[0], 2);
    assert_eq!(pages[49], 100);
}

// ---- uppercase ----

#[test]
fn pagelist_test_uppercase_even_odd() {
    let pages_e = parse("E", 10).unwrap();
    assert_eq!(pages_e, vec![2, 4, 6, 8, 10]);
    let pages_o = parse("O", 10).unwrap();
    assert_eq!(pages_o, vec![1, 3, 5, 7, 9]);
    let pages_range = parse("1-5E", 10).unwrap();
    assert_eq!(pages_range, vec![2, 4]);
}
