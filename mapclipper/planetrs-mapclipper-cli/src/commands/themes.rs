//! Listing the themes built in.

use std::path::Path;

use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::theme::BUILT_IN;

use crate::cli::{Shape, ThemesArgs};
use crate::report::{Report, quoted};

/// Lists the built-in themes, as text or JSON.
pub fn run(arguments: ThemesArgs) -> Result<(), ClipError> {
    let listed = match arguments.output {
        Shape::Json => themes_as_json()?,
        Shape::Text => themes_as_text()?,
    };
    print!("{listed}");
    Ok(())
}

fn themes_as_text() -> Result<String, ClipError> {
    let mut report = Report::new(false);
    for (name, text) in BUILT_IN {
        let theme = planetrs_mapclipper::theme::parse(text, Path::new(name))?;
        report.line(name, &theme.description);
    }
    report.blank();
    report.note("any of these, or a path to a theme file of your own, goes to --theme");
    Ok(report.text().to_string())
}

fn themes_as_json() -> Result<String, ClipError> {
    let described: Vec<String> = BUILT_IN
        .iter()
        .map(|(name, text)| {
            let theme = planetrs_mapclipper::theme::parse(text, Path::new(name))?;
            let drawn: Vec<String> = theme
                .order
                .iter()
                .map(|category| quoted(category.name()))
                .collect();
            Ok(format!(
                "{{\"name\":{},\"description\":{},\"background\":{},\"draws\":[{}]}}",
                quoted(name),
                quoted(&theme.description),
                quoted(&theme.background),
                drawn.join(",")
            ))
        })
        .collect::<Result<_, ClipError>>()?;

    Ok(format!("[{}]\n", described.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_the_built_in_themes_when_listed_as_json_then_every_one_is_there() {
        let listed = themes_as_json().unwrap();

        assert!(
            BUILT_IN
                .iter()
                .all(|(name, _)| listed.contains(&format!("\"name\":\"{name}\"")))
        );
    }

    #[test]
    fn given_the_built_in_themes_when_listed_as_text_then_each_carries_its_description() {
        let listed = themes_as_text().unwrap();

        assert!(listed.contains("linework"));
        assert!(listed.contains("road network"));
    }
}
