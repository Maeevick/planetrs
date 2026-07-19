//! Turning "twelve kilometres across, centred here" into a box in degrees, and back into the
//! integer units a PBF block stores coordinates in.
//!
//! Ground size is asked for in kilometres and never in degrees. A degree of longitude is
//! `1/cos(latitude)` shorter than a degree of latitude, so the same box in raw degrees is 1.62
//! times too wide at Rotterdam and 2.88 times too wide at Tromsø.
