//! The gazetteer: every administrative area of the planet, from countries to communes, and what it
//! takes to settle a site in one of them.
//!
//! It is the one file a geocoder reads whole. It holds the names and postcodes an address is matched
//! against, each area's centre, and, for the communes of the countries indexed, their area and the
//! commune that absorbed them when they are former towns.

use planetrs_geo::wkb;

use crate::geoparquet;
use crate::manifest;
use crate::tables::{
    self, Column, TableError, Values, integers, optional_bytes, optional_integers, optional_reals,
    optional_texts, small_integers, text_lists,
};

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

/// How many areas a row group holds. The gazetteer is read whole, so this only bounds the memory
/// a writer and a reader need at once.
const AREAS_PER_ROW_GROUP: usize = 100_000;

/// The gazetteer as a GeoParquet file, one row per area in the order given.
pub fn to_parquet(areas: &[Area]) -> Result<Vec<u8>, TableError> {
    let row_groups: Vec<Vec<Column>> = areas.chunks(AREAS_PER_ROW_GROUP).map(columns_of).collect();
    tables::write(
        &row_groups,
        &[geoparquet::geo(&["Point"]), manifest::notice("gazetteer")],
        &["relation"],
    )
}

/// The areas of a gazetteer file, in its order.
pub fn from_parquet(bytes: &[u8]) -> Result<Vec<Area>, TableError> {
    let columns = tables::read(bytes)?;
    let relations = integers(&columns, "relation")?;
    let levels = small_integers(&columns, "level")?;
    let countries = optional_texts(&columns, "country")?;
    let names = text_lists(&columns, "names")?;
    let postcodes = text_lists(&columns, "postcodes")?;
    let areas = optional_reals(&columns, "square_kilometres")?;
    let absorbed = optional_integers(&columns, "absorbed_by")?;
    let geometries = optional_bytes(&columns, "geometry")?;
    (0..relations.len())
        .map(|row| {
            Ok(Area {
                relation: relations[row],
                level: u8::try_from(levels[row]).map_err(|_| unexpected("level"))?,
                country: countries[row].clone(),
                names: names[row].clone(),
                postcodes: postcodes[row].clone(),
                square_kilometres: areas[row],
                absorbed_by: absorbed[row],
                centre: geometries[row]
                    .as_deref()
                    .map(|bytes| {
                        wkb::read_point(bytes)
                            .map(|(longitude, latitude)| [longitude, latitude])
                            .map_err(|_| unexpected("geometry"))
                    })
                    .transpose()?,
            })
        })
        .collect()
}

fn columns_of(areas: &[Area]) -> Vec<Column> {
    vec![
        Column::of(
            "relation",
            Values::Integers(areas.iter().map(|area| area.relation).collect()),
        ),
        Column::of(
            "level",
            Values::SmallIntegers(areas.iter().map(|area| i32::from(area.level)).collect()),
        ),
        Column::of(
            "country",
            Values::OptionalTexts(areas.iter().map(|area| area.country.clone()).collect()),
        ),
        Column::of(
            "names",
            Values::TextLists(areas.iter().map(|area| area.names.clone()).collect()),
        ),
        Column::of(
            "postcodes",
            Values::TextLists(areas.iter().map(|area| area.postcodes.clone()).collect()),
        ),
        Column::of(
            "square_kilometres",
            Values::OptionalReals(areas.iter().map(|area| area.square_kilometres).collect()),
        ),
        Column::of(
            "absorbed_by",
            Values::OptionalIntegers(areas.iter().map(|area| area.absorbed_by).collect()),
        ),
        Column::of(
            "geometry",
            Values::OptionalBytes(
                areas
                    .iter()
                    .map(|area| {
                        area.centre
                            .map(|[longitude, latitude]| wkb::point(longitude, latitude))
                    })
                    .collect(),
            ),
        ),
    ]
}

fn unexpected(name: &str) -> TableError {
    TableError::Unreadable {
        detail: format!("the gazetteer's column {name} is missing or of another kind"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let read_back = from_parquet(&to_parquet(&areas)?)?;

        assert_eq!(read_back, areas);
        Ok(())
    }

    fn metadata_of(bytes: &[u8], key: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let value = tables::metadata(bytes)?
            .into_iter()
            .find(|(found, _)| found == key)
            .map(|(_, value)| value)
            .ok_or(format!("no {key} metadata"))?;
        Ok(serde_json::from_str(&value)?)
    }

    #[test]
    fn given_a_gazetteer_file_when_its_footer_is_read_then_it_declares_geoparquet_with_centres_as_wkb_points()
    -> Result<(), Box<dyn std::error::Error>> {
        let written = to_parquet(&france_sens_and_a_former_town())?;

        let geo = metadata_of(&written, "geo")?;

        assert_eq!(geo["version"], "1.1.0");
        assert_eq!(geo["primary_column"], "geometry");
        assert_eq!(geo["columns"]["geometry"]["encoding"], "WKB");
        assert_eq!(
            geo["columns"]["geometry"]["geometry_types"],
            serde_json::json!(["Point"])
        );
        Ok(())
    }

    /// A file holds its content alone, so that the same content built from next week's planet is the
    /// same file, and only the manifest says which planet a dataset came from.
    #[test]
    fn given_a_gazetteer_file_when_its_footer_is_read_then_it_names_the_odbl_and_its_format_and_leaves_the_planet_to_the_manifest()
    -> Result<(), Box<dyn std::error::Error>> {
        let written = to_parquet(&france_sens_and_a_former_town())?;

        let notice = metadata_of(&written, "planetrs")?;

        assert_eq!(notice["file"], "gazetteer");
        assert_eq!(notice["format"], "planetrs dataset 1");
        assert!(notice.get("planet").is_none(), "{notice}");
        assert_eq!(notice["licence"], "ODbL-1.0");
        assert_eq!(
            notice["attribution"],
            "© OpenStreetMap contributors, https://www.openstreetmap.org/copyright"
        );
        Ok(())
    }

    #[test]
    fn given_the_same_areas_when_written_twice_then_the_two_files_are_byte_identical()
    -> Result<(), TableError> {
        let areas = france_sens_and_a_former_town();

        let (first, second) = (to_parquet(&areas)?, to_parquet(&areas)?);

        assert_eq!(first, second);
        Ok(())
    }

    #[test]
    fn given_a_gazetteer_file_cut_short_or_not_parquet_at_all_when_read_then_it_is_refused_rather_than_misread()
    -> Result<(), TableError> {
        let written = to_parquet(&france_sens_and_a_former_town())?;

        let refused = [
            &written[..written.len() - 9],
            b"PAR1 but nothing else".as_slice(),
            &[],
        ]
        .map(|bytes| matches!(from_parquet(bytes), Err(TableError::Unreadable { .. })));

        assert_eq!(refused, [true, true, true]);
        Ok(())
    }
}
