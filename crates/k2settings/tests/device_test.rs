//! Integration tests for `k2settings::device`.
//!
//! Step 3.1 verification: `cargo test -p k2settings device_test`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use k2settings::device::{count, find_by_alias, list_devices, DEVICES};

#[test]
fn device_test_count_matches_c() {
    // C `devprofiles_count()` returns 23 (all entries before sentinel).
    assert_eq!(count(), 23);
    assert_eq!(DEVICES.len(), 23);
}

#[test]
fn device_test_find_by_alias_exact() {
    let dev = find_by_alias("kpw").expect("kpw should resolve");
    assert_eq!(dev.name, "Kindle Paperwhite");
    assert_eq!(dev.alias, "kpw");
    assert_eq!(dev.width, 658);
    assert_eq!(dev.height, 889);
    assert_eq!(dev.dpi, 212);
    assert_eq!(dev.color, 0);
    assert_eq!(dev.mark_corners, 1);
    assert_eq!(dev.padding, [0, 0, 3, 4]);
}

#[test]
fn device_test_find_by_name_exact() {
    let dev = find_by_alias("Kindle DX").expect("exact name match");
    assert_eq!(dev.alias, "dx");
    assert_eq!(dev.width, 800);
    assert_eq!(dev.height, 1180);
}

#[test]
fn device_test_case_insensitive() {
    assert_eq!(find_by_alias("KPW").unwrap().alias, "kpw");
    assert_eq!(find_by_alias("kindle paperwhite").unwrap().alias, "kpw");
    assert_eq!(find_by_alias("Kindle dx").unwrap().alias, "dx");
}

#[test]
fn device_test_no_match() {
    assert!(find_by_alias("nonexistent_device_xyz").is_none());
}

#[test]
fn device_test_ambiguous_partial() {
    // "kindle" matches multiple devices -> None
    assert!(find_by_alias("kindle").is_none());
    // "kobo" matches multiple -> None
    assert!(find_by_alias("kobo").is_none());
}

#[test]
fn device_test_unique_partial() {
    assert_eq!(find_by_alias("nexus").unwrap().alias, "nex7");
    assert_eq!(find_by_alias("aura one").unwrap().alias, "kao");
}

#[test]
fn device_test_all_23_fields() {
    // Verify every device's fields match C source lines 31-70 of devprofile.c.
    let checks: &[(&str, u16, u16, u16, u8, u8, [u8; 4])] = &[
        ("k2", 560, 735, 167, 0, 1, [0, 0, 3, 4]),
        ("dx", 800, 1180, 167, 0, 1, [0, 0, 0, 0]),
        ("kpw", 658, 889, 212, 0, 1, [0, 0, 3, 4]),
        ("kp2", 718, 965, 212, 0, 1, [0, 0, 3, 4]),
        ("kp3", 1016, 1364, 300, 0, 1, [0, 0, 3, 4]),
        ("kv", 1016, 1364, 300, 0, 1, [0, 0, 3, 4]),
        ("ko2", 1200, 1583, 300, 0, 1, [0, 0, 3, 4]),
        ("pb2", 600, 800, 167, 0, 1, [0, 0, 3, 4]),
        ("nookst", 552, 725, 167, 0, 1, [0, 0, 0, 0]),
        ("kbt", 600, 730, 167, 0, 1, [0, 0, 3, 4]),
        ("kbg", 758, 942, 213, 0, 1, [0, 0, 3, 4]),
        ("kghd", 1072, 1328, 250, 0, 1, [0, 0, 3, 4]),
        ("kghdfs", 1072, 1448, 250, 0, 1, [0, 0, 3, 4]),
        ("kbm", 600, 730, 200, 0, 1, [0, 0, 3, 4]),
        ("kba", 758, 932, 211, 0, 1, [0, 0, 3, 4]),
        ("kbhd", 1080, 1320, 250, 0, 1, [0, 0, 3, 4]),
        ("kbh2o", 1080, 1309, 265, 0, 1, [0, 0, 3, 4]),
        ("kbh2ofs", 1080, 1429, 265, 0, 1, [0, 0, 3, 4]),
        ("kao", 1404, 1713, 300, 0, 0, [0, 0, 3, 4]),
        ("koc", 1072, 1317, 300, 0, 0, [0, 0, 3, 4]),
        ("kof", 1440, 1745, 300, 0, 0, [0, 0, 3, 4]),
        ("kol", 1264, 1527, 300, 0, 0, [0, 0, 3, 4]),
        ("nex7", 1187, 1811, 323, 1, 1, [0, 0, 3, 4]),
    ];
    assert_eq!(checks.len(), 23, "must cover all 23 devices");

    for (alias, w, h, d, c, mc, pad) in checks {
        let dev = find_by_alias(alias).unwrap_or_else(|| panic!("alias '{alias}' not found"));
        assert_eq!(dev.width, *w, "{alias} width");
        assert_eq!(dev.height, *h, "{alias} height");
        assert_eq!(dev.dpi, *d, "{alias} dpi");
        assert_eq!(dev.color, *c, "{alias} color");
        assert_eq!(dev.mark_corners, *mc, "{alias} mark_corners");
        assert_eq!(dev.padding, *pad, "{alias} padding");
    }
}

#[test]
fn device_test_list_devices_output() {
    let text = list_devices();
    assert!(text.starts_with("\nAvailable devices:\n"));
    assert!(text.contains("Kindle Paperwhite"));
    assert!(text.contains("kpw"));
    assert!(text.contains("Nexus 7"));
    assert!(text.contains("658 x 889"));
    assert!(text.contains("212 dpi"));
}

#[test]
fn device_test_special_cases() {
    // Nexus 7 is the only color device
    let nex7 = find_by_alias("nex7").unwrap();
    assert_eq!(nex7.color, 1);

    // Kobo Aura One / Clara HD / Forma / Libra have mark_corners=0
    for alias in ["kao", "koc", "kof", "kol"] {
        let dev = find_by_alias(alias).unwrap();
        assert_eq!(dev.mark_corners, 0, "{alias} mark_corners should be 0");
    }

    // Kindle DX and Nook ST have all-zero padding
    for alias in ["dx", "nookst"] {
        let dev = find_by_alias(alias).unwrap();
        assert_eq!(
            dev.padding,
            [0, 0, 0, 0],
            "{alias} padding should be all zero"
        );
    }
}
