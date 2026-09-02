//! Core library for `RepoSphereExplorer`.
//!
//! The binary is a thin shell over this crate so that every behaviour is
//! testable without spawning a process.

/// Describes the target the explorer has been pointed at.
///
/// This is placeholder behaviour: it exists so the pipeline has something real
/// to build, lint and test end to end.
#[must_use]
pub fn describe(target: &str) -> String {
    format!("RepoSphereExplorer: target `{target}` registered, no explorers implemented yet")
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn describe_mentions_the_target() {
        assert!(describe("acme/widgets").contains("acme/widgets"));
    }
}
