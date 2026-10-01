//! The scene as GeoJSON, which is the deliverable for anything that draws its own map.
//!
//! Coordinates stay in lon/lat and are never projected: RFC 7946 mandates WGS84, so projecting is
//! the renderer's job and never the file's. That is also what lets this file and a `geoMercator`
//! over it agree exactly rather than nearly.
//!
//! Written by hand rather than through a serialiser, because the shape is fixed, the escaping is
//! four characters, and the output has to be byte-identical between runs.

use crate::scene::{Feature, Label, Point, Scene};

/// Six decimals is eleven centimetres on the ground, against about thirteen metres per pixel in the
/// frames this is drawn at. Writing the full double would triple the file for precision nothing
/// downstream can use, and would make the output depend on the platform's float formatting.
const DECIMALS: usize = 6;

fn escaped(text: &str) -> String {
    text.chars()
        .flat_map(|character| match character {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' | '\r' | '\t' => vec![' '],
            other if (other as u32) < 0x20 => vec![' '],
            other => vec![other],
        })
        .collect()
}

fn positions(points: &[Point]) -> String {
    let mut out = String::with_capacity(points.len() * 24);
    out.push('[');
    for (index, [longitude, latitude]) in points.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{longitude:.DECIMALS$},{latitude:.DECIMALS$}]"));
    }
    out.push(']');
    out
}

fn geometry(feature: &Feature) -> String {
    if !feature.shape.closed {
        return format!(
            "{{\"type\":\"LineString\",\"coordinates\":{}}}",
            positions(&feature.shape.points)
        );
    }

    let mut rings = positions(&feature.shape.points);
    for hole in &feature.shape.holes {
        rings.push(',');
        rings.push_str(&positions(hole));
    }
    format!("{{\"type\":\"Polygon\",\"coordinates\":[{rings}]}}")
}

fn place(scene: &Scene) -> String {
    let names: Vec<String> = scene
        .place
        .names
        .iter()
        .map(|(language, name)| format!("\"{}\":\"{}\"", escaped(language), escaped(name)))
        .collect();

    format!(
        "{{\"names\":{{{}}},\"latitude\":{:.DECIMALS$},\"longitude\":{:.DECIMALS$}}}",
        names.join(","),
        scene.place.at.0,
        scene.place.at.1
    )
}

/// Every name the scene found, ranked, as a foreign member.
///
/// RFC 7946 allows these, and it is the honest place for them: a name is not a geometry, and a
/// reader drawing its own map wants to place them its own way rather than take ours.
fn named(labels: &[Label]) -> String {
    labels
        .iter()
        .map(|label| {
            format!(
                "\n{{\"text\":\"{}\",\"kind\":\"{}\",\"rank\":{},\"osm\":{},\
                 \"longitude\":{:.DECIMALS$},\"latitude\":{:.DECIMALS$}}}",
                escaped(&label.text),
                label.class.name(),
                label.rank,
                label.osm,
                label.at[0],
                label.at[1]
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub fn write(scene: &Scene) -> String {
    let frame = &scene.frame;
    let mut out = String::with_capacity(scene.features.len() * 256);

    out.push_str("{\"type\":\"FeatureCollection\",\n");
    out.push_str(&format!(
        "\"attribution\":\"{}\",\n",
        escaped(crate::credit::DATA_NOTICE)
    ));
    out.push_str(&format!(
        "\"source\":{{\"file\":\"{}\",\"replication_timestamp\":{},\"replication_sequence\":{}}},\n",
        escaped(&scene.provenance.source_name),
        scene.provenance.replication_timestamp,
        scene.provenance.replication_sequence
    ));
    out.push_str(&format!("\"place\":{},\n", place(scene)));
    out.push_str(&format!(
        "\"bbox\":[{:.DECIMALS$},{:.DECIMALS$},{:.DECIMALS$},{:.DECIMALS$}],\n",
        frame.west, frame.south, frame.east, frame.north
    ));
    out.push_str(&format!("\"labels\":[{}],\n", named(&scene.labels)));
    out.push_str("\"features\":[\n");

    for (index, feature) in scene.features.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        let name = match &feature.name {
            Some(name) => format!(",\"name\":\"{}\"", escaped(name)),
            None => String::new(),
        };
        out.push_str(&format!(
            "{{\"type\":\"Feature\",\"properties\":{{\"category\":\"{}\",\"osm\":{}{}}},\"geometry\":{}}}",
            feature.category.name(),
            feature.id,
            name,
            geometry(feature)
        ));
    }

    out.push_str("\n]}\n");
    out
}
