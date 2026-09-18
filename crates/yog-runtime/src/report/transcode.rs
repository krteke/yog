use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TranscodeResult {
    pub bytes: Option<u64>,
    pub duration_seconds: Option<f64>,
    pub size_percent: Option<f64>,
    pub elapsed_seconds: f64,
    pub speed: Option<f64>,
    pub frames: Option<u64>,
    pub fps: Option<f64>,
}

impl TranscodeResult {
    pub fn new(
        bytes: Option<u64>,
        duration_seconds: Option<f64>,
        elapsed_seconds: f64,
        speed: Option<f64>,
        frames: Option<u64>,
        fps: Option<f64>,
    ) -> Self {
        Self {
            bytes,
            duration_seconds,
            size_percent: None,
            elapsed_seconds,
            speed,
            frames,
            fps,
        }
    }
}
