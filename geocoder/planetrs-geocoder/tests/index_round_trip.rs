use planetrs_geocoder::index::Commune;
use planetrs_geocoder::index::Country;
use planetrs_geocoder::index::{read_index, write_index};

fn scratch(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("planetrs-index-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn sens() -> Commune {
    Commune {
        relation: 72_806,
        level: 8,
        names: vec!["SENS".into(), "SENS YONNE".into()],
        postcodes: vec!["89100".into()],
        centre: Some([3.2834567, 48.1972345]),
        country: Some("FR".into()),
    }
}

#[test]
fn given_a_written_index_when_read_back_then_every_commune_is_unchanged() {
    let directory = scratch("communes");
    let countries = vec![Country {
        code: "FR".into(),
        names: vec!["FRANCE".into()],
    }];

    write_index(&directory, &[sens()], &countries, "test provenance").unwrap();
    let (communes, _) = read_index(&directory).unwrap();

    assert_eq!(communes, vec![sens()]);
}

#[test]
fn given_a_commune_with_no_centre_nor_country_when_round_tripped_then_both_stay_absent() {
    let directory = scratch("absent");
    let mut lonely = sens();
    lonely.centre = None;
    lonely.country = None;
    lonely.postcodes = vec![];

    write_index(&directory, &[lonely.clone()], &[], "test provenance").unwrap();
    let (communes, _) = read_index(&directory).unwrap();

    assert_eq!(communes, vec![lonely]);
}

#[test]
fn given_a_written_index_when_read_back_then_every_country_is_unchanged() {
    let directory = scratch("countries");
    let countries = vec![Country {
        code: "FR".into(),
        names: vec!["FRANCE".into(), "REPUBLIQUE FRANCAISE".into()],
    }];

    write_index(&directory, &[], &countries, "test provenance").unwrap();
    let (_, read) = read_index(&directory).unwrap();

    assert_eq!(read, countries);
}

#[test]
fn given_a_directory_with_no_index_when_read_then_it_fails_rather_than_returning_nothing() {
    let directory = scratch("missing");

    assert!(read_index(&directory).is_err());
}

use planetrs_geocoder::index::HouseNumber;
use planetrs_geocoder::index::{AddressPoint, Street, StreetIndex};
use planetrs_geocoder::index::{read_streets, write_streets};

#[test]
fn given_written_streets_when_read_back_then_doors_and_the_road_middle_are_unchanged() {
    let directory = scratch("streets");
    let filed = vec![
        (
            1,
            Street {
                name: "ROUTE TILLEULS".into(),
                addresses: vec![AddressPoint {
                    number: HouseNumber {
                        number: 8,
                        suffix: "BIS".into(),
                    },
                    position: [3.2652, 48.2000],
                }],
                path: vec![[3.26, 48.19], [3.28, 48.21]],
            },
        ),
        (
            2,
            Street {
                name: "RUE PARON".into(),
                addresses: vec![],
                path: vec![[3.25, 48.17]],
            },
        ),
    ];

    write_streets(&directory, &StreetIndex::of(filed), "test provenance").unwrap();
    let index = read_streets(&directory).unwrap();

    let tilleuls = &index.streets_of(1)[0];
    assert_eq!(
        tilleuls.addresses[0].number,
        HouseNumber {
            number: 8,
            suffix: "BIS".into()
        }
    );
    assert_eq!(tilleuls.path, vec![[3.27, 48.2]]);
    assert_eq!(index.streets_of(2)[0].name, "RUE PARON");
}

#[test]
fn given_a_directory_with_no_street_file_when_read_then_there_are_simply_no_streets() {
    let directory = scratch("no-streets");
    std::fs::create_dir_all(&directory).unwrap();

    assert_eq!(read_streets(&directory).unwrap().communes().count(), 0);
}

use planetrs_geocoder::index::{read_parents, write_parents};

#[test]
fn given_written_parents_when_read_back_then_each_former_town_keeps_its_commune() {
    let directory = scratch("parents");
    let parents = std::collections::HashMap::from([(9070, 5812062), (9122, 5812062)]);

    write_parents(&directory, &parents, "test provenance").unwrap();

    assert_eq!(read_parents(&directory).unwrap(), parents);
}

#[test]
fn given_a_directory_with_no_parents_file_when_read_then_no_former_town_has_a_parent() {
    let directory = scratch("no-parents");
    std::fs::create_dir_all(&directory).unwrap();

    assert!(read_parents(&directory).unwrap().is_empty());
}

use planetrs_geocoder::index::{Shop, ShopIndex};
use planetrs_geocoder::index::{read_shops, write_shops};

#[test]
fn given_written_shops_when_read_back_then_names_street_and_position_are_unchanged() {
    let directory = scratch("shops");
    let shop = Shop {
        names: vec!["ACME CITY".into(), "ACME CITY GAMBETTA".into()],
        street: "RUE GAMBETTA".into(),
        position: [-1.6245, 49.6381],
    };

    write_shops(
        &directory,
        &ShopIndex::of(vec![(9070, shop.clone())]),
        "test provenance",
    )
    .unwrap();

    assert_eq!(read_shops(&directory).unwrap().shops_of(9070), &[shop]);
}

#[test]
fn given_a_directory_with_no_shops_file_when_read_then_there_are_no_shops() {
    let directory = scratch("no-shops");
    std::fs::create_dir_all(&directory).unwrap();

    assert_eq!(read_shops(&directory).unwrap().communes().count(), 0);
}

use planetrs_geocoder::index::{read_areas, write_areas};

#[test]
fn given_written_areas_when_read_back_then_each_commune_keeps_its_area_to_the_hectare() {
    let directory = scratch("areas");
    let areas = std::collections::HashMap::from([(72_806, 44.91234), (9_070, 2.5)]);

    write_areas(&directory, &areas, "test provenance").unwrap();
    let read = read_areas(&directory).unwrap();

    assert!((read[&72_806] - 44.91).abs() < 0.01);
    assert!((read[&9_070] - 2.5).abs() < 0.01);
}

#[test]
fn given_a_directory_with_no_areas_file_when_read_then_no_commune_has_an_area() {
    let directory = scratch("no-areas");
    std::fs::create_dir_all(&directory).unwrap();

    assert!(read_areas(&directory).unwrap().is_empty());
}

#[test]
fn given_any_index_file_when_written_then_it_carries_the_openstreetmap_attribution_and_reads_the_same() {
    let directory = scratch("attribution");
    write_index(&directory, &[sens()], &[], "test provenance").unwrap();
    write_streets(&directory, &StreetIndex::default(), "test provenance").unwrap();
    write_parents(
        &directory,
        &std::collections::HashMap::from([(9, 8)]),
        "test provenance",
    )
    .unwrap();
    write_areas(
        &directory,
        &std::collections::HashMap::from([(8, 2.5)]),
        "test provenance",
    )
    .unwrap();
    write_shops(&directory, &ShopIndex::default(), "test provenance").unwrap();

    for file in [
        "communes.tsv",
        "countries.tsv",
        "streets.tsv",
        "parents.tsv",
        "areas.tsv",
        "shops.tsv",
    ] {
        let written = std::fs::read_to_string(directory.join(file)).unwrap();
        assert_eq!(
            written.lines().nth(1),
            Some(
                "# Data © OpenStreetMap contributors, under the Open Database License (ODbL) 1.0: \
                 https://opendatacommons.org/licenses/odbl/1-0/ (https://www.openstreetmap.org/copyright)"
            ),
            "{file}"
        );
    }
    assert_eq!(read_index(&directory).unwrap().0, vec![sens()]);
    assert_eq!(read_parents(&directory).unwrap().get(&9), Some(&8));
    assert_eq!(read_areas(&directory).unwrap().get(&8), Some(&2.5));
}
