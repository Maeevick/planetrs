//! The gazetteer on disk, so that geocoding a file is a lookup rather than a scan of the planet.
//!
//! Tab-separated and deliberately dull: one commune per line, one country per line, a comment line
//! saying which planet file it was built from. Readable with `less`, diffable between two builds,
//! and deterministic, so two indexings of the same planet are byte-identical.

use std::fs;
use std::io;
use std::path::Path;

use thiserror::Error;

use crate::boundary::Country;
use crate::gazetteer::Commune;

pub(crate) const COMMUNES: &str = "communes.tsv";
const COUNTRIES: &str = "countries.tsv";

/// The second line of every index file. An index is a Derivative Database of OpenStreetMap, so it
/// carries the attribution wherever it is copied.
const ATTRIBUTION: &str = "# Data © OpenStreetMap contributors, under the Open Database License (ODbL) 1.0: \
     https://opendatacommons.org/licenses/odbl/1-0/ (https://www.openstreetmap.org/copyright)";

/// Why an index file could not be read or written.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IndexError {
    /// The file system refused.
    #[error("reading or writing the index failed")]
    Io(#[from] io::Error),
    /// A line does not read as an entry of its file.
    #[error("{file} line {line} does not read as an index entry")]
    Malformed {
        /// The file, `communes.tsv` for one.
        file: &'static str,
        /// The line, counted from one.
        line: usize,
    },
}

/// Writes the gazetteer to `directory`, communes sorted by relation so the bytes depend only on
/// the planet file.
pub fn write_index(
    directory: &Path,
    communes: &[Commune],
    countries: &[Country],
    provenance: &str,
) -> Result<(), IndexError> {
    fs::create_dir_all(directory)?;

    let mut sorted: Vec<&Commune> = communes.iter().collect();
    sorted.sort_by_key(|commune| commune.relation);

    let mut text = format!("# planetrs gazetteer v1, {provenance}\n{ATTRIBUTION}\n");
    for commune in sorted {
        let (longitude, latitude) = match commune.centre {
            Some([longitude, latitude]) => (format!("{longitude:.7}"), format!("{latitude:.7}")),
            None => (String::new(), String::new()),
        };
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            commune.relation,
            commune.level,
            commune.postcodes.join(";"),
            commune.names.join("|"),
            longitude,
            latitude,
            commune.country.as_deref().unwrap_or(""),
        ));
    }
    fs::write(directory.join(COMMUNES), text)?;

    let mut text = format!("# planetrs countries v1, {provenance}\n{ATTRIBUTION}\n");
    for country in countries {
        text.push_str(&format!("{}\t{}\n", country.code, country.names.join("|")));
    }
    fs::write(directory.join(COUNTRIES), text)?;
    Ok(())
}

fn split_list(field: &str, separator: char) -> Vec<String> {
    field
        .split(separator)
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect()
}

/// Reads a gazetteer written by [`write_index`].
pub fn read_index(directory: &Path) -> Result<(Vec<Commune>, Vec<Country>), IndexError> {
    let mut communes = Vec::new();
    for (at, line) in fs::read_to_string(directory.join(COMMUNES))?.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let malformed = || IndexError::Malformed {
            file: COMMUNES,
            line: at + 1,
        };
        let field: Vec<&str> = line.split('\t').collect();
        let [relation, level, postcodes, names, longitude, latitude, country] = field[..] else {
            return Err(malformed());
        };
        let centre = match (longitude.parse::<f64>(), latitude.parse::<f64>()) {
            (Ok(longitude), Ok(latitude)) => Some([longitude, latitude]),
            _ => None,
        };
        communes.push(Commune {
            relation: relation.parse().map_err(|_| malformed())?,
            level: level.parse().map_err(|_| malformed())?,
            postcodes: split_list(postcodes, ';'),
            names: split_list(names, '|'),
            centre,
            country: (!country.is_empty()).then(|| country.to_string()),
        });
    }

    let mut countries = Vec::new();
    for (at, line) in fs::read_to_string(directory.join(COUNTRIES))?.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((code, names)) = line.split_once('\t') else {
            return Err(IndexError::Malformed {
                file: COUNTRIES,
                line: at + 1,
            });
        };
        countries.push(Country {
            code: code.to_string(),
            names: split_list(names, '|'),
        });
    }

    Ok((communes, countries))
}

