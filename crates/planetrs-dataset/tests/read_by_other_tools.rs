//! The files a dataset writes, read by the tools the public will read them with.
//!
//! These run on request, with `make test-system`, because they need DuckDB and GDAL installed. They
//! are the check that a file written here is GeoParquet to everyone else, not only to this crate.

use std::path::PathBuf;
use std::process::Command;

use planetrs_dataset::Provenance;
use planetrs_dataset::gazetteer::{self, Area};

type Outcome = Result<(), Box<dyn std::error::Error>>;

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

fn sens_and_monaco() -> Vec<Area> {
    vec![
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
            relation: 1_124_039,
            level: 2,
            country: Some("MC".into()),
            names: vec!["MONACO".into()],
            postcodes: vec![],
            square_kilometres: None,
            absorbed_by: None,
            centre: None,
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
    ]
}

fn written_gazetteer(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!("planetrs-dataset-{name}-{}.parquet", std::process::id()));
    std::fs::write(&path, gazetteer::to_parquet(&sens_and_monaco(), &planet())?)?;
    Ok(path)
}

fn output_of(program: &str, arguments: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("{program} could not be run, is it installed? {error}"))?;
    if !output.status.success() {
        return Err(format!("{program} failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
#[ignore = "needs DuckDB on the PATH"]
fn given_a_gazetteer_file_when_duckdb_reads_it_then_every_area_comes_back_with_its_lists_and_its_centre_as_a_geometry()
-> Outcome {
    let path = written_gazetteer("duckdb")?;
    let file = path.display();

    let rows = output_of(
        "duckdb",
        &[
            "-csv",
            "-c",
            &format!(
                "SELECT relation, level, country, names, postcodes, square_kilometres, absorbed_by, \
                 typeof(geometry), ST_AsText(geometry) FROM read_parquet('{file}') ORDER BY relation"
            ),
        ],
    )?;

    assert_eq!(
        rows,
        "relation,level,country,names,postcodes,square_kilometres,absorbed_by,typeof(geometry),st_astext(geometry)\n\
         946433,8,FR,\"[SENS, SENS YONNE]\",[89100],105.08,NULL,\"GEOMETRY('OGC:CRS84')\",POINT (3.282606 48.1978559)\n\
         1124039,2,MC,[MONACO],[],NULL,NULL,\"GEOMETRY('OGC:CRS84')\",NULL\n\
         1402847,9,FR,[SAINT CLEMENT],\"[89100, 89101]\",12.5,946433,\"GEOMETRY('OGC:CRS84')\",POINT (3.29 48.21)\n"
    );
    Ok(())
}

#[test]
#[ignore = "needs DuckDB on the PATH"]
fn given_a_gazetteer_file_when_duckdb_reads_its_footer_then_it_names_planetrs_as_its_writer() -> Outcome {
    let path = written_gazetteer("duckdb-footer")?;

    let writer = output_of(
        "duckdb",
        &[
            "-csv",
            "-noheader",
            "-c",
            &format!(
                "SELECT created_by FROM parquet_file_metadata('{}')",
                path.display()
            ),
        ],
    )?;

    assert!(writer.starts_with("planetrs-dataset version "), "{writer}");
    Ok(())
}

#[test]
#[ignore = "needs GDAL's ogrinfo on the PATH"]
fn given_a_gazetteer_file_when_gdal_reads_it_then_it_sees_one_point_layer_holding_every_area() -> Outcome {
    let path = written_gazetteer("gdal")?;

    let summary = output_of("ogrinfo", &["-so", "-al", &path.display().to_string()])?;

    assert!(summary.contains("Geometry: Point"), "{summary}");
    assert!(summary.contains("Feature Count: 3"), "{summary}");
    assert!(summary.contains("absorbed_by: Integer64"), "{summary}");
    Ok(())
}
