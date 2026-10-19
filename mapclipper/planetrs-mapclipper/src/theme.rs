//! What colour things are, as data rather than as code.
//!
//! A theme is a small TOML file naming a background, an ink, a draw order and what each category is
//! painted with. Nothing here knows about OSM tags: the categories are fixed by [`crate::scene`],
//! so a theme decides what a quay looks like and never what counts as one.
//!
//! The reader is deliberately strict. An unknown key, an unknown category or a malformed line is an
//! error naming the line, never something skipped, because a theme that silently styles nothing
//! produces a blank map rather than a complaint.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::ClipError;
use crate::scene::{Category, LabelClass};

/// The themes that ship with the tool, by name, with the text of each.
pub const BUILT_IN: [(&str, &str); 3] = [
    ("atlas", include_str!("../themes/atlas.toml")),
    ("tonal", include_str!("../themes/tonal.toml")),
    ("linework", include_str!("../themes/linework.toml")),
];

/// How a category is drawn: what fills it, what outlines it, and how thick the outline is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ink {
    /// The colour inside an area, none for a line.
    pub fill: Option<String>,
    /// The colour of the outline or of the line itself.
    pub stroke: Option<String>,
    /// The stroke's width in pixels of the canvas.
    pub width: f64,
}

/// How a class of name is drawn. A theme that says nothing gets the defaults below.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelInk {
    /// Type size in pixels of the canvas.
    pub size: f64,
    /// The colour of the glyphs, the theme's own ink when unstated.
    pub colour: Option<String>,
    /// What goes behind the glyphs so they stay readable over whatever they fall on.
    pub halo: Option<String>,
}

impl LabelInk {
    fn for_class(class: LabelClass) -> Self {
        let size = match class {
            LabelClass::Settlement => 13.0,
            LabelClass::Water | LabelClass::Area => 10.0,
        };
        Self {
            size,
            colour: None,
            halo: None,
        }
    }
}

/// What a map looks like: its background, the ink of each category and the order they are drawn in.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// The theme's name, as `--theme` takes it.
    pub name: String,
    /// What the theme is for, in a sentence.
    pub description: String,
    /// The colour of the ground under everything.
    pub background: String,
    /// The colour of text the theme does not style otherwise.
    pub ink: String,
    /// Back to front. A category left out of this is not drawn, whatever colour it was given.
    pub order: Vec<Category>,
    /// How each category is drawn.
    pub inks: BTreeMap<Category, Ink>,
    /// How each class of name is drawn, where the theme says.
    pub labels: BTreeMap<LabelClass, LabelInk>,
}

impl Theme {
    /// A built-in theme by name, or a theme file at that path.
    pub fn named_or_at(named: &str) -> Result<Self, ClipError> {
        if let Some((_, text)) = BUILT_IN.iter().find(|(name, _)| *name == named) {
            return parse(text, Path::new(named));
        }

        let path = PathBuf::from(named);
        if path.extension().is_some() || path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|error| ClipError::reading(&path, error))?;
            return parse(&text, &path);
        }

        Err(ClipError::UnknownTheme {
            named: named.to_string(),
            known: BUILT_IN.iter().map(|(name, _)| (*name).to_string()).collect(),
        })
    }

    /// How a category is drawn, nothing at all when the theme leaves it out.
    pub fn ink_of(&self, category: Category) -> Ink {
        self.inks.get(&category).cloned().unwrap_or_default()
    }

    /// How a class of name is drawn, falling back to the theme's own ink and background.
    pub fn label_ink(&self, class: LabelClass) -> LabelInk {
        let stated = self
            .labels
            .get(&class)
            .cloned()
            .unwrap_or_else(|| LabelInk::for_class(class));

        LabelInk {
            size: stated.size,
            colour: Some(stated.colour.unwrap_or_else(|| self.ink.clone())),
            halo: Some(stated.halo.unwrap_or_else(|| self.background.clone())),
        }
    }

    /// The names of the themes built in.
    pub fn built_in_names() -> Vec<&'static str> {
        BUILT_IN.iter().map(|(name, _)| *name).collect()
    }
}

/// Which kind of thing the section currently open styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Styled {
    Shapes(Category),
    Names(LabelClass),
}

