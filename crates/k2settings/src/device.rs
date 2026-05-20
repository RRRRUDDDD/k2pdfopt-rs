//! E-reader device profile table, 1:1 ported from C `devprofile.c` (166 lines).
//!
//! Source: `k2pdfoptlib/devprofile.c` lines 29-72 (23 profiles + sentinel).
//! See also: `k2pdfopt.h` lines 770-780 (DEVPROFILE struct).

/// E-reader device profile, mirroring C `DEVPROFILE`.
///
/// # C struct fields
/// ```c
/// typedef struct {
///     char *name;          // Human-readable name, e.g. "Kindle Paperwhite"
///     char *alias;         // Short alias, e.g. "kpw"
///     int   width;         // Screen width in pixels
///     int   height;        // Screen height in pixels
///     int   dpi;           // Display DPI
///     int   color;         // 0 = grayscale, 1 = color
///     int   mark_corners;  // 1 = mark corners on output
///     int   padding[4];    // Left, top, right, bottom padding in pixels
/// } DEVPROFILE;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceProfile {
    /// Human-readable name.
    pub name: &'static str,
    /// Short CLI alias.
    pub alias: &'static str,
    /// Screen width in pixels.
    pub width: u16,
    /// Screen height in pixels.
    pub height: u16,
    /// Display DPI.
    pub dpi: u16,
    /// 0 = grayscale, 1 = color screen.
    pub color: u8,
    /// 1 = mark corners on output pages.
    pub mark_corners: u8,
    /// Padding [left, top, right, bottom] in pixels.
    pub padding: [u8; 4],
}

