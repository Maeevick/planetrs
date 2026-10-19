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
