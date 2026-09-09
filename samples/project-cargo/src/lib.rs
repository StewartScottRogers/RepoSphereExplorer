//! The crate `Cargo.toml` beside this file declares.
//!
//! A manifest naming a package that has no source would be a manifest for
//! a crate that cannot build, and this sample set holds working files.

/// One line of an instrument log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Milliseconds since the run began.
    pub elapsed_ms: u64,
    /// The channel the reading came from.
    pub channel: String,
    /// The reading itself, in the channel's own units.
    pub value: i32,
}

/// Parses `line` in the form `<elapsed>,<channel>,<value>`.
#[must_use]
pub fn parse(line: &str) -> Option<Entry> {
    let mut fields = line.split(',');
    Some(Entry {
        elapsed_ms: fields.next()?.trim().parse().ok()?,
        channel: fields.next()?.trim().to_owned(),
        value: fields.next()?.trim().parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_a_well_formed_line() {
        let entry = parse("1200, pressure, -47").unwrap();
        assert_eq!(entry.elapsed_ms, 1200);
        assert_eq!(entry.channel, "pressure");
        assert_eq!(entry.value, -47);
    }

    #[test]
    fn refuses_a_line_that_is_missing_a_field() {
        assert!(parse("1200, pressure").is_none());
    }
}
