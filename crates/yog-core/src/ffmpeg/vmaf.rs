use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VmafLog {
    pub frames: Vec<Frame>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Frame {
    #[serde(rename = "frameNum")]
    pub frame_num: u64,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Metrics {
    #[serde(default, deserialize_with = "metric")]
    pub vmaf: Option<f64>,
    #[serde(default, deserialize_with = "metric")]
    pub float_ssim: Option<f64>,
    #[serde(default, deserialize_with = "metric")]
    pub psnr_y: Option<f64>,
}

fn metric<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Option<f64>, D::Error> {
    Ok(serde_json::Value::deserialize(decoder)?.as_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_missing_and_non_numeric_metrics_remain_distinct() {
        let log: VmafLog = serde_json::from_str(
            r#"{"frames":[
            {"frameNum":0,"metrics":{"vmaf":98.5,"psnr_y":"inf"}},
            {"frameNum":1,"metrics":{"vmaf":0,"float_ssim":null}}
        ]}"#,
        )
        .unwrap();
        assert_eq!(log.frames[0].metrics.vmaf, Some(98.5));
        assert_eq!(log.frames[0].metrics.psnr_y, None);
        assert_eq!(log.frames[0].metrics.float_ssim, None);
        assert_eq!(log.frames[1].metrics.vmaf, Some(0.0));
        assert_eq!(log.frames[1].metrics.float_ssim, None);
        assert!(serde_json::from_str::<VmafLog>("{}").is_err());
    }
}
