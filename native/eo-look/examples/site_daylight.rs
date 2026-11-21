//! Solar and radiometric inputs for offline studies of the playable city.
use eo_look::{LookFrame, sun::sun_direction};

fn main() {
    println!("hour,east,up,south,key_r,key_g,key_b,sky_irradiance_up");
    for hour in [
        7.0, 8.0, 8.5, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 22.0,
    ] {
        let sun = sun_direction(hour);
        let frame = LookFrame::new(hour);
        let [r, g, b] = frame.key;
        println!(
            "{hour},{},{},{},{r},{g},{b},{}",
            sun.x, sun.y, sun.z, frame.sky_irradiance_up
        );
    }
}
