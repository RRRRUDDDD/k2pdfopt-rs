//! Page list parser — ported from C `pagelist.c` (409 lines).
//!
//! Supported syntax (comma-separated tokens):
//! - `5` — single page
//! - `1-5` — pages 1 through 5
//! - `5-` — page 5 to end (total_pages)
//! - `-5` — pages 1 through 5
//! - `5-1` — reverse range (pages 5,4,3,2,1)
//! - `e` / `o` — all even / odd pages
//! - `1-5e` — even pages in range 1-5
//! - `1-5o` — odd pages in range 1-5
//! - `c` — cover page indicator (see [`includes_cover`])
//! - Empty string — all pages

use anyhow::Context;

/// Parity filter for page ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    /// Both even and odd pages (default).
    All,
    /// Even pages only.
    Even,
    /// Odd pages only.
    Odd,
}

/// A parsed item from a page list specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageRangeItem {
    /// A single page number with parity filter.
    Single(u32, Parity),
    /// A page range: start, optional end (None = open-ended to total_pages), parity.
    Range(u32, Option<u32>, Parity),
    /// Cover page indicator "c".
    Cover,
}

/// Check if a string contains only valid page range characters.
/// Matches C `pagelist_valid_page_range`: digits, comma, hyphen, space, tab, 'e', 'o'.
/// Note: 'c' is valid in practice but excluded here to match C behavior.
pub fn is_valid(input: &str) -> bool {
    input.chars().all(|c| {
        c.is_ascii_digit()
            || matches!(c, ',' | '-' | ' ' | '\t')
            || c.eq_ignore_ascii_case(&'e')
            || c.eq_ignore_ascii_case(&'o')
    })
}

/// Check if the page list includes a cover page indicator ('c' or 'C').
/// Matches C `pagelist_includes_page` cover-page logic.
pub fn includes_cover(input: &str) -> bool {
    input.to_ascii_lowercase().contains('c')
}

/// Parse a page list string and return resolved page numbers.
///
/// Pages are 1-based, returned in input order (not deduplicated).
/// `total_pages` resolves open-ended ranges and caps page numbers.
/// Cover page ('c') is not expanded into page numbers; use [`includes_cover`].
pub fn parse(input: &str, total_pages: u32) -> anyhow::Result<Vec<u32>> {
    if input.trim().is_empty() {
        return Ok((1..=total_pages).collect());
    }
    let items = parse_items(input, total_pages)?;
    expand_items(&items, total_pages)
}

/// Count the number of pages in the page list.
pub fn count(input: &str, total_pages: u32) -> usize {
    parse(input, total_pages).map_or(0, |v| v.len())
}

/// Check if a page number is included in the page list.
pub fn includes_page(input: &str, page: u32, total_pages: u32) -> bool {
    parse(input, total_pages).is_ok_and(|p| p.contains(&page))
}

// ---- internal implementation ----

fn parse_items(input: &str, total_pages: u32) -> anyhow::Result<Vec<PageRangeItem>> {
    let lower = input.trim().to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut pos = 0;
    let mut items = Vec::new();

    while pos < bytes.len() {
        skip_sep(&mut pos, bytes);
        if pos >= bytes.len() {
            break;
        }
        let token_start = pos;
        let mut parity = Parity::All;

        // Prefix parity (e/o)
        if bytes[pos] == b'e' || bytes[pos] == b'o' {
            parity = if bytes[pos] == b'e' {
                Parity::Even
            } else {
                Parity::Odd
            };
            pos += 1;
            skip_ws(&mut pos, bytes);
        }

        if pos >= bytes.len() {
            // Trailing e/o → standalone even/odd
            if parity != Parity::All {
                items.push(PageRangeItem::Range(1, Some(total_pages), parity));
            }
            break;
        }

        // Cover page
        if bytes[pos] == b'c' {
            items.push(PageRangeItem::Cover);
            pos += 1;
            continue;
        }

        // Start number
        let (start, has_start) = if bytes[pos].is_ascii_digit() {
            (read_u32(bytes, &mut pos)?, true)
        } else {
            (0u32, false)
        };

        // Suffix parity on start number
        if pos < bytes.len() && (bytes[pos] == b'e' || bytes[pos] == b'o') {
            parity = if bytes[pos] == b'e' {
                Parity::Even
            } else {
                Parity::Odd
            };
            pos += 1;
        }

        skip_ws(&mut pos, bytes);

        // Range separator '-'
        if pos < bytes.len() && bytes[pos] == b'-' {
            pos += 1;
            skip_ws(&mut pos, bytes);

            // Prefix parity on end
            if pos < bytes.len() && (bytes[pos] == b'e' || bytes[pos] == b'o') {
                parity = if bytes[pos] == b'e' {
                    Parity::Even
                } else {
                    Parity::Odd
                };
                pos += 1;
            }

            // End number (None = open-ended)
            let end = if pos < bytes.len() && bytes[pos].is_ascii_digit() {
                Some(read_u32(bytes, &mut pos)?)
            } else {
                None
            };

            // Suffix parity on end number
            if pos < bytes.len() && (bytes[pos] == b'e' || bytes[pos] == b'o') {
                parity = if bytes[pos] == b'e' {
                    Parity::Even
                } else {
                    Parity::Odd
                };
                pos += 1;
            }

            let start_page = if has_start { start } else { 1 };
            items.push(PageRangeItem::Range(start_page, end, parity));
        } else if !has_start && parity != Parity::All {
            // Standalone e/o → all even/odd pages
            items.push(PageRangeItem::Range(1, Some(total_pages), parity));
        } else if has_start {
            items.push(PageRangeItem::Single(start, parity));
        }

        // Fallback: skip unknown char to avoid infinite loop
        if pos == token_start {
            pos += 1;
        }
    }

    Ok(items)
}

