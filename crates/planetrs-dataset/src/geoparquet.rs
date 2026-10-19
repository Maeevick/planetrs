//! What makes a Parquet file GeoParquet: the `geo` key naming its geometry column and how it is
//! encoded, so that DuckDB, GDAL and QGIS read the geometries as geometries.

/// The GeoParquet version the files declare: the last stable one, whose WKB columns every reader
/// knows.
const VERSION: &str = "1.1.0";

/// The `geo` key-value pair of a file whose geometry column holds WKB of the given types, in
/// longitude and latitude on WGS84, GeoParquet's default when no CRS is named.
pub(crate) fn geo(geometry_types: &[&str]) -> (&'static str, String) {
    let declaration = serde_json::json!({
        "version": VERSION,
        "primary_column": "geometry",
        "columns": {
            "geometry": {
                "encoding": "WKB",
                "geometry_types": geometry_types,
            }
        }
    });
    ("geo", declaration.to_string())
}

/// The `geo` key-value pair of a file whose geometry column holds WKB of the given types, and whose
/// `bbox` column holds each row's bounding box, which readers prune row groups by.
pub(crate) fn geo_covered_by_bbox(geometry_types: &[&str]) -> (&'static str, String) {
    let declaration = serde_json::json!({
        "version": VERSION,
        "primary_column": "geometry",
        "columns": {
            "geometry": {
                "encoding": "WKB",
                "geometry_types": geometry_types,
                "covering": {
                    "bbox": {
                        "xmin": ["bbox", "xmin"],
                        "ymin": ["bbox", "ymin"],
                        "xmax": ["bbox", "xmax"],
                        "ymax": ["bbox", "ymax"],
                    }
                }
            }
        }
    });
    ("geo", declaration.to_string())
}