enum Line {
    Blank,
    Table(String),
    Pair(String, Value),
}

enum Value {
    Text(String),
    Number(f64),
    List(Vec<String>),
}

fn fault(path: &Path, line: usize, detail: impl Into<String>) -> ClipError {
    ClipError::Theme {
        path: path.to_path_buf(),
        line,
        detail: detail.into(),
    }
}

fn quoted(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let inner = trimmed.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then(|| inner.to_string())
}

/// Everything before the first `#` that is not inside a quoted string.
///
/// A colour is written `"#f1efe3"`, so cutting at the first hash anywhere turns every colour in the
/// file into a comment and the theme loads with nothing painted.
fn without_comment(raw: &str) -> &str {
    let mut quoted = false;
    for (at, character) in raw.char_indices() {
        match character {
            '"' => quoted = !quoted,
            '#' if !quoted => return &raw[..at],
            _ => {}
        }
    }
    raw
}

/// Whether every bracket opened on a line is closed on it, ignoring those inside quotes.
///
/// A draw order is long enough to want several lines, and TOML lets it have them, so a line that
/// opens a list and does not close it is continued rather than refused.
fn brackets_balanced(text: &str) -> bool {
    let mut depth = 0i32;
    let mut quoted = false;
    for character in text.chars() {
        match character {
            '"' => quoted = !quoted,
            '[' if !quoted => depth += 1,
            ']' if !quoted => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

/// The file's lines, with a list spread over several folded back into the line that opened it.
fn logical_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, String)> = None;

    for (index, raw) in text.lines().enumerate() {
        let stripped = without_comment(raw).trim();
        let (at, joined) = match open.take() {
            Some((at, mut buffer)) => {
                buffer.push(' ');
                buffer.push_str(stripped);
                (at, buffer)
            }
            None => (index + 1, stripped.to_string()),
        };

        if brackets_balanced(&joined) {
            out.push((at, joined));
        } else {
            open = Some((at, joined));
        }
    }

    out.extend(open);
    out
}

fn read_line(text: &str) -> Result<Line, String> {
    if text.is_empty() {
        return Ok(Line::Blank);
    }

    if let Some(name) = text.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
        return Ok(Line::Table(name.trim().to_string()));
    }

    let (key, value) = text
        .split_once('=')
        .ok_or("expected key = value, a [table], or a comment")?;
    let (key, value) = (key.trim(), value.trim());

    if key.is_empty() {
        return Err("a value with no key".to_string());
    }

    if let Some(list) = value.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
        let entries: Option<Vec<String>> = list
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(quoted)
            .collect();
        return entries
            .map(|entries| Line::Pair(key.to_string(), Value::List(entries)))
            .ok_or_else(|| format!("{key} is a list of quoted names"));
    }

    if let Some(text) = quoted(value) {
        return Ok(Line::Pair(key.to_string(), Value::Text(text)));
    }

    value
        .parse::<f64>()
        .map(|number| Line::Pair(key.to_string(), Value::Number(number)))
        .map_err(|_| format!("{key} is a quoted string, a number, or a list"))
}