const STREETS: &str = "streets.tsv";

/// Writes every commune's streets: one line per door, and one line for the middle of each road.
///
/// A road's own points are collapsed to their middle before writing, because that middle is all the
/// lookup ever asks of them and the points themselves would triple the file.
pub fn write_streets(
    directory: &Path,
    index: &crate::street_index::StreetIndex,
    provenance: &str,
) -> Result<(), IndexError> {
    use std::io::Write;

    fs::create_dir_all(directory)?;
    let mut out = io::BufWriter::new(fs::File::create(directory.join(STREETS))?);
    writeln!(out, "# planetrs streets v1, {provenance}\n{ATTRIBUTION}")?;

    let mut communes: Vec<(&i64, &Vec<crate::street_index::Street>)> = index.communes().collect();
    communes.sort_by_key(|(commune, _)| **commune);
    for (commune, streets) in communes {
        for street in streets {
            let mut doors: Vec<&crate::street_index::AddressPoint> = street.addresses.iter().collect();
            doors.sort_by(|left, right| {
                (left.number.number, &left.number.suffix)
                    .cmp(&(right.number.number, &right.number.suffix))
                    .then(left.position[0].total_cmp(&right.position[0]))
                    .then(left.position[1].total_cmp(&right.position[1]))
            });
            for door in doors {
                writeln!(
                    out,
                    "{commune}\t{}\tD\t{}\t{}\t{:.7}\t{:.7}",
                    street.name, door.number.number, door.number.suffix, door.position[0], door.position[1]
                )?;
            }
            if !street.path.is_empty() {
                let count = street.path.len() as f64;
                let [x, y] = street
                    .path
                    .iter()
                    .fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
                        [x + longitude, y + latitude]
                    });
                writeln!(
                    out,
                    "{commune}\t{}\tM\t\t\t{:.7}\t{:.7}",
                    street.name,
                    x / count,
                    y / count
                )?;
            }
        }
    }
    out.flush()?;
    Ok(())
}

/// Reads the streets written by [`write_streets`]. An index built without streets has none, which is
/// not an error: the lookup then stops at the commune.
pub fn read_streets(directory: &Path) -> Result<crate::street_index::StreetIndex, IndexError> {
    use crate::street_index::{AddressPoint, Street, StreetIndex};
    use crate::text::HouseNumber;

    let path = directory.join(STREETS);
    if !path.exists() {
        return Ok(StreetIndex::default());
    }
    let mut filed = Vec::new();
    for (at, line) in fs::read_to_string(path)?.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let malformed = || IndexError::Malformed {
            file: STREETS,
            line: at + 1,
        };
        let field: Vec<&str> = line.split('\t').collect();
        let [commune, name, kind, number, suffix, longitude, latitude] = field[..] else {
            return Err(malformed());
        };
        let commune: i64 = commune.parse().map_err(|_| malformed())?;
        let position = [
            longitude.parse().map_err(|_| malformed())?,
            latitude.parse().map_err(|_| malformed())?,
        ];
        let street = match kind {
            "D" => Street {
                name: name.to_string(),
                addresses: vec![AddressPoint {
                    number: HouseNumber {
                        number: number.parse().map_err(|_| malformed())?,
                        suffix: suffix.to_string(),
                    },
                    position,
                }],
                path: vec![],
            },
            "M" => Street {
                name: name.to_string(),
                addresses: vec![],
                path: vec![position],
            },
            _ => return Err(malformed()),
        };
        filed.push((commune, street));
    }
    Ok(StreetIndex::of(filed))
}

const PARENTS: &str = "parents.tsv";

/// Writes which commune each former town belongs to, one pair per line, sorted.
pub fn write_parents(
    directory: &Path,
    parents: &std::collections::HashMap<i64, i64>,
    provenance: &str,
) -> Result<(), IndexError> {
    fs::create_dir_all(directory)?;
    let mut pairs: Vec<(&i64, &i64)> = parents.iter().collect();
    pairs.sort();
    let mut text = format!("# planetrs former towns v1, {provenance}\n{ATTRIBUTION}\n");
    for (town, commune) in pairs {
        text.push_str(&format!("{town}\t{commune}\n"));
    }
    fs::write(directory.join(PARENTS), text)?;
    Ok(())
}

