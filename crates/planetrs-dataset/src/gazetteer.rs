//! The gazetteer: every administrative area of the planet, from countries to communes, and what it
//! takes to settle a site in one of them.
//!
//! It is the one file a geocoder reads whole. It holds the names and postcodes an address is matched
//! against, each area's centre, and, for the communes of the countries indexed, their area and the
//! commune that absorbed them when they are former towns.

use crate::manifest::Provenance;
use crate::tables::TableError;

/// One administrative area: a country at level 2, a commune at level 8, anything between or below.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    /// The OpenStreetMap relation the area is drawn by.
    pub relation: i64,
    /// Its `admin_level`.
    pub level: u8,
    /// The ISO 3166-1 code of the country it lies in, or its own for a country.
    pub country: Option<String>,
    /// Every name it answers to, as comparison keys.
    pub names: Vec<String>,
    /// Every postcode it carries.
    pub postcodes: Vec<String>,
    /// Its area, for the communes of the countries indexed.
    pub square_kilometres: Option<f64>,
    /// For a former town, the commune that absorbed it.
    pub absorbed_by: Option<i64>,
    /// Its centre, longitude then latitude in degrees.
    pub centre: Option<[f64; 2]>,
}

/// The gazetteer as a GeoParquet file, one row per area in the order given.
pub fn to_parquet(areas: &[Area], planet: &Provenance) -> Result<Vec<u8>, TableError> {
    let _ = (areas, planet);
    Ok(Vec::new())
}

/// The areas of a gazetteer file, in its order.
pub fn from_parquet(bytes: &[u8]) -> Result<Vec<Area>, TableError> {
    let _ = bytes;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planet() -> Provenance {
        Provenance {
            source_name: "planet-260907.osm.pbf".into(),
            file_size: 94_746_421_723,
            blob_count: 52_810,
            replication_timestamp: 1_788_739_204,
            replication_sequence: 4_321,
            writing_program: "planet-dump-ng 1.2.7".into(),
        }
    }

    fn france_sens_and_a_former_town() -> Vec<Area> {
        vec![
            Area {
                relation: 7_395,
                level: 6,
                country: Some("FR".into()),
                names: vec!["YONNE".into()],
                postcodes: vec![],
                square_kilometres: None,
                absorbed_by: None,
                centre: None,
            },
            Area {
                relation: 946_433,
                level: 8,
                country: Some("FR".into()),
                names: vec!["SENS".into(), "SENS YONNE".into()],
                postcodes: vec!["89100".into()],
                square_kilometres: Some(105.08),
                absorbed_by: None,
                centre: Some([3.282_606, 48.197_855_9]),
            },
            Area {
                relation: 1_402_847,
                level: 9,
                country: Some("FR".into()),
                names: vec!["SAINT CLEMENT".into()],
                postcodes: vec!["89100".into(), "89101".into()],
                square_kilometres: Some(12.5),
                absorbed_by: Some(946_433),
                centre: Some([3.29, 48.21]),
            },
            Area {
                relation: 2_202_162,
                level: 2,
                country: Some("FR".into()),
                names: vec!["FRANCE".into(), "REPUBLIQUE FRANCAISE".into()],
                postcodes: vec![],
                square_kilometres: None,
                absorbed_by: None,
                centre: None,
            },
        ]
    }

    #[test]
    fn given_areas_when_written_as_parquet_and_read_back_then_they_are_the_same_areas_in_the_same_order()
    -> Result<(), TableError> {
        let areas = france_sens_and_a_former_town();

        let read_back = from_parquet(&to_parquet(&areas, &planet())?)?;

        assert_eq!(read_back, areas);
        Ok(())
    }
}