/// Reads a theme, refusing anything it does not understand.
pub fn parse(text: &str, path: &Path) -> Result<Theme, ClipError> {
    let mut theme = Theme {
        name: String::new(),
        description: String::new(),
        background: String::new(),
        ink: String::new(),
        order: Vec::new(),
        inks: BTreeMap::new(),
        labels: BTreeMap::new(),
    };
    let mut table: Option<Styled> = None;

    for (line, raw) in logical_lines(text) {
        match read_line(&raw).map_err(|detail| fault(path, line, detail))? {
            Line::Blank => {}
            Line::Table(name) => {
                table = Some(match name.strip_prefix("label-") {
                    Some(class) => {
                        let class = LabelClass::named(class)
                            .ok_or_else(|| fault(path, line, format!("no kind of label called {class}")))?;
                        theme
                            .labels
                            .entry(class)
                            .or_insert_with(|| LabelInk::for_class(class));
                        Styled::Names(class)
                    }
                    None => {
                        let category = Category::named(&name)
                            .ok_or_else(|| fault(path, line, format!("no category called {name}")))?;
                        theme.inks.entry(category).or_default();
                        Styled::Shapes(category)
                    }
                });
            }
            Line::Pair(key, value) => match (table, key.as_str(), value) {
                (None, "name", Value::Text(text)) => theme.name = text,
                (None, "description", Value::Text(text)) => theme.description = text,
                (None, "background", Value::Text(text)) => theme.background = text,
                (None, "ink", Value::Text(text)) => theme.ink = text,
                (None, "order", Value::List(names)) => {
                    theme.order = names
                        .iter()
                        .map(|name| {
                            Category::named(name)
                                .ok_or_else(|| fault(path, line, format!("no category called {name}")))
                        })
                        .collect::<Result<_, _>>()?;
                }
                (Some(Styled::Shapes(category)), "fill", Value::Text(text)) => {
                    theme.inks.entry(category).or_default().fill = Some(text);
                }
                (Some(Styled::Shapes(category)), "stroke", Value::Text(text)) => {
                    theme.inks.entry(category).or_default().stroke = Some(text);
                }
                (Some(Styled::Shapes(category)), "width", Value::Number(number)) => {
                    theme.inks.entry(category).or_default().width = number;
                }
                (Some(Styled::Names(class)), "size", Value::Number(number)) => {
                    theme
                        .labels
                        .entry(class)
                        .or_insert_with(|| LabelInk::for_class(class))
                        .size = number;
                }
                (Some(Styled::Names(class)), "colour", Value::Text(text)) => {
                    theme
                        .labels
                        .entry(class)
                        .or_insert_with(|| LabelInk::for_class(class))
                        .colour = Some(text);
                }
                (Some(Styled::Names(class)), "halo", Value::Text(text)) => {
                    theme
                        .labels
                        .entry(class)
                        .or_insert_with(|| LabelInk::for_class(class))
                        .halo = Some(text);
                }
                (None, key, _) => {
                    return Err(fault(path, line, format!("{key} is not a theme setting")));
                }
                (Some(Styled::Shapes(category)), key, _) => {
                    return Err(fault(
                        path,
                        line,
                        format!(
                            "{key} is not something a category can have, under [{}]",
                            category.name()
                        ),
                    ));
                }
                (Some(Styled::Names(class)), key, _) => {
                    return Err(fault(
                        path,
                        line,
                        format!(
                            "{key} is not something a label can have, under [label-{}]",
                            class.name()
                        ),
                    ));
                }
            },
        }
    }

    if theme.background.is_empty() {
        return Err(fault(path, 0, "a theme needs a background colour"));
    }
    if theme.order.is_empty() {
        return Err(fault(
            path,
            0,
            "a theme needs an order, listing what to draw and in what sequence",
        ));
    }

    Ok(theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Theme, ClipError> {
        parse(text, Path::new("a-theme.toml"))
    }

    const MINIMAL: &str = r##"
name = "test"
background = "#ffffff"
order = ["water"]

[water]
fill = "#0000ff"
"##;

    #[test]
    fn given_a_theme_naming_a_colour_for_a_category_when_read_then_that_is_what_it_is_drawn_with() {
        let theme = read(MINIMAL).unwrap();

        assert_eq!(theme.ink_of(Category::Water).fill.as_deref(), Some("#0000ff"));
    }

    #[test]
    fn given_a_theme_stating_an_order_when_read_then_the_order_comes_back_as_categories() {
        let theme = read(MINIMAL).unwrap();

        assert_eq!(theme.order, vec![Category::Water]);
    }

    /// A category with a colour but no place in the order is not drawn, which is how a theme turns
    /// something off without deleting its colours.
    #[test]
    fn given_a_category_with_a_colour_but_no_place_in_the_order_when_read_then_it_is_still_not_drawn() {
        let theme =
            read("background = \"#fff\"\norder = [\"water\"]\n\n[building]\nfill = \"#111\"\n").unwrap();

        assert!(!theme.order.contains(&Category::Building));
        assert_eq!(theme.ink_of(Category::Building).fill.as_deref(), Some("#111"));
    }

    /// Every colour in a theme is written with a leading hash, so a reader that cuts at the first
    /// hash anywhere loads a theme that paints nothing and says so nowhere.
    #[test]
    fn given_a_colour_written_with_a_leading_hash_when_read_then_it_is_a_colour_and_not_a_comment() {
        let theme = read(MINIMAL).unwrap();

        assert_eq!(theme.background, "#ffffff");
    }

    #[test]
    fn given_comments_on_their_own_line_and_after_a_value_when_read_then_they_are_ignored() {
        let theme = read("# a theme\nbackground = \"#fff\" # the paper\norder = [\"water\"]\n").unwrap();

        assert_eq!(theme.background, "#fff");
    }

    /// A theme is the one file a reader writes by hand, so a typo has to name its own line rather
    /// than produce a map with something quietly missing.
    #[test]
    fn given_a_misspelled_setting_on_the_third_line_when_read_then_the_error_names_the_line_it_is_on() {
        let error = read("background = \"#fff\"\norder = [\"water\"]\nbackgrond = \"#000\"\n").unwrap_err();

        assert!(error.to_string().contains("a-theme.toml:3"), "got {error}");
        assert!(error.to_string().contains("backgrond"));
    }

    #[test]
    fn given_a_table_naming_no_category_we_know_when_read_then_it_is_refused() {
        let error =
            read("background = \"#fff\"\norder = [\"water\"]\n[chartreuse]\nfill = \"#0f0\"\n").unwrap_err();

        assert!(error.to_string().contains("no category called chartreuse"));
    }

    #[test]
    fn given_an_order_naming_no_category_we_know_when_read_then_it_is_refused() {
        let error = read("background = \"#fff\"\norder = [\"water\", \"chartreuse\"]\n").unwrap_err();

        assert!(error.to_string().contains("no category called chartreuse"));
    }

    #[test]
    fn given_a_category_with_a_setting_it_cannot_have_when_read_then_it_is_refused() {
        let error = read("background = \"#fff\"\norder = [\"water\"]\n[water]\nopacity = 0.5\n").unwrap_err();

        assert!(error.to_string().contains("opacity"));
        assert!(error.to_string().contains("[water]"));
    }

    #[test]
    fn given_a_theme_stating_no_background_when_read_then_it_is_refused_rather_than_drawn_on_nothing() {
        assert!(read("order = [\"water\"]\n").is_err());
    }

    #[test]
    fn given_a_theme_stating_no_order_when_read_then_it_is_refused_rather_than_drawing_nothing() {
        assert!(read("background = \"#fff\"\n").is_err());
    }

    /// A draw order names up to fourteen categories, which is more than fits comfortably on one
    /// line, and TOML lets it spread over several.
    #[test]
    fn given_a_list_spread_over_several_lines_when_read_then_it_is_taken_as_one() {
        let theme = read("background = \"#fff\"\norder = [\n  \"water\",\n  \"green\",\n]\n").unwrap();

        assert_eq!(theme.order, vec![Category::Water, Category::Green]);
    }

    #[test]
    fn given_a_list_spread_over_lines_holding_a_comment_when_read_then_the_comment_is_still_ignored() {
        let theme = read("background = \"#fff\"\norder = [\n  \"water\", # the wet bits\n]\n").unwrap();

        assert_eq!(theme.order, vec![Category::Water]);
    }

    #[test]
    fn given_a_width_for_major_roads_when_read_then_it_comes_back_as_a_number() {
        let theme = read(
            "background = \"#fff\"\norder = [\"road-major\"]\n[road-major]\nstroke = \"#fff\"\nwidth = 2.2\n",
        )
        .unwrap();

        assert_eq!(theme.ink_of(Category::RoadMajor).width, 2.2);
    }

    /// Every theme that ships has to load, or the tool is broken out of the box.
    #[test]
    fn given_a_built_in_theme_name_when_asked_for_then_the_theme_loads() {
        for (name, _) in BUILT_IN {
            let theme = Theme::named_or_at(name).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(theme.name, name);
            assert!(!theme.order.is_empty(), "{name} draws nothing");
        }
    }

    #[test]
    fn given_a_name_neither_built_in_nor_a_path_when_asked_for_then_the_error_lists_the_ones_that_are() {
        let error = Theme::named_or_at("chartreuse").unwrap_err();

        assert!(
            matches!(&error, ClipError::UnknownTheme { known, .. } if known.iter().any(|name| name == "atlas")),
            "{error}"
        );
    }
}