/// Reads the pairs written by [`write_parents`]. An index built without streets has none.
pub fn read_parents(directory: &Path) -> Result<std::collections::HashMap<i64, i64>, IndexError> {
    let path = directory.join(PARENTS);
    if !path.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let mut parents = std::collections::HashMap::new();
    for (at, line) in fs::read_to_string(path)?.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let malformed = || IndexError::Malformed {
            file: PARENTS,
            line: at + 1,
        };
        let (town, commune) = line.split_once('\t').ok_or_else(malformed)?;
        parents.insert(
            town.parse().map_err(|_| malformed())?,
            commune.parse().map_err(|_| malformed())?,
        );
    }
    Ok(parents)
}

const AREAS: &str = "areas.tsv";

/// Writes each outlined commune's area in square kilometres, one per line, sorted.
pub fn write_areas(
    directory: &Path,
    areas: &std::collections::HashMap<i64, f64>,
    provenance: &str,
) -> Result<(), IndexError> {
    fs::create_dir_all(directory)?;
    let mut pairs: Vec<(&i64, &f64)> = areas.iter().collect();
    pairs.sort_by_key(|(relation, _)| **relation);
    let mut text = format!("# planetrs commune areas v1, square kilometres, {provenance}\n{ATTRIBUTION}\n");
    for (relation, area) in pairs {
        text.push_str(&format!("{relation}\t{area:.3}\n"));
    }
    fs::write(directory.join(AREAS), text)?;
    Ok(())
}

/// Reads the areas written by [`write_areas`]. An index built without streets has none.
pub fn read_areas(directory: &Path) -> Result<std::collections::HashMap<i64, f64>, IndexError> {
    let path = directory.join(AREAS);
    if !path.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let mut areas = std::collections::HashMap::new();
    for (at, line) in fs::read_to_string(path)?.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let malformed = || IndexError::Malformed {
            file: AREAS,
            line: at + 1,
        };
        let (relation, area) = line.split_once('\t').ok_or_else(malformed)?;
        areas.insert(
            relation.parse().map_err(|_| malformed())?,
            area.parse().map_err(|_| malformed())?,
        );
    }
    Ok(areas)
}

const SHOPS: &str = "shops.tsv";

/// Writes every commune's shops, one per line, sorted so two builds of one planet are identical.
pub fn write_shops(
    directory: &Path,
    index: &crate::shops::ShopIndex,
    provenance: &str,
) -> Result<(), IndexError> {
    use std::io::Write;

    fs::create_dir_all(directory)?;
    let mut lines: Vec<String> = index
        .communes()
        .flat_map(|(commune, shops)| {
            shops.iter().map(move |shop| {
                format!(
                    "{commune}\t{}\t{}\t{:.7}\t{:.7}",
                    shop.names.join("|"),
                    shop.street,
                    shop.position[0],
                    shop.position[1]
                )
            })
        })
        .collect();
    lines.sort();
    let mut out = io::BufWriter::new(fs::File::create(directory.join(SHOPS))?);
    writeln!(out, "# planetrs shops v1, {provenance}\n{ATTRIBUTION}")?;
    for line in lines {
        writeln!(out, "{line}")?;
    }
    out.flush()?;
    Ok(())
}

/// Reads the shops written by [`write_shops`]. An index built without them has none.
pub fn read_shops(directory: &Path) -> Result<crate::shops::ShopIndex, IndexError> {
    use crate::shops::{Shop, ShopIndex};

    let path = directory.join(SHOPS);
    if !path.exists() {
        return Ok(ShopIndex::default());
    }
    let mut filed = Vec::new();
    for (at, line) in fs::read_to_string(path)?.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let malformed = || IndexError::Malformed {
            file: SHOPS,
            line: at + 1,
        };
        let field: Vec<&str> = line.split('\t').collect();
        let [commune, names, street, longitude, latitude] = field[..] else {
            return Err(malformed());
        };
        filed.push((
            commune.parse().map_err(|_| malformed())?,
            Shop {
                names: split_list(names, '|'),
                street: street.to_string(),
                position: [
                    longitude.parse().map_err(|_| malformed())?,
                    latitude.parse().map_err(|_| malformed())?,
                ],
            },
        ));
    }
    Ok(ShopIndex::of(filed))
}
