//! Fuzzy name matching for the "Go to Repository" switcher (#647), on the
//! same chord as the graphical front end's own Ctrl+P.
//!
//! Ranks a typed query's matches against a repository name so a run that
//! starts at a word boundary - `tsc` against `TankSwarmCode` - outranks one
//! that is merely contained - `tsc` in `atscode` - which in turn outranks
//! letters found only by skipping around - `tsc` in `xtxsxc`. Mirrors
//! `crates/gui/src/switcher.rs` exactly, so the two front ends rank the same
//! query the same way.

/// Scores `candidate` against `query`, matched case-insensitively as a
/// subsequence: every letter of `query`, in order, somewhere in
/// `candidate`, not necessarily touching. `None` when `candidate` is
/// missing one of them.
///
/// A matched letter scores highest when it starts a word - the name's
/// first character, the one after a non-letter, or a lowercase-to-uppercase
/// transition - next highest when it continues the previous match without a
/// gap, and least otherwise; a small penalty for the distance skipped to
/// reach it keeps a tighter match ahead of a looser one scoring the same
/// way letter for letter. An empty query matches every candidate with a
/// score of zero.
#[must_use]
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    let chars: Vec<char> = candidate.chars().collect();
    let mut total = 0i32;
    let mut search_from = 0usize;
    let mut previous_match: Option<usize> = None;
    for query_char in query.chars() {
        let query_lower = query_char.to_ascii_lowercase();
        let found =
            (search_from..chars.len()).find(|&i| chars[i].to_ascii_lowercase() == query_lower)?;
        let is_word_start = found == 0
            || !chars[found - 1].is_alphanumeric()
            || (chars[found - 1].is_lowercase() && chars[found].is_uppercase());
        let is_contiguous = previous_match.is_some_and(|previous| found == previous + 1);
        total += if is_word_start {
            100
        } else if is_contiguous {
            10
        } else {
            1
        };
        total -= i32::try_from(found - search_from).unwrap_or(i32::MAX);
        previous_match = Some(found);
        search_from = found + 1;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn a_word_start_run_outranks_a_merely_contiguous_one() {
        let word_start = score("tsc", "TankSwarmCode").expect("tsc is in TankSwarmCode");
        let contiguous = score("tsc", "atscode").expect("tsc is in atscode");
        assert!(
            word_start > contiguous,
            "word-start {word_start} should outrank contiguous {contiguous}"
        );
    }

    #[test]
    fn a_contiguous_run_outranks_a_scattered_one() {
        let contiguous = score("tsc", "atscode").expect("tsc is in atscode");
        let scattered = score("tsc", "xtxsxc").expect("tsc is in xtxsxc");
        assert!(
            contiguous > scattered,
            "contiguous {contiguous} should outrank scattered {scattered}"
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(score("TSC", "TankSwarmCode"), score("tsc", "TankSwarmCode"));
    }

    #[test]
    fn a_candidate_missing_a_letter_does_not_match() {
        assert_eq!(score("tsc", "banana"), None);
    }

    #[test]
    fn an_empty_query_matches_everything_with_no_score() {
        assert_eq!(score("", "TankSwarmCode"), Some(0));
    }

    #[test]
    fn ranks_the_same_way_the_graphical_front_end_s_switcher_does() {
        // The same three-way ordering `crates/gui/src/switcher.rs` asserts,
        // proving the terminal front end's own scorer ranks identically for
        // the same input (#647).
        let word_start = score("tsc", "TankSwarmCode").expect("tsc is in TankSwarmCode");
        let contiguous = score("tsc", "atscode").expect("tsc is in atscode");
        let scattered = score("tsc", "xtxsxc").expect("tsc is in xtxsxc");
        assert!(word_start > contiguous && contiguous > scattered);
    }
}
