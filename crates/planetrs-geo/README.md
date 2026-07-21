# planetrs-geo

Bounding boxes in kilometres, and the integer windows a PBF block is tested against.

Grimble, archivist to the [planetrs](https://github.com/maeevick/planetrs) goblins, keeps this crate
for the same reason he keeps a ruler: the planet is filed in degrees, and a degree of longitude is
not the length it looks.

Ground size is asked for in kilometres and never in degrees. A degree of longitude is
`1/cos(latitude)` shorter than a degree of latitude, so the same box in raw degrees is 1.62 times too
wide at Rotterdam and 2.88 times too wide at Tromsø. An `Area` keeps two boxes apart, the frame that
is drawn and the larger one that is collected, because a way crossing the edge of a map needs its
vertices just outside it.

```rust
use planetrs_geo::Area;

let area = Area::around(43.7375, 7.4210, 3.0, 2.0, 0.25)?;
let frame = area.frame();
println!("{:.2} km by {:.2} km", frame.width_km(), frame.height_km());
# Ok::<(), planetrs_geo::GeoError>(())
```

## Licence

MIT ([LICENSE](https://github.com/maeevick/planetrs/blob/main/LICENSE)).
