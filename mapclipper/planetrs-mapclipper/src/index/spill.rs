//! Elements set down on disk as they are built, in sorted runs, and read back merged in the order a
//! map index writes them: the planet's elements weigh many times what memory holds.
//!
//! Workers hand their elements over in batches to one sink, which sorts what it holds into a run
//! every couple of gigabytes, so that the runs stay few. A run is written in chunk files, each removed
//! once read, so that merging the runs takes little more room than the tiles they become.

use std::io;
use std::path::{Path, PathBuf};

use planetrs_dataset::map_index::{Element, Grid, Placement};

/// Elements set down on disk, waiting to be merged.
pub(super) struct Spill {
    directory: PathBuf,
}

impl Spill {
    /// A spill writing its runs into `directory`, positions kept on `grid`, a run every `run_bytes`.
    pub(super) fn new(directory: &Path, _grid: Grid, _run_bytes: usize) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    /// Sets down an element at its placement.
    pub(super) fn add(&self, _placement: &Placement, _element: &Element) -> io::Result<()> {
        Ok(())
    }

    /// Every element set down, in the order of their placements, each with its key, removing the
    /// runs as they are read.
    pub(super) fn merged(self) -> io::Result<impl Iterator<Item = io::Result<(u64, Element)>>> {
        let _ = self.directory;
        Ok(std::iter::empty())
    }
}

#[cfg(test)]
mod tests {
    use planetrs_dataset::map_index::{Line, Position};

    use super::*;

    fn grid() -> Grid {
        Grid {
            granularity: 100,
            latitude_offset: 0,
            longitude_offset: 0,
        }
    }

    fn at(latitude: i64, longitude: i64) -> Position {
        Position { latitude, longitude }
    }

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }

    /// Monaco's harbour and Abidjan's port: named and lone nodes, a quay leaving through a node the
    /// planet does not place, and a basin whose inner member lies far away.
    fn harbours() -> Vec<Element> {
        let quay = Line {
            id: 10,
            tags: tags(&[("man_made", "quay"), ("name", "Quai Antoine Ier")]),
            refs: vec![1, 2, 99],
            positions: vec![
                Some(at(43_734_700_000, 7_418_600_000)),
                Some(at(43_735_200_000, 7_421_400_000)),
                None,
            ],
        };
        let island = Line {
            id: 12,
            tags: Vec::new(),
            refs: vec![8, 9, 8],
            positions: vec![
                Some(at(43_800_000_000, 7_500_000_000)),
                Some(at(43_810_000_000, 7_510_000_000)),
                Some(at(43_800_000_000, 7_500_000_000)),
            ],
        };
        vec![
            Element::Node {
                id: 3,
                at: at(43_735_000_000, 7_420_000_000),
                tags: tags(&[("name", "Place d'Armes"), ("place", "square")]),
            },
            Element::Node {
                id: 4,
                at: at(43_736_000_000, 7_419_000_000),
                tags: Vec::new(),
            },
            Element::Way(quay.clone()),
            Element::Multipolygon {
                id: 100,
                tags: tags(&[("type", "multipolygon"), ("natural", "water")]),
                members: vec![(10, "outer".into()), (12, "inner".into())],
                ways: vec![quay, island],
            },
            Element::Way(Line {
                id: 20,
                tags: tags(&[("man_made", "quay")]),
                refs: vec![21, 22],
                positions: vec![
                    Some(at(5_269_000_000, -4_012_000_000)),
                    Some(at(5_270_000_000, -4_010_000_000)),
                ],
            }),
            Element::Node {
                id: 23,
                at: at(-5_269_000_000, -180_000_000_000),
                tags: tags(&[("name", "Île Dérivée")]),
            },
        ]
    }

    #[test]
    fn given_elements_set_down_out_of_order_across_runs_when_merged_then_they_come_back_whole_in_the_order_of_their_placements_and_the_runs_are_gone()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory =
            std::env::temp_dir().join(format!("planetrs-mapclipper-spill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let elements = harbours();
        let spill = Spill::new(&directory, grid(), 200)?;
        for element in elements.iter().rev() {
            spill.add(&element.placement()?, element)?;
        }

        let merged = spill.merged()?.collect::<io::Result<Vec<_>>>()?;

        let mut expected = elements
            .iter()
            .map(|element| element.placement().map(|placement| (placement, element.clone())))
            .collect::<Result<Vec<_>, _>>()?;
        expected.sort_by_key(|(placement, _)| (placement.key, placement.order));
        assert_eq!(
            merged,
            expected
                .into_iter()
                .map(|(placement, element)| (placement.key, element))
                .collect::<Vec<_>>()
        );
        assert_eq!(std::fs::read_dir(&directory)?.count(), 0, "the runs are gone");
        Ok(())
    }
}
