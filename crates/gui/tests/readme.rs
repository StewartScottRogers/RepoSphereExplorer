//! The README opens with the repository history film.
//!
//! The owner asked for "Repository history" to be at the top of the README
//! always, not only today. A section moved down by a later edit would still
//! read fine, so nothing but a test notices.

/// The README, as the repository's front page shows it.
const README: &str = include_str!("../../../README.md");

#[test]
fn the_first_section_of_the_readme_is_the_repository_history() {
    let first = README
        .lines()
        .find(|line| line.starts_with("## "))
        .expect("the README has at least one section");
    assert_eq!(first, "## Repository history");
}

#[test]
fn nothing_but_the_title_comes_before_the_repository_history() {
    let before: Vec<&str> = README
        .lines()
        .take_while(|line| *line != "## Repository history")
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(before, vec!["# Repos Explorer"]);
}

#[test]
fn the_repository_history_section_links_the_film() {
    let section: String = README
        .lines()
        .skip_while(|line| *line != "## Repository history")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        section.contains("https://stewartscottrogers.github.io/RepoSphereExplorer/#film"),
        "the section no longer links the film:\n{section}"
    );
}
