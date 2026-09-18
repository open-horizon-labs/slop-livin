use crate::entities::{Residual, VolumeTruth};
pub fn truth(attributed_bytes: u64, reported_bytes: u64, residuals: Vec<Residual>) -> VolumeTruth {
    VolumeTruth {
        attributed_bytes,
        reported_bytes,
        residuals,
    }
}
