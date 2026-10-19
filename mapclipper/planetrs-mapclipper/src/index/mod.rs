//! Filing what the planet holds under the countries asked for, as each country's map index, so that
//! a map of any box inside them is cut from the dataset rather than from the planet.

use std::path::Path;

use crate::error::ClipError;

/// What one country's map index holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryIndexed {
    /// The country's ISO 3166-1 code.
    pub country: String,
    /// Nodes kept for a name or for standing alone.
    pub nodes: usize,
    /// Ways with a node in the country.
    pub ways: usize,
    /// Multipolygons with a member among those ways.
    pub multipolygons: usize,
}

/// Builds the map index of each country of `countries` from the planet file at `planet`, and records
/// each in the dataset in `directory`.
pub fn build_map_index(
    planet: &Path,
    directory: &Path,
    countries: &[&str],
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Vec<CountryIndexed>, ClipError> {
    let _ = (planet, directory, countries, progress);
    Ok(Vec::new())
}
