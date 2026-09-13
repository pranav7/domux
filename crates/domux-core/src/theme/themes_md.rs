//! Holds `docs/themes.md` to the code: its role table names every role once with the values the
//! two built-in themes give it, and the files it shows are the files domux reads.

use super::builtin::BUILTIN;
use super::file::ThemeLayer;
use super::role::Role;
use super::value::ColorValue;
use std::collections::BTreeMap;

/// The text of `docs/themes.md`.
pub(super) fn text() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/themes.md");
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The lines of the section under `heading`, up to the next heading of the same level.
fn section(text: &str, heading: &str) -> Vec<String> {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let mut lines = text.lines().skip_while(|l| *l != heading);
    assert!(lines.next().is_some(), "docs/themes.md has no {heading:?}");
    lines
        .take_while(|l| {
            let hashes = l.chars().take_while(|c| *c == '#').count();
            !(hashes > 0 && hashes <= level && l[hashes..].starts_with(' '))
        })
        .map(str::to_string)
        .collect()
}

fn builtin_layer(name: &str) -> ThemeLayer {
    let (_, text) = BUILTIN.iter().find(|(n, _)| *n == name).expect("built in");
    ThemeLayer::parse(name, text)
        .expect("a built-in theme parses")
        .0
}

/// The text between the first pair of backticks in a table cell.
fn quoted(cell: &str) -> Option<&str> {
    let start = cell.find('`')? + 1;
    let len = cell[start..].find('`')?;
    Some(&cell[start..start + len])
}

#[test]
fn the_role_table_names_every_role_once_with_both_built_in_values() {
    let text = text();
    let domux = builtin_layer("domux");
    let terminal = builtin_layer("terminal");
    let mut seen: BTreeMap<Role, usize> = BTreeMap::new();
    for line in section(&text, "## Every role") {
        let Some(row) = line.strip_prefix('|') else {
            continue;
        };
        let cells: Vec<&str> = row
            .trim_end_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(name) = quoted(cells[0]) else {
            continue;
        };
        let role = Role::from_name(name)
            .unwrap_or_else(|| panic!("the role table names {name}, which is not a role"));
        *seen.entry(role).or_default() += 1;
        assert_eq!(
            cells.len(),
            4,
            "{name}: role, what it colours, domux, terminal"
        );
        let domux_cell = quoted(cells[2]).unwrap_or_else(|| panic!("{name}: no domux value"));
        assert_eq!(
            ColorValue::parse(domux_cell).ok(),
            domux.roles.get(&role).copied(),
            "{name}'s domux value"
        );
        let terminal_value = match cells[3] {
            "not set" => None,
            cell => Some(
                ColorValue::parse(quoted(cell).unwrap_or_else(|| panic!("{name}: {cell}")))
                    .unwrap_or_else(|e| panic!("{name}'s terminal value: {e}")),
            ),
        };
        assert_eq!(
            terminal_value,
            terminal.roles.get(&role).copied(),
            "{name}'s terminal value"
        );
    }
    for role in Role::ALL {
        assert_eq!(
            seen.get(role),
            Some(&1),
            "{} is not in the role table exactly once",
            role.name()
        );
    }
}

#[test]
fn the_themes_doc_shows_the_built_in_terminal_theme_as_it_is() {
    let (_, file) = BUILTIN
        .iter()
        .find(|(n, _)| *n == "terminal")
        .expect("built in");
    assert!(
        text().contains(&format!("```toml\n{file}```\n")),
        "docs/themes.md does not carry builtin/terminal.toml word for word"
    );
}
