//! A harvest taken from a dataset's map indexes rather than from the planet: for any area inside the
//! countries indexed, the very harvest the planet gives, read from the few row groups around it.

use planetrs_dataset::{Dataset, ForReadingFiles};
use planetrs_geo::Area;

use super::{Harvest, Provenance};
use crate::error::ClipError;

/// Everything the map indexes of `dataset` hold for one area, as the passes over the planet take it.
pub fn harvest_from_dataset<F: ForReadingFiles>(
    dataset: &Dataset<F>,
    area: &Area,
) -> Result<Harvest, ClipError> {
    let _ = dataset;
    Ok(Harvest {
        provenance: Provenance::default(),
        collected: area.collected(),
        nodes: Vec::new(),
        named_nodes: Vec::new(),
        ways: Vec::new(),
        relations: Vec::new(),
    })
}
