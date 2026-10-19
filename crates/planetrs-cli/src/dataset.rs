//! Rebuilding one tool's parts of a dataset in a directory beside it, and putting the result in its
//! place whole, with the other tool's parts of the same planet carried over.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use planetrs_dataset::Part;
use planetrs_dataset::directory::{CarriedOver, carry_over};

use crate::files::replace_directory;

/// The country codes as an index wants them: upper case, trimmed, each once, in the order given.
pub fn country_codes(asked: &[String]) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for code in asked
        .iter()
        .map(|code| code.trim().to_uppercase())
        .filter(|code| !code.is_empty())
    {
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

/// The directory a build happens in before it replaces `destination`.
pub fn building_beside(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".building");
    destination.with_file_name(name)
}

/// Builds one tool's parts with `build` in a directory beside `destination`, brings in the parts of
/// the dataset already there that `others` says the other tool builds, and puts the result in its
/// place, so that a crash or a full disk midway never leaves half a dataset where a whole one was.
///
/// A failed build leaves the previous dataset untouched.
pub fn rebuild<T>(
    destination: &Path,
    others: impl Fn(&Part) -> bool,
    build: impl FnOnce(&Path) -> Result<T>,
) -> Result<(T, CarriedOver)> {
    let building = building_beside(destination);
    if building.exists() {
        std::fs::remove_dir_all(&building)
            .with_context(|| format!("removing an unfinished build at {}", building.display()))?;
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let built = match build(&building) {
        Ok(built) => built,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&building);
            return Err(error.context("the dataset was not rebuilt; the previous one, if any, is untouched"));
        }
    };
    let carried = carry_over(destination, &building, others)
        .with_context(|| format!("keeping the other tool's parts of {}", destination.display()))?;
    replace_directory(&building, destination)
        .with_context(|| format!("moving the new dataset into {}", destination.display()))?;
    Ok((built, carried))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_codes_in_any_case_and_repeated_when_read_then_each_is_kept_once_in_upper_case() {
        let asked = [" fr", "MC", "Fr", ""].map(String::from);

        assert_eq!(country_codes(&asked), vec!["FR", "MC"]);
    }

    #[test]
    fn given_a_dataset_directory_when_rebuilt_then_the_build_happens_beside_it() {
        assert_eq!(
            building_beside(Path::new("/data/dataset")),
            PathBuf::from("/data/dataset.building")
        );
    }
}