fn expand_items(items: &[PageRangeItem], total_pages: u32) -> anyhow::Result<Vec<u32>> {
    let mut pages = Vec::new();
    for item in items {
        match item {
            PageRangeItem::Single(page, parity) => {
                if *page >= 1 && *page <= total_pages && matches_parity(*page, *parity) {
                    pages.push(*page);
                }
            }
            PageRangeItem::Range(start, end, parity) => {
                let end_val = end.unwrap_or(total_pages);
                if *start <= end_val {
                    for p in *start..=end_val {
                        if p >= 1 && p <= total_pages && matches_parity(p, *parity) {
                            pages.push(p);
                        }
                    }
                } else {
                    for p in (end_val..=*start).rev() {
                        if p >= 1 && p <= total_pages && matches_parity(p, *parity) {
                            pages.push(p);
                        }
                    }
                }
            }
            PageRangeItem::Cover => {}
        }
    }
    Ok(pages)
}

fn matches_parity(page: u32, parity: Parity) -> bool {
    match parity {
        Parity::All => true,
        Parity::Even => page % 2 == 0,
        Parity::Odd => page % 2 == 1,
    }
}

fn skip_sep(pos: &mut usize, bytes: &[u8]) {
    while *pos < bytes.len() && (bytes[*pos] == b' ' || bytes[*pos] == b'\t' || bytes[*pos] == b',')
    {
        *pos += 1;
    }
}

fn skip_ws(pos: &mut usize, bytes: &[u8]) {
    while *pos < bytes.len() && (bytes[*pos] == b' ' || bytes[*pos] == b'\t') {
        *pos += 1;
    }
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> anyhow::Result<u32> {
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
        *pos += 1;
    }
    let s = std::str::from_utf8(&bytes[start..*pos]).context("invalid UTF-8")?;
    s.parse::<u32>().context("page number overflow")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parity_all() {
        assert!(matches_parity(1, Parity::All));
        assert!(matches_parity(2, Parity::All));
    }

    #[test]
    fn parity_even() {
        assert!(!matches_parity(1, Parity::Even));
        assert!(matches_parity(2, Parity::Even));
        assert!(matches_parity(10, Parity::Even));
    }

    #[test]
    fn parity_odd() {
        assert!(matches_parity(1, Parity::Odd));
        assert!(!matches_parity(2, Parity::Odd));
        assert!(matches_parity(9, Parity::Odd));
    }

    #[test]
    fn is_valid_empty() {
        assert!(is_valid(""));
    }

    #[test]
    fn is_valid_normal() {
        assert!(is_valid("1-5,7,e,o"));
        assert!(is_valid("1-5e,2o"));
    }

    #[test]
    fn is_valid_rejects_unknown() {
        assert!(!is_valid("1-5x"));
        assert!(!is_valid("abc"));
    }

    #[test]
    fn includes_cover_check() {
        assert!(includes_cover("c"));
        assert!(includes_cover("c,1-5"));
        assert!(includes_cover("1-5,C"));
        assert!(!includes_cover("1-5"));
        assert!(!includes_cover("e,o"));
    }

    #[test]
    fn read_u32_normal() {
        let bytes = b"123abc";
        let mut pos = 0;
        assert_eq!(read_u32(bytes, &mut pos).unwrap(), 123);
        assert_eq!(pos, 3);
    }

    #[test]
    fn read_u32_overflow() {
        let bytes = b"99999999999";
        let mut pos = 0;
        assert!(read_u32(bytes, &mut pos).is_err());
    }
}