/// All 23 device profiles from C `devprof[]` (lines 31-70 of devprofile.c).
///
/// Sentinel entry from C (`{"", "", 0, ...}`) is omitted; use [`count`] for length.
pub const DEVICES: &[DeviceProfile] = &[
    // devprofile.c line 31
    DeviceProfile {
        name: "Kindle 1-5",
        alias: "k2",
        width: 560,
        height: 735,
        dpi: 167,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 32
    DeviceProfile {
        name: "Kindle DX",
        alias: "dx",
        width: 800,
        height: 1180,
        dpi: 167,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 0, 0],
    },
    // line 34 — v2.02: Changed kpw to 658 x 889
    DeviceProfile {
        name: "Kindle Paperwhite",
        alias: "kpw",
        width: 658,
        height: 889,
        dpi: 212,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 39 — PW2 dims from Doitsu's screenshots = 718 x 964
    DeviceProfile {
        name: "Kindle Paperwhite 2",
        alias: "kp2",
        width: 718,
        height: 965,
        dpi: 212,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 41 — PW3 (released Summer 2015) = 1016 x 1364
    DeviceProfile {
        name: "Kindle Paperwhite 3",
        alias: "kp3",
        width: 1016,
        height: 1364,
        dpi: 300,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 45 — Voyage dims = 1016 x 1364
    DeviceProfile {
        name: "Kindle Voyage/PW3+/Oasis",
        alias: "kv",
        width: 1016,
        height: 1364,
        dpi: 300,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 47 — Kindle Oasis 2nd-gen released Oct 2017
    DeviceProfile {
        name: "Kindle Oasis 2",
        alias: "ko2",
        width: 1200,
        height: 1583,
        dpi: 300,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 49 — Pocketbook Basic 2 = 600 x 800
    DeviceProfile {
        name: "Pocketbook Basic 2",
        alias: "pb2",
        width: 600,
        height: 800,
        dpi: 167,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 50
    DeviceProfile {
        name: "Nook Simple Touch",
        alias: "nookst",
        width: 552,
        height: 725,
        dpi: 167,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 0, 0],
    },
    // line 52
    DeviceProfile {
        name: "Kobo Touch",
        alias: "kbt",
        width: 600,
        height: 730,
        dpi: 167,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 53
    DeviceProfile {
        name: "Kobo Glo",
        alias: "kbg",
        width: 758,
        height: 942,
        dpi: 213,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 55 — Kobo Glo HD
    DeviceProfile {
        name: "Kobo Glo HD",
        alias: "kghd",
        width: 1072,
        height: 1328,
        dpi: 250,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 56
    DeviceProfile {
        name: "Kobo Glo HD Full Screen",
        alias: "kghdfs",
        width: 1072,
        height: 1448,
        dpi: 250,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 58 — v2.13: Added Kobo mini
    DeviceProfile {
        name: "Kobo Mini",
        alias: "kbm",
        width: 600,
        height: 730,
        dpi: 200,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 59
    DeviceProfile {
        name: "Kobo Aura",
        alias: "kba",
        width: 758,
        height: 932,
        dpi: 211,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 60
    DeviceProfile {
        name: "Kobo Aura HD",
        alias: "kbhd",
        width: 1080,
        height: 1320,
        dpi: 250,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 61
    DeviceProfile {
        name: "Kobo H2O",
        alias: "kbh2o",
        width: 1080,
        height: 1309,
        dpi: 265,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 62
    DeviceProfile {
        name: "Kobo H2O Full Screen",
        alias: "kbh2ofs",
        width: 1080,
        height: 1429,
        dpi: 265,
        color: 0,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
    // line 63
    DeviceProfile {
        name: "Kobo Aura One",
        alias: "kao",
        width: 1404,
        height: 1713,
        dpi: 300,
        color: 0,
        mark_corners: 0,
        padding: [0, 0, 3, 4],
    },
    // line 65 — v2.52
    DeviceProfile {
        name: "Kobo Clara HD",
        alias: "koc",
        width: 1072,
        height: 1317,
        dpi: 300,
        color: 0,
        mark_corners: 0,
        padding: [0, 0, 3, 4],
    },
    // line 66
    DeviceProfile {
        name: "Kobo Forma",
        alias: "kof",
        width: 1440,
        height: 1745,
        dpi: 300,
        color: 0,
        mark_corners: 0,
        padding: [0, 0, 3, 4],
    },
    // line 67
    DeviceProfile {
        name: "Kobo Libra H2O",
        alias: "kol",
        width: 1264,
        height: 1527,
        dpi: 300,
        color: 0,
        mark_corners: 0,
        padding: [0, 0, 3, 4],
    },
    // line 70 — Nexus 7
    DeviceProfile {
        name: "Nexus 7",
        alias: "nex7",
        width: 1187,
        height: 1811,
        dpi: 323,
        color: 1,
        mark_corners: 1,
        padding: [0, 0, 3, 4],
    },
];

/// Number of device profiles (mirrors C `devprofiles_count()`).
pub const fn count() -> usize {
    DEVICES.len()
}

/// Find a device profile by name or alias (case-insensitive).
///
/// Mirrors C `devprofile_get()`: exact match preferred; if no exact match,
/// returns the sole partial (substring) match. Returns `None` if zero or
/// multiple partial matches.
pub fn find_by_alias(name: &str) -> Option<&'static DeviceProfile> {
    let lower = name.to_ascii_lowercase();
    let mut partial: Option<&DeviceProfile> = None;
    let mut partial_count: usize = 0;

    for dev in DEVICES {
        let name_lower = dev.name.to_ascii_lowercase();
        let alias_lower = dev.alias.to_ascii_lowercase();

        if name_lower == lower || alias_lower == lower {
            return Some(dev);
        }

        if name_lower.contains(&lower) || alias_lower.contains(&lower) {
            partial_count += 1;
            partial = Some(dev);
        }
    }

    if partial_count == 1 {
        partial
    } else {
        None
    }
}

/// Format the full device list for `--list-devices` output.
///
/// Mirrors C `devprofiles_echo()`.
pub fn list_devices() -> String {
    let mut out = String::from("\nAvailable devices:\n");
    for dev in DEVICES {
        out.push_str(&format!(
            "    {} (alias {}): {} x {}, {} dpi\n",
            dev.name, dev.alias, dev.width, dev.height, dev.dpi
        ));
        out.push_str(&format!(
            "        Mark corners={}, Padding (l,t,r,b)={},{},{},{}\n\n",
            dev.mark_corners, dev.padding[0], dev.padding[1], dev.padding[2], dev.padding[3]
        ));
    }
    out.push('\n');
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]
mod tests {
    use super::*;

    #[test]
    fn device_count_matches_c() {
        // C `devprofiles_count()` returns 23 (all entries before sentinel).
        assert_eq!(count(), 23);
    }

    #[test]
    fn find_by_exact_alias() {
        let dev = find_by_alias("kpw").expect("kpw should resolve");
        assert_eq!(dev.name, "Kindle Paperwhite");
        assert_eq!(dev.width, 658);
        assert_eq!(dev.height, 889);
        assert_eq!(dev.dpi, 212);
    }

    #[test]
    fn find_by_exact_name() {
        let dev = find_by_alias("Kindle DX").expect("exact name should resolve");
        assert_eq!(dev.alias, "dx");
        assert_eq!(dev.width, 800);
        assert_eq!(dev.height, 1180);
    }

    #[test]
    fn find_case_insensitive() {
        let dev = find_by_alias("KPW").expect("case-insensitive alias");
        assert_eq!(dev.name, "Kindle Paperwhite");
        let dev2 = find_by_alias("kindle paperwhite").expect("case-insensitive name");
        assert_eq!(dev2.alias, "kpw");
    }

    #[test]
    fn find_no_match_returns_none() {
        assert_eq!(find_by_alias("nonexistent_device_xyz"), None);
    }

    #[test]
    fn find_ambiguous_partial_returns_none() {
        // "kindle" is a substring of many device names → ambiguous
        assert_eq!(find_by_alias("kindle"), None);
    }

    #[test]
    fn find_unique_partial() {
        // "nexus" appears only in "Nexus 7"
        let dev = find_by_alias("nexus").expect("unique partial match");
        assert_eq!(dev.alias, "nex7");
    }

    #[test]
    fn all_fields_match_c_source() {
        // Spot-check every device's key fields against C source lines 31-70.
        // This also catches typos in the const table.
        let checks: &[(&str, u16, u16, u16, u8, u8, [u8; 4])] = &[
            // (alias, width, height, dpi, color, mark_corners, padding)
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

        assert_eq!(checks.len(), count(), "check table must cover all devices");

        for (alias, w, h, d, c, mc, pad) in checks {
            let dev = find_by_alias(alias).unwrap_or_else(|| {
                panic!("alias '{alias}' should resolve");
            });
            assert_eq!(dev.width, *w, "{alias} width");
            assert_eq!(dev.height, *h, "{alias} height");
            assert_eq!(dev.dpi, *d, "{alias} dpi");
            assert_eq!(dev.color, *c, "{alias} color");
            assert_eq!(dev.mark_corners, *mc, "{alias} mark_corners");
            assert_eq!(dev.padding, *pad, "{alias} padding");
        }
    }

    #[test]
    fn list_devices_not_empty() {
        let text = list_devices();
        assert!(text.contains("Kindle Paperwhite"));
        assert!(text.contains("kpw"));
        assert!(text.contains("Nexus 7"));
    }
}
