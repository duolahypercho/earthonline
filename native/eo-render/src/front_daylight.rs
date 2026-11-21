//! Lambertian light returned by the street and sky at six measured garden positions.
use anyhow::{Result, ensure};

pub type Field = [[[f32; 4]; 9]; 6];

#[derive(Clone)]
pub struct FrontDaylight {
    skies: Box<[Field; 7]>,
}

impl FrontDaylight {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == 7 * 6 * 9 * 3 * 4,
            "invalid front daylight field size"
        );
        let mut values = bytes.chunks_exact(4);
        let mut skies = Box::new([[[[0.; 4]; 9]; 6]; 7]);
        for field in skies.iter_mut() {
            for terms in field {
                for rgb in terms {
                    for channel in &mut rgb[..3] {
                        *channel = f32::from_le_bytes(values.next().unwrap().try_into().unwrap());
                        ensure!(channel.is_finite(), "non-finite front daylight coefficient");
                    }
                }
            }
        }
        Ok(Self { skies })
    }

    /// The six clear surveys are two hours apart. The last survey measures
    /// transfer from a unit white sky. The live overcast sky supplies its colour.
    pub fn at(&self, frame: &eo_look::LookFrame, overcast: f32) -> Field {
        let index = ((frame.hour as f32 - 8.) * 0.5).clamp(0., 5.);
        let left = (index as usize).min(4);
        let t = index - left as f32;
        let cloud = overcast.clamp(0., 1.);
        let sky = eo_look::sh_eval(&frame.env_sh, [0., 1., 0.]);
        let luma = 0.2126 * sky[0] + 0.7152 * sky[1] + 0.0722 * sky[2];
        let radiance = sky.map(|c| {
            (c.max(0.) * frame.sky_irradiance_up / (std::f64::consts::PI * luma.max(1e-7))) as f32
        });
        let mut result = [[[0.; 4]; 9]; 6];
        for (probe, terms) in result.iter_mut().enumerate() {
            for (term, rgb) in terms.iter_mut().enumerate() {
                for c in 0..3 {
                    let a = self.skies[left][probe][term][c];
                    let b = self.skies[left + 1][probe][term][c];
                    let diffuse = self.skies[6][probe][term][c] * radiance[c];
                    rgb[c] = (a + (b - a) * t) * (1. - cloud) + diffuse * cloud;
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blends_clear_hours_and_transfers_the_live_overcast_sky() {
        let mut bytes = Vec::new();
        for row in 0..7 {
            for _ in 0..6 {
                for term in 0..9 {
                    let level = if term == 0 { (row + 1) as f32 } else { 0. };
                    for _ in 0..3 {
                        bytes.extend_from_slice(&level.to_le_bytes());
                    }
                }
            }
        }
        let light = FrontDaylight::from_bytes(&bytes).unwrap();
        let frame = eo_look::LookFrame::new(11.);
        assert_eq!(light.at(&frame, 0.)[0][0], [2.5, 2.5, 2.5, 0.]);
        let cloud = light.at(&frame, 1.)[0][0];
        let luminance = 0.2126 * cloud[0] + 0.7152 * cloud[1] + 0.0722 * cloud[2];
        assert!(
            (luminance - 7. * frame.sky_irradiance_up as f32 / std::f32::consts::PI).abs()
                < 1.0e-5
        );
        assert_eq!(light.at(&frame, 1.)[5][1], [0.; 4]);
        assert!(FrontDaylight::from_bytes(&bytes[..bytes.len() - 4]).is_err());
        bytes[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(FrontDaylight::from_bytes(&bytes).is_err());
    }
}
